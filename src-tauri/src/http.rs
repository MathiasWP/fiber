//! The HTTP core.
//!
//! Every request the app sends goes through here rather than through the
//! webview's `fetch()`. The webview can't set `Origin`/`Cookie`/`User-Agent`,
//! is bound by CORS, and can't relax TLS verification — all of which an API
//! client needs. This module is deliberately free of any Tauri types so the
//! MCP server can call it headlessly.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// Responses larger than this are truncated instead of being pulled into
/// memory. The full byte count is still reported as `size_bytes`.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Applied when a spec names no timeout of its own. An API client that can
/// hang forever because a server stopped answering mid-body helps nobody, and
/// a minute is longer than any response worth waiting for.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Ceiling on manually-followed redirects, matching the client-level policy.
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// How the request body is built. JSON remains the default so existing
/// collections keep the editor they already have.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, Default, PartialEq, Eq,
)]
#[serde(rename_all = "camelCase")]
pub enum BodyKind {
    #[default]
    Json,
    Text,
    /// `application/x-www-form-urlencoded` from `form`.
    Form,
    /// `multipart/form-data` from `form`, including file fields.
    Multipart,
    /// Raw bytes of the file at `file`.
    File,
}

/// One field of a form or multipart body.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub name: String,
    #[serde(default)]
    pub value: String,
    /// Absolute path of a file to attach. Empty means this is a text field,
    /// unless `is_file` is set and the user has not picked one yet.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_file: bool,
}

/// Headers that carry a credential. They must never travel back to a model
/// (see `mcp`) and never land in the history database — a token in a response
/// header outlives its request the moment either happens.
pub fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization" | "x-api-key"
    )
}

fn is_sensitive(name: &str, additional: Option<&str>) -> bool {
    is_credential(name)
        || additional.is_some_and(|sensitive| name.eq_ignore_ascii_case(sensitive.trim()))
}

/// Redacts the standard credential headers plus a section's configured auth
/// header, which can be any valid HTTP header name.
pub fn redact_with(headers: &[Header], additional: Option<&str>) -> Vec<Header> {
    headers
        .iter()
        .map(|header| Header {
            name: header.name.clone(),
            value: if is_sensitive(&header.name, additional) {
                "<redacted>".into()
            } else {
                header.value.clone()
            },
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    /// Client-generated; also the handle used to cancel the request, and the
    /// primary key of its history entry.
    pub id: String,
    /// The saved request this belongs to, so history can bucket it per request.
    #[serde(default)]
    pub request_id: String,
    /// The section this came from, if any — its auth config is applied on send.
    #[serde(default)]
    pub section_id: Option<String>,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_kind: BodyKind,
    #[serde(default)]
    pub form: Vec<FormField>,
    /// Absolute path when `body_kind` is `file`.
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub path_params: Vec<Header>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default = "yes")]
    pub follow_redirects: bool,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub proxy: String,
    /// The name of the header `apply_auth` injected the section's credential
    /// into, if it did. Never set by the frontend — `skip` keeps it off the
    /// bridge — because it exists for one reason: reqwest strips
    /// `Authorization` and `Cookie` when a redirect changes host, but knows
    /// nothing about a custom credential header like `X-Api-Key`. Naming the
    /// header here is what lets the redirect handling below shed it too.
    #[serde(skip)]
    pub sensitive_header: Option<String>,
}

impl Default for RequestSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            request_id: String::new(),
            section_id: None,
            method: "GET".into(),
            url: String::new(),
            headers: Vec::new(),
            body: None,
            body_kind: BodyKind::Json,
            form: Vec::new(),
            file: String::new(),
            path_params: Vec::new(),
            timeout_ms: None,
            follow_redirects: true,
            accept_invalid_certs: false,
            proxy: String::new(),
            sensitive_header: None,
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Chunks were pushed to the streaming sink. The window already has the
    /// text; the command result may leave `body` empty for large replies so
    /// the same bytes do not cross the IPC bridge twice.
    #[serde(default)]
    pub body_streamed: bool,
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
    #[error("authentication failed: {0}")]
    Auth(String),
    // The section file could not be read at send time. Its own message names
    // the file; wrapping it in more words would only bury that.
    #[error("{0}")]
    Section(String),
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
/// pool). The key is the section plus the options that actually change a
/// client's shape: cookie jars are per-section so one API's session cookie
/// cannot leak onto another, and proxy / TLS / redirect policy belong to the
/// collection rather than the process.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct ClientKey {
    section_id: String,
    follow_redirects: bool,
    accept_invalid_certs: bool,
    proxy: String,
}

