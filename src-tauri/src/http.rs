//! The HTTP core.
//!
//! Every request the app sends goes through here rather than through the
//! webview's `fetch()`. The webview can't set `Origin`/`Cookie`/`User-Agent`,
//! is bound by CORS, and can't relax TLS verification — all of which an API
//! client needs. This module is deliberately free of any Tauri types so the
//! MCP server can call it headlessly.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// Responses larger than this are truncated instead of being pulled into
/// memory. The full byte count is still reported as `size_bytes`.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    /// Client-generated; also the handle used to cancel the request.
    pub id: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default = "yes")]
    pub follow_redirects: bool,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    /// Time to response headers.
    pub ttfb_ms: u64,
    /// Time to the last byte of the body.
    pub total_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    /// Differs from the request URL when redirects were followed.
    pub final_url: String,
    pub headers: Vec<Header>,
    /// UTF-8 text, or standard base64 when `is_binary` is set.
    pub body: String,
    pub is_binary: bool,
    pub truncated: bool,
    pub size_bytes: u64,
    pub timing: Timing,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("invalid method `{0}`")]
    Method(String),
    #[error("invalid URL: {0}")]
    Url(String),
    #[error("invalid header name `{0}`")]
    HeaderName(String),
    #[error("invalid value for header `{0}`")]
    HeaderValue(String),
    #[error("could not build HTTP client: {0}")]
    Client(String),
    #[error("request timed out")]
    Timeout,
    #[error("request cancelled")]
    Cancelled,
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("{0}")]
    Transport(String),
}

// Tauri sends command errors to the frontend as JSON; a flat string is all the
// UI needs.
impl Serialize for HttpError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

fn classify(err: reqwest::Error) -> HttpError {
    if err.is_timeout() {
        HttpError::Timeout
    } else if err.is_connect() {
        HttpError::Connect(root_cause(&err))
    } else {
        HttpError::Transport(root_cause(&err))
    }
}

/// reqwest's `Display` is often just "error sending request"; the useful detail
/// is further down the source chain.
fn root_cause(err: &dyn std::error::Error) -> String {
    let mut cause: &dyn std::error::Error = err;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}

/// Clients are cached because building one is expensive (TLS setup, connection
/// pool) and these two options are the only things that change its shape today.
/// Sections will widen this key when they bring their own proxy and cookie jar.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
struct ClientKey {
    follow_redirects: bool,
    accept_invalid_certs: bool,
}

#[derive(Default)]
pub struct HttpState {
    clients: Mutex<HashMap<ClientKey, reqwest::Client>>,
    inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl HttpState {
    fn client(&self, key: ClientKey) -> Result<reqwest::Client, HttpError> {
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }

        let redirect = if key.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        };

        let client = reqwest::Client::builder()
            .user_agent(concat!("fetch/", env!("CARGO_PKG_VERSION")))
            // One jar per client for now. Per-section jars arrive with sections.
            .cookie_store(true)
            .redirect(redirect)
            .danger_accept_invalid_certs(key.accept_invalid_certs)
            .build()
            .map_err(|e| HttpError::Client(e.to_string()))?;

        clients.insert(key, client.clone());
        Ok(client)
    }

    /// Drops the cancel handle for `id`, if any, which resolves the receiver
    /// held by the in-flight request.
    pub fn cancel(&self, id: &str) -> bool {
        self.inflight.lock().unwrap().remove(id).is_some()
    }
}