#[derive(Default)]
pub struct HttpState {
    clients: Mutex<HashMap<ClientKey, reqwest::Client>>,
    inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl HttpState {
    fn client(&self, key: ClientKey) -> Result<reqwest::Client, HttpError> {
        // A poisoned lock means some other request's thread panicked while
        // holding it. The data here (a client cache, a cancel map) is still
        // coherent; refusing every request from then on would turn one panic
        // into a dead app. Same reasoning at every `into_inner` below.
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }

        let redirect = if key.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        };

        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("fiber/", env!("CARGO_PKG_VERSION")))
            // One jar per client, and one client per section — so a session
            // cookie from collection A never rides along with collection B.
            .cookie_store(true)
            .redirect(redirect)
            .danger_accept_invalid_certs(key.accept_invalid_certs);

        if !key.proxy.trim().is_empty() {
            let proxy = reqwest::Proxy::all(key.proxy.trim())
                .map_err(|e| HttpError::Client(format!("invalid proxy: {e}")))?;
            builder = builder.proxy(proxy);
        }

        let client = builder
            .build()
            .map_err(|e| HttpError::Client(e.to_string()))?;

        clients.insert(key, client.clone());
        Ok(client)
    }

    /// Drops the cancel handle for `id`, if any, which resolves the receiver
    /// held by the in-flight request.
    pub fn cancel(&self, id: &str) -> bool {
        self.inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
            .is_some()
    }
}

/// A body arriving in pieces, for the pane that shows it.
///
/// `Start` is what clears that pane. A 401 is retried, and the second attempt's
/// body replaces the first rather than continuing it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum BodyEvent {
    Start,
    Chunk { text: String },
}

/// Where body chunks go as they arrive. The window forwards them to the
/// response pane; the MCP server, the loader and the tests pass nothing and get
/// the body in one piece at the end, exactly as before.
pub type ChunkSink = Arc<dyn Fn(BodyEvent) + Send + Sync>;

pub async fn send(state: &HttpState, spec: RequestSpec) -> Result<ResponseData, HttpError> {
    send_streaming(state, spec, None).await
}

/// The headers reqwest's own redirect policy already removes on a cross-host
/// hop. A credential living in one of these can keep the fast path; anything
/// else has to be walked by hand.
fn reqwest_strips(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization" | "cookie" | "proxy-authorization" | "www-authenticate"
    )
}

/// Scheme, host and port — the parts a credential is scoped to.
fn origin_of(url: &reqwest::Url) -> (String, Option<String>, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().map(str::to_owned),
        url.port_or_known_default(),
    )
}

/// Everything `run` needs to issue — and, on the manual-redirect path,
/// re-issue — the request. Owned parts rather than a `RequestBuilder`, because
/// following a redirect by hand means building the next hop from them.
struct Outbound {
    client: reqwest::Client,
    method: reqwest::Method,
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    body: OutboundBody,
    timeout: Duration,
    /// Set only when redirects are followed here rather than by the client:
    /// the auth-injected header to shed the moment a hop leaves the original
    /// scheme+host+port.
    sensitive: Option<HeaderName>,
}

/// The body as it is actually sent. Multipart and files cannot be a `String`.
#[derive(Clone)]
enum OutboundBody {
    None,
    Text(String),
    Bytes(Vec<u8>),
    Form(Vec<(String, String)>),
    /// Rebuilt on each hop because `reqwest::multipart::Form` is not `Clone`.
    Multipart(Vec<FormField>),
}