pub async fn send(state: &HttpState, spec: RequestSpec) -> Result<ResponseData, HttpError> {
    let method = reqwest::Method::from_bytes(spec.method.trim().as_bytes())
        .map_err(|_| HttpError::Method(spec.method.clone()))?;
    let url = reqwest::Url::parse(spec.url.trim()).map_err(|e| HttpError::Url(e.to_string()))?;

    let client = state.client(ClientKey {
        follow_redirects: spec.follow_redirects,
        accept_invalid_certs: spec.accept_invalid_certs,
    })?;

    let mut request = client.request(method, url);
    for header in &spec.headers {
        // The UI's header table keeps blank rows around for editing.
        let name = header.name.trim();
        if name.is_empty() {
            continue;
        }
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| HttpError::HeaderName(header.name.clone()))?;
        let value = HeaderValue::from_str(&header.value)
            .map_err(|_| HttpError::HeaderValue(header.name.clone()))?;
        request = request.header(name, value);
    }
    if let Some(body) = spec.body.filter(|b| !b.is_empty()) {
        request = request.body(body);
    }
    if let Some(ms) = spec.timeout_ms {
        request = request.timeout(Duration::from_millis(ms));
    }

    let (cancel_tx, cancel_rx) = oneshot::channel();
    state
        .inflight
        .lock()
        .unwrap()
        .insert(spec.id.clone(), cancel_tx);

    let outcome = run(request, cancel_rx).await;
    state.inflight.lock().unwrap().remove(&spec.id);
    outcome
}

async fn run(
    request: reqwest::RequestBuilder,
    mut cancel: oneshot::Receiver<()>,
) -> Result<ResponseData, HttpError> {
    let started = Instant::now();

    let response = tokio::select! {
        biased;
        _ = &mut cancel => return Err(HttpError::Cancelled),
        result = request.send() => result.map_err(classify)?,
    };
    let ttfb_ms = started.elapsed().as_millis() as u64;

    let status = response.status();
    let final_url = response.url().to_string();
    let headers: Vec<Header> = response
        .headers()
        .iter()
        .map(|(name, value)| Header {
            name: name.to_string(),
            value: value.to_str().unwrap_or("<non-utf8 value>").to_string(),
        })
        .collect();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // Read the body as a stream so cancellation stays responsive on slow or
    // very large responses. Step 5 pushes these chunks to the UI over an
    // `ipc::Channel`; the shape here is already right for that.
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut size_bytes: u64 = 0;
    let mut truncated = false;

    loop {
        let chunk = tokio::select! {
            biased;
            _ = &mut cancel => return Err(HttpError::Cancelled),
            chunk = stream.next() => chunk,
        };

        match chunk {
            Some(Ok(bytes)) => {
                size_bytes += bytes.len() as u64;
                let room = MAX_BODY_BYTES.saturating_sub(buffer.len());
                if room == 0 {
                    truncated = true;
                } else {
                    let take = room.min(bytes.len());
                    buffer.extend_from_slice(&bytes[..take]);
                    truncated |= take < bytes.len();
                }
            }
            Some(Err(err)) => return Err(classify(err)),
            None => break,
        }
    }

    let total_ms = started.elapsed().as_millis() as u64;

    // Trust the declared content type, but fall back to a UTF-8 check so an
    // unlabelled JSON response still renders as text.
    let (body, is_binary) = match String::from_utf8(buffer) {
        Ok(text) if looks_textual(content_type.as_deref()) => (text, false),
        Ok(text) if content_type.is_none() => (text, false),
        Ok(text) => (
            base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
            true,
        ),
        Err(err) => (
            base64::engine::general_purpose::STANDARD.encode(err.as_bytes()),
            true,
        ),
    };

    Ok(ResponseData {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or_default().to_string(),
        final_url,
        headers,
        body,
        is_binary,
        truncated,
        size_bytes,
        timing: Timing { ttfb_ms, total_ms },
    })
}

fn looks_textual(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    essence.starts_with("text/")
        || essence.ends_with("+json")
        || essence.ends_with("+xml")
        || matches!(
            essence.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/ecmascript"
                | "application/x-www-form-urlencoded"
                | "application/graphql"
                | "application/x-ndjson"
                | "application/problem+json"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one connection with a canned response and hands back whatever the
    /// client sent, so tests can assert on both directions.
    async fn one_shot_server(
        response: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buf = [0u8; 1024];

            // Read until the body is in — good enough for the small fixtures here.
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                received.extend_from_slice(&buf[..n]);
                if n == 0 || received.windows(4).any(|w| w == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&received).to_string();
                    let has_body = text
                        .to_lowercase()
                        .contains("content-length: ")
                        && text.split("\r\n\r\n").nth(1).is_some_and(|b| !b.is_empty());
                    if has_body || !text.to_lowercase().contains("content-length: ") {
                        break;
                    }
                }
            }

            socket.write_all(response).await.unwrap();
            socket.flush().await.unwrap();
            drop(socket);
            String::from_utf8_lossy(&received).to_string()
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn round_trips_a_json_post() {
        let (url, server) = one_shot_server(
            b"HTTP/1.1 201 Created\r\n\
              Content-Type: application/json\r\n\
              X-Trace: abc123\r\n\
              Content-Length: 17\r\n\
              \r\n\
              {\"created\":true}\n",
        )
        .await;

        let state = HttpState::default();
        let response = send(
            &state,
            RequestSpec {
                id: "test-1".into(),
                method: "POST".into(),
                url: format!("{url}/user/create"),
                headers: vec![Header {
                    name: "Content-Type".into(),
                    value: "application/json".into(),
                }],
                body: Some("{\"name\":\"ada\"}".into()),
                timeout_ms: Some(5_000),
                follow_redirects: true,
                accept_invalid_certs: false,
            },
        )
        .await
        .expect("request should succeed");

        assert_eq!(response.status, 201);
        assert_eq!(response.status_text, "Created");
        assert_eq!(response.body, "{\"created\":true}\n");
        assert!(!response.is_binary);
        assert!(!response.truncated);
        assert_eq!(response.size_bytes, 17);
        assert!(response
            .headers
            .iter()
            .any(|h| h.name == "x-trace" && h.value == "abc123"));

        // The request actually went out with the method, path and body we asked for.
        let sent = server.await.unwrap();
        assert!(sent.starts_with("POST /user/create HTTP/1.1"), "{sent}");
        assert!(sent.contains("{\"name\":\"ada\"}"), "{sent}");
    }

    #[tokio::test]
    async fn reports_binary_responses_as_base64() {
        // A PNG magic number — not valid UTF-8, and not a textual content type.
        let (url, _server) = one_shot_server(
            b"HTTP/1.1 200 OK\r\n\
              Content-Type: image/png\r\n\
              Content-Length: 4\r\n\
              \r\n\
              \x89PNG",
        )
        .await;

        let state = HttpState::default();
        let response = send(
            &state,
            RequestSpec {
                id: "test-2".into(),
                method: "GET".into(),
                url,
                headers: vec![],
                body: None,
                timeout_ms: Some(5_000),
                follow_redirects: true,
                accept_invalid_certs: false,
            },
        )
        .await
        .expect("request should succeed");

        assert!(response.is_binary);
        assert_eq!(response.body, "iVBORw==");
    }

    #[tokio::test]
    async fn cancels_an_in_flight_request() {
        // Accepts the connection but never answers, so the request is still
        // waiting on headers when we cancel it.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(socket);
        });

        let state = std::sync::Arc::new(HttpState::default());
        let spec = RequestSpec {
            id: "test-3".into(),
            method: "GET".into(),
            url: format!("http://{addr}"),
            headers: vec![],
            body: None,
            timeout_ms: Some(30_000),
            follow_redirects: true,
            accept_invalid_certs: false,
        };

        let sending = tokio::spawn({
            let state = state.clone();
            async move { send(&state, spec).await }
        });

        // Let it get as far as waiting on the server before pulling the plug.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(state.cancel("test-3"), "request should have been in flight");

        let outcome = sending.await.unwrap();
        assert!(matches!(outcome, Err(HttpError::Cancelled)), "{outcome:?}");
        assert!(!state.cancel("test-3"), "handle should be cleaned up");
    }

    #[test]
    fn recognises_textual_content_types() {
        assert!(looks_textual(Some("application/json")));
        assert!(looks_textual(Some("application/json; charset=utf-8")));
        assert!(looks_textual(Some("TEXT/HTML")));
        assert!(looks_textual(Some("application/vnd.api+json")));
        assert!(!looks_textual(Some("image/png")));
        assert!(!looks_textual(Some("application/octet-stream")));
        assert!(!looks_textual(None));
    }
}