pub async fn send_streaming(
    state: &HttpState,
    spec: RequestSpec,
    sink: Option<&ChunkSink>,
) -> Result<ResponseData, HttpError> {
    let method = reqwest::Method::from_bytes(spec.method.trim().as_bytes())
        .map_err(|_| HttpError::Method(spec.method.clone()))?;
    let url = crate::store::apply_path_params(spec.url.trim(), &spec.path_params);
    let url = reqwest::Url::parse(&url).map_err(|e| HttpError::Url(e.to_string()))?;

    let mut headers = reqwest::header::HeaderMap::new();
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
        headers.append(name, value);
    }

    // A credential in a custom header switches redirect handling to the manual
    // path (see `Outbound::sensitive`); reqwest already sheds the standard
    // ones itself, so those requests keep the client-level fast path.
    let sensitive = spec
        .sensitive_header
        .as_deref()
        .filter(|_| spec.follow_redirects)
        .filter(|name| !reqwest_strips(name))
        .and_then(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok());

    let client = state.client(ClientKey {
        section_id: spec.section_id.clone().unwrap_or_default(),
        follow_redirects: spec.follow_redirects && sensitive.is_none(),
        accept_invalid_certs: spec.accept_invalid_certs,
        proxy: spec.proxy.clone(),
    })?;

    let body = prepare_body(&spec).await?;
    if matches!(body, OutboundBody::Multipart(_) | OutboundBody::Form(_)) {
        // reqwest sets these Content-Types (multipart carries a boundary).
        // A leftover application/json from the JSON editor would win.
        headers.remove(CONTENT_TYPE);
    }

    let outbound = Outbound {
        client,
        method,
        url,
        headers,
        body,
        timeout: Duration::from_millis(spec.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
        sensitive,
    };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    state
        .inflight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(spec.id.clone(), cancel_tx);

    let outcome = run(outbound, cancel_rx, sink).await;
    state
        .inflight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&spec.id);
    outcome
}

async fn prepare_body(spec: &RequestSpec) -> Result<OutboundBody, HttpError> {
    match spec.body_kind {
        BodyKind::Json | BodyKind::Text => Ok(spec
            .body
            .as_ref()
            .filter(|body| !body.is_empty())
            .cloned()
            .map(OutboundBody::Text)
            .unwrap_or(OutboundBody::None)),
        BodyKind::Form => {
            let pairs: Vec<(String, String)> = spec
                .form
                .iter()
                .filter(|field| {
                    !field.name.trim().is_empty() && field.file.is_empty() && !field.is_file
                })
                .map(|field| (field.name.clone(), field.value.clone()))
                .collect();
            if pairs.is_empty() {
                Ok(OutboundBody::None)
            } else {
                Ok(OutboundBody::Form(pairs))
            }
        }
        BodyKind::Multipart => {
            if spec.form.iter().all(|field| field.name.trim().is_empty()) {
                Ok(OutboundBody::None)
            } else {
                Ok(OutboundBody::Multipart(spec.form.clone()))
            }
        }
        BodyKind::File => {
            let path = spec.file.trim();
            if path.is_empty() {
                return Ok(OutboundBody::None);
            }
            // `tokio::fs`, not `std::fs`: this runs on the async runtime, and a
            // large file read blocking one of its worker threads stalls every
            // other request being sent or streamed at the same time. The
            // multipart path below reads its files the same way.
            let bytes = tokio::fs::read(path).await.map_err(|err| {
                HttpError::Transport(format!(
                    "could not read {}: {err}",
                    Path::new(path).display()
                ))
            })?;
            Ok(OutboundBody::Bytes(bytes))
        }
    }
}

async fn apply_body(
    request: reqwest::RequestBuilder,
    body: &OutboundBody,
) -> Result<reqwest::RequestBuilder, HttpError> {
    match body {
        OutboundBody::None => Ok(request),
        OutboundBody::Text(text) => Ok(request.body(text.clone())),
        OutboundBody::Bytes(bytes) => Ok(request.body(bytes.clone())),
        OutboundBody::Form(pairs) => {
            let encoded = encode_form(pairs);
            Ok(request
                .header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                )
                .body(encoded))
        }
        OutboundBody::Multipart(fields) => Ok(request.multipart(multipart_form(fields).await?)),
    }
}

fn encode_form(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&form_encode(key));
        out.push('=');
        out.push_str(&form_encode(value));
    }
    out
}

fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

async fn multipart_form(fields: &[FormField]) -> Result<Form, HttpError> {
    let mut form = Form::new();
    for field in fields {
        let name = field.name.trim();
        if name.is_empty() {
            continue;
        }
        if field.file.trim().is_empty() {
            if field.is_file {
                continue;
            }
            form = form.text(name.to_string(), field.value.clone());
            continue;
        }
        let path = Path::new(field.file.trim());
        let bytes = tokio::fs::read(path).await.map_err(|err| {
            HttpError::Transport(format!("could not read {}: {err}", path.display()))
        })?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        let part = Part::bytes(bytes).file_name(file_name);
        form = form.part(name.to_string(), part);
    }
    Ok(form)
}

async fn run(
    outbound: Outbound,
    mut cancel: oneshot::Receiver<()>,
    sink: Option<&ChunkSink>,
) -> Result<ResponseData, HttpError> {
    let started = Instant::now();
    let Outbound {
        client,
        mut method,
        mut url,
        mut headers,
        mut body,
        timeout,
        sensitive,
    } = outbound;
    let origin = origin_of(&url);
    let mut hops = 0;

    let response = loop {
        let mut request = client
            .request(method.clone(), url.clone())
            .headers(headers.clone())
            .timeout(timeout);
        request = apply_body(request, &body).await?;

        let response = tokio::select! {
            biased;
            _ = &mut cancel => return Err(HttpError::Cancelled),
            result = request.send() => result.map_err(classify)?,
        };

        // Only the manual path looks at redirects — on the fast path the
        // client has already followed them and this is the final answer.
        let Some(sensitive_name) = sensitive.as_ref() else {
            break response;
        };
        if !response.status().is_redirection() {
            break response;
        }
        let Some(next) = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| url.join(location).ok())
        else {
            // A redirect status with no usable Location is the final answer.
            break response;
        };
        if hops >= MAX_REDIRECTS {
            return Err(HttpError::Transport("too many redirects".into()));
        }

        // The same method rewrites reqwest's own policy performs: 303 (and,
        // by long-standing convention, 301/302 for POST) replay as a bodyless
        // GET; 307/308 replay exactly as sent.
        let becomes_get = match response.status().as_u16() {
            301 | 302 => method == reqwest::Method::POST,
            303 => method != reqwest::Method::HEAD,
            307 | 308 => false,
            // 300, 304 and friends carry no follow semantics.
            _ => break response,
        };
        if becomes_get {
            method = reqwest::Method::GET;
            body = OutboundBody::None;
            headers.remove(CONTENT_TYPE);
            headers.remove(reqwest::header::CONTENT_LENGTH);
        }
        // The reason this loop exists: leaving the original origin takes the
        // credential header out of the request — and it stays out, even if a
        // later hop happens to lead back.
        if origin_of(&next) != origin {
            headers.remove(sensitive_name);
        }

        url = next;
        hops += 1;
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
    // Only text is streamed. Part of a binary body is not a string, and base64
    // of a fragment is not a prefix of base64 of the whole, so those still
    // arrive once at the end.
    let streaming =
        sink.filter(|_| looks_textual(content_type.as_deref()) || content_type.is_none());
    if let Some(sink) = streaming {
        sink(BodyEvent::Start);
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    // Bytes held back because they are the start of a character whose rest has
    // not arrived yet.
    let mut partial: Vec<u8> = Vec::new();
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

                    if let Some(sink) = streaming {
                        partial.extend_from_slice(&bytes[..take]);
                        let text = take_utf8(&mut partial);
                        if !text.is_empty() {
                            sink(BodyEvent::Chunk { text });
                        }
                    }
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
        body_streamed: streaming.is_some(),
        is_binary,
        truncated,
        size_bytes,
        timing: Timing { ttfb_ms, total_ms },
    })
}

/// The longest run of whole characters at the front of `pending`, taken out of
/// it. A character split across two chunks waits for the rest of itself.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let text = text.to_string();
            pending.clear();
            text
        }
        Err(error) => {
            let valid = error.valid_up_to();
            // Guaranteed valid, so nothing is actually replaced here.
            let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
            match error.error_len() {
                // Genuinely malformed rather than merely incomplete. Drop it,
                // or it sits at the front and stalls everything behind it.
                Some(len) => pending.drain(..valid + len),
                None => pending.drain(..valid),
            };
            text
        }
    }
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
    async fn one_shot_server(response: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
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
                    let has_body = text.to_lowercase().contains("content-length: ")
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
                request_id: "req-test".into(),
                section_id: None,
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
                sensitive_header: None,
                ..Default::default()
            },
        )
        .await
        .expect("request should succeed");

        assert_eq!(response.status, 201);
        assert_eq!(response.status_text, "Created");
        assert_eq!(response.body, "{\"created\":true}\n");
        assert!(!response.is_binary);
        assert!(!response.body_streamed);
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
                request_id: "req-test".into(),
                section_id: None,
                method: "GET".into(),
                url,
                headers: vec![],
                body: None,
                timeout_ms: Some(5_000),
                follow_redirects: true,
                accept_invalid_certs: false,
                sensitive_header: None,
                ..Default::default()
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
            request_id: "req-test".into(),
            section_id: None,
            method: "GET".into(),
            url: format!("http://{addr}"),
            headers: vec![],
            body: None,
            timeout_ms: Some(30_000),
            follow_redirects: true,
            accept_invalid_certs: false,
            sensitive_header: None,
            ..Default::default()
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

    /// A character can straddle two chunks. The half that has arrived is held
    /// back rather than shown as a replacement character.
    #[test]
    fn holds_back_a_character_split_across_chunks() {
        // "é" is two bytes; give it one at a time.
        let mut pending = vec![b'a', 0xC3];
        assert_eq!(take_utf8(&mut pending), "a");
        assert_eq!(pending, vec![0xC3], "the lone lead byte waits");

        pending.push(0xA9);
        assert_eq!(take_utf8(&mut pending), "é");
        assert!(pending.is_empty());
    }

    /// Truncation can leave a byte that will never become a character. It has
    /// to be dropped, or it sits at the front and stalls everything behind it.
    #[test]
    fn a_malformed_byte_does_not_stall_the_stream() {
        let mut pending = vec![b'a', 0xFF, b'b'];
        assert_eq!(take_utf8(&mut pending), "a");
        assert_eq!(take_utf8(&mut pending), "b");
        assert!(pending.is_empty());
    }

    #[test]
    fn takes_nothing_from_nothing() {
        let mut pending = Vec::new();
        assert_eq!(take_utf8(&mut pending), "");
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

    #[test]
    fn redacts_a_collection_specific_auth_header() {
        let headers = vec![
            Header {
                name: "X-Custom-Auth".into(),
                value: "secret".into(),
            },
            Header {
                name: "Content-Type".into(),
                value: "application/json".into(),
            },
        ];
        let redacted = redact_with(&headers, Some("x-custom-auth"));
        assert_eq!(redacted[0].value, "<redacted>");
        assert_eq!(redacted[1].value, "application/json");
    }

    /// Serves a scripted sequence of responses, one per connection, recording
    /// each request — enough to walk a redirect chain and see what was sent at
    /// every hop.
    async fn scripted_server(
        responses: Vec<String>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let requests = seen.clone();
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..read]).to_string());
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        (format!("http://{addr}"), seen)
    }

    fn spec_with_api_key(url: String) -> RequestSpec {
        RequestSpec {
            id: "redirect-test".into(),
            request_id: "req-test".into(),
            section_id: None,
            method: "GET".into(),
            url,
            headers: vec![Header {
                name: "X-Api-Key".into(),
                value: "sekret".into(),
            }],
            body: None,
            timeout_ms: Some(5_000),
            follow_redirects: true,
            accept_invalid_certs: false,
            sensitive_header: Some("X-Api-Key".into()),
            ..Default::default()
        }
    }

    /// A redirect that stays on the same scheme+host+port keeps the custom
    /// credential header — moving within the API is the normal case, and
    /// dropping the key there would break every redirecting endpoint.
    #[tokio::test]
    async fn a_same_host_redirect_keeps_a_custom_auth_header() {
        let (url, seen) = scripted_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: /landing\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".into(),
        ])
        .await;

        let state = HttpState::default();
        let response = send(&state, spec_with_api_key(format!("{url}/start")))
            .await
            .expect("request should succeed");

        assert_eq!(response.status, 200);
        assert!(
            response.final_url.ends_with("/landing"),
            "{}",
            response.final_url
        );

        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert!(
                request.to_lowercase().contains("x-api-key: sekret"),
                "the key should have travelled to both hops: {request}"
            );
        }
    }

    /// The gap reqwest leaves open: it sheds `Authorization` and `Cookie` on a
    /// cross-host redirect, but a custom header like `X-Api-Key` would ride
    /// along to whatever host the redirect named. It must not.
    #[tokio::test]
    async fn a_cross_host_redirect_drops_the_custom_auth_header() {
        let (elsewhere, landed) =
            scripted_server(vec!["HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".into()]).await;
        // Same loopback address, different port — a different origin.
        let (url, _) = scripted_server(vec![format!(
            "HTTP/1.1 302 Found\r\nLocation: {elsewhere}/landed\r\nContent-Length: 0\r\n\r\n"
        )])
        .await;

        let state = HttpState::default();
        let response = send(&state, spec_with_api_key(format!("{url}/start")))
            .await
            .expect("request should succeed");

        assert_eq!(response.status, 200);
        let requests = landed.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].to_lowercase().contains("x-api-key"),
            "the key crossed origins: {}",
            requests[0]
        );
        assert!(requests[0].starts_with("GET /landed"), "{}", requests[0]);
    }

    /// A 303 answer to a POST replays as a bodyless GET — the semantics the
    /// manual path has to reproduce because the client no longer does it.
    #[tokio::test]
    async fn a_see_other_redirect_becomes_a_get() {
        let (url, seen) = scripted_server(vec![
            "HTTP/1.1 303 See Other\r\nLocation: /result\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".into(),
        ])
        .await;

        let state = HttpState::default();
        let mut spec = spec_with_api_key(format!("{url}/submit"));
        spec.method = "POST".into();
        spec.body = Some("{\"a\":1}".into());

        let response = send(&state, spec).await.expect("request should succeed");
        assert_eq!(response.status, 200);

        let requests = seen.lock().unwrap();
        assert!(requests[0].starts_with("POST /submit"), "{}", requests[0]);
        assert!(requests[1].starts_with("GET /result"), "{}", requests[1]);
        assert!(
            !requests[1].contains("{\"a\":1}"),
            "the body should not replay"
        );
    }

    #[tokio::test]
    async fn sends_form_urlencoded_bodies() {
        let (url, server) = one_shot_server(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok",
        )
        .await;

        let state = HttpState::default();
        let response = send(
            &state,
            RequestSpec {
                id: "form-1".into(),
                method: "POST".into(),
                url: format!("{url}/form"),
                body_kind: BodyKind::Form,
                form: vec![
                    FormField {
                        name: "q".into(),
                        value: "a b".into(),
                        ..Default::default()
                    },
                    FormField {
                        name: "n".into(),
                        value: "1".into(),
                        ..Default::default()
                    },
                ],
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .expect("form post should succeed");
        assert_eq!(response.status, 200);

        let sent = server.await.unwrap();
        assert!(sent.contains("q=a+b&n=1"), "{sent}");
        assert!(
            sent.to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded"),
            "{sent}"
        );
    }

    #[tokio::test]
    async fn sends_a_file_as_the_raw_body() {
        let dir = std::env::temp_dir().join(format!("fiber-file-body-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("payload.bin");
        std::fs::write(&path, b"hello file").unwrap();

        let (url, server) =
            one_shot_server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;

        let state = HttpState::default();
        send(
            &state,
            RequestSpec {
                id: "file-1".into(),
                method: "POST".into(),
                url: format!("{url}/upload"),
                body_kind: BodyKind::File,
                file: path.to_string_lossy().into(),
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .expect("file post should succeed");

        let sent = server.await.unwrap();
        assert!(sent.contains("hello file"), "{sent}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cookie_jars_do_not_leak_across_sections() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let read = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();
                    let response = if request.starts_with("GET /set") {
                        "HTTP/1.1 200 OK\r\nSet-Cookie: sid=secret\r\nContent-Length: 2\r\n\r\nok"
                    } else if request.contains("sid=secret") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nyes"
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nno"
                    };
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        let base = format!("http://{addr}");
        let state = HttpState::default();
        send(
            &state,
            RequestSpec {
                id: "cookie-a".into(),
                section_id: Some("section-a".into()),
                method: "GET".into(),
                url: format!("{base}/set"),
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let leaked = send(
            &state,
            RequestSpec {
                id: "cookie-b".into(),
                section_id: Some("section-b".into()),
                method: "GET".into(),
                url: format!("{base}/echo"),
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            leaked.body, "no",
            "another collection must not inherit the jar"
        );

        let kept = send(
            &state,
            RequestSpec {
                id: "cookie-a2".into(),
                section_id: Some("section-a".into()),
                method: "GET".into(),
                url: format!("{base}/echo"),
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(kept.body, "yes", "the same collection keeps its cookie");
    }

    #[tokio::test]
    async fn substitutes_path_params_on_the_way_out() {
        let (url, server) =
            one_shot_server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;

        let state = HttpState::default();
        send(
            &state,
            RequestSpec {
                id: "path-1".into(),
                method: "GET".into(),
                url: format!("{url}/pet/{{petId}}"),
                path_params: vec![Header {
                    name: "petId".into(),
                    value: "a/b".into(),
                }],
                timeout_ms: Some(5_000),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let sent = server.await.unwrap();
        assert!(sent.starts_with("GET /pet/a%2Fb HTTP/1.1"), "{sent}");
    }
}
