//! Auth, with the token refresh built in.
//!
//! The point of this module is to kill "paste a fresh Bearer token every hour".
//! A section describes how to *obtain* a token — usually by making a request —
//! and the token is cached, injected, and re-fetched automatically when the API
//! answers 401. See §5 of the design doc.
//!
//! Secrets are passed in by the caller rather than read here, so the logic is
//! testable without touching the real keychain.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{Header, HttpState, RequestSpec};
use crate::store::{self, Section};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AuthConfig {
    #[default]
    None,

    /// A fixed token living in the keychain.
    Bearer { secret_ref: String },

    /// A token obtained by making a request, cached, and refreshed on 401.
    Login {
        method: String,
        /// Absolute, or relative to the section's base URL.
        url: String,
        /// Where the token is in the response, e.g. `$.data.access_token`.
        token_path: String,
        /// Header to inject. Usually `Authorization`.
        header: String,
        /// Scheme prefix. Usually `Bearer`.
        prefix: String,
        /// 0 means "cache until a 401 says otherwise".
        ttl_seconds: u64,
        /// Keychain reference for the login request body — it holds credentials,
        /// so it never touches the section file.
        secret_ref: String,
    },

    /// A credential lifted out of a real browser session.
    ///
    /// For flows a request can't reproduce: an emailed verification code, an
    /// SDK that mints the token in the page, or a session cookie the server
    /// sets and marks HttpOnly. You sign in normally in a real webview; the app
    /// takes the credential out afterwards.
    Browser {
        /// The page to open for signing in.
        login_url: String,
        capture: CaptureKind,
        /// localStorage key, or cookie name.
        capture_key: String,
        /// Dotted path into the stored JSON. Empty means use the raw value —
        /// Auth0's SDK, for instance, hides the token inside a JSON blob.
        capture_path: String,
        /// Composed with `prefix` into the outgoing header. `Cookie` with an
        /// empty prefix for cookie captures; `Authorization`/`Bearer` for tokens.
        header: String,
        prefix: String,
        ttl_seconds: u64,
        secret_ref: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKind {
    LocalStorage,
    Cookie,
    /// Keyed by `database/store/key` — see `browser::IndexedEntry`.
    IndexedDb,
}

impl AuthConfig {
    pub fn secret_ref(&self) -> Option<&str> {
        match self {
            AuthConfig::None => None,
            AuthConfig::Bearer { secret_ref }
            | AuthConfig::Login { secret_ref, .. }
            | AuthConfig::Browser { secret_ref, .. } => Some(secret_ref),
        }
    }

    /// Whether a 401 is worth retrying. A static token won't have changed;
    /// a login can be re-run, and a browser session can be re-captured.
    pub fn can_refresh(&self) -> bool {
        matches!(
            self,
            AuthConfig::Login { .. } | AuthConfig::Browser { .. }
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials stored for this section")]
    MissingSecret,
    #[error("not signed in — open Section settings and sign in")]
    NotSignedIn,
    #[error("login request failed: {0}")]
    Transport(String),
    #[error("login returned {0}")]
    Status(u16),
    #[error("login response was not JSON")]
    NotJson,
    #[error("no token at `{0}` in the login response")]
    NoToken(String),
}

struct CachedToken {
    value: String,
    /// `None` means it only expires when the API rejects it.
    expires_at: Option<Instant>,
}

impl CachedToken {
    fn valid(&self) -> bool {
        self.expires_at.is_none_or(|at| Instant::now() < at)
    }
}

#[derive(Default)]
pub struct AuthState {
    tokens: Mutex<HashMap<String, CachedToken>>,
}

impl AuthState {
    /// Drops a section's token so the next send fetches a new one.
    pub fn invalidate(&self, section_id: &str) {
        self.tokens.lock().unwrap().remove(section_id);
    }

    fn cached(&self, section_id: &str) -> Option<String> {
        let tokens = self.tokens.lock().unwrap();
        tokens
            .get(section_id)
            .filter(|token| token.valid())
            .map(|token| token.value.clone())
    }

    pub(crate) fn store(&self, section_id: &str, value: String, ttl_seconds: u64) {
        let expires_at = (ttl_seconds > 0).then(|| Instant::now() + Duration::from_secs(ttl_seconds));
        self.tokens.lock().unwrap().insert(
            section_id.to_string(),
            CachedToken { value, expires_at },
        );
    }
}

/// The header to add to an outgoing request, if the section calls for one.
///
/// `lookup` resolves `section.auth.secret_ref()` against the keychain, and is
/// taken as a closure rather than a value so it is only called on a cache miss.
/// That distinction is the whole point on macOS: an ad-hoc signed build cannot
/// hold a keychain ACL across releases, so every read is a password prompt, and
/// reading eagerly meant one prompt per request rather than one per app run.
pub async fn header_for<F>(
    state: &AuthState,
    http: &HttpState,
    section: &Section,
    lookup: &F,
) -> Result<Option<Header>, AuthError>
where
    F: Fn(&str) -> Option<String>,
{
    let fetch = || section.auth.secret_ref().and_then(|reference| lookup(reference));

    match &section.auth {
        AuthConfig::None => Ok(None),

        // Cached like a login token, though nothing expires it but a 401 or the
        // user replacing it — both of which already call `invalidate`.
        AuthConfig::Bearer { .. } => {
            let token = match state.cached(&section.id) {
                Some(token) => token,
                None => {
                    let token = fetch().ok_or(AuthError::MissingSecret)?;
                    state.store(&section.id, token.clone(), 0);
                    token
                }
            };
            Ok(Some(Header {
                name: "Authorization".into(),
                value: format!("Bearer {}", token.trim()),
            }))
        }

        AuthConfig::Login {
            method,
            url,
            token_path,
            header,
            prefix,
            ttl_seconds,
            ..
        } => {
            let token = match state.cached(&section.id) {
                Some(token) => token,
                None => {
                    let body = fetch().ok_or(AuthError::MissingSecret)?;
                    let token =
                        log_in(http, section, method, url, &body, token_path).await?;
                    state.store(&section.id, token.clone(), *ttl_seconds);
                    token
                }
            };

            Ok(Some(compose(header, prefix, &token)))
        }

        // The credential was lifted out of a browser session and stored as-is.
        // Re-capturing it needs a webview, which lives outside this module —
        // see `browser.rs`. Here we only compose what's already stored.
        AuthConfig::Browser { header, prefix, .. } => {
            let captured = match state.cached(&section.id) {
                Some(value) => value,
                None => {
                    let value = fetch().ok_or(AuthError::NotSignedIn)?;
                    state.store(&section.id, value.clone(), 0);
                    value
                }
            };
            Ok(Some(compose(header, prefix, &captured)))
        }
    }
}

/// `Authorization` + `Bearer` + token, or `Cookie` + nothing + `sid=…`.
fn compose(header: &str, prefix: &str, value: &str) -> Header {
    let name = match header.trim() {
        "" => "Authorization".to_string(),
        header => header.to_string(),
    };
    let value = match prefix.trim() {
        "" => value.trim().to_string(),
        prefix => format!("{prefix} {}", value.trim()),
    };
    Header { name, value }
}

async fn log_in(
    http: &HttpState,
    section: &Section,
    method: &str,
    url: &str,
    body: &str,
    token_path: &str,
) -> Result<String, AuthError> {
    let spec = RequestSpec {
        id: format!("auth:{}", section.id),
        request_id: String::new(),
        section_id: None,
        method: if method.trim().is_empty() {
            "POST".into()
        } else {
            method.trim().to_string()
        },
        url: store::join_url(&section.base_url, url),
        headers: vec![Header {
            name: "Content-Type".into(),
            value: "application/json".into(),
        }],
        body: Some(body.to_string()),
        timeout_ms: Some(30_000),
        follow_redirects: true,
        accept_invalid_certs: false,
    };

    let response = crate::http::send(http, spec)
        .await
        .map_err(|err| AuthError::Transport(err.to_string()))?;

    if !(200..300).contains(&response.status) {
        return Err(AuthError::Status(response.status));
    }

    let parsed: Value = serde_json::from_str(&response.body).map_err(|_| AuthError::NotJson)?;
    value_at(&parsed, token_path).ok_or_else(|| AuthError::NoToken(token_path.to_string()))
}

/// Pulls a value out of a JSON document by dotted path.
///
/// Shared with `browser.rs`, which uses it to dig a token out of whatever JSON
/// blob an SDK left in `localStorage`.
///
/// Accepts an optional `$.` prefix, and numeric segments index into arrays:
/// `$.data.tokens.0.value`. Deliberately not a full JSONPath — the extra syntax
/// buys nothing for reading one field out of a login response.
pub(crate) fn value_at(value: &Value, path: &str) -> Option<String> {
    let trimmed = path.trim().trim_start_matches('$').trim_start_matches('.');
    if trimmed.is_empty() {
        return None;
    }

    let mut current = value;
    for segment in trimmed.split('.') {
        if segment.is_empty() {
            continue;
        }
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    match current {
        Value::String(text) => Some(text.clone()),
        // A numeric or boolean token is unusual but not worth rejecting.
        Value::Number(_) | Value::Bool(_) => Some(current.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Section;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn section(auth: AuthConfig, base_url: &str) -> Section {
        Section {
            id: "sec-1".into(),
            name: "Test".into(),
            base_url: base_url.into(),
            collapsed: false,
            order: 0,
            auth,
            loader: None,
            mcp: Default::default(),
            requests: vec![],
            overlay: vec![],
        }
    }

    fn login_auth(ttl_seconds: u64) -> AuthConfig {
        AuthConfig::Login {
            method: "POST".into(),
            url: "/login".into(),
            token_path: "$.data.access_token".into(),
            header: "Authorization".into(),
            prefix: "Bearer".into(),
            ttl_seconds,
            secret_ref: "sec-1:login".into(),
        }
    }

    #[test]
    fn extracts_tokens_by_path() {
        let doc: Value = serde_json::from_str(
            r#"{"data":{"access_token":"abc","tokens":[{"value":"first"}]},"n":7}"#,
        )
        .unwrap();

        assert_eq!(value_at(&doc, "$.data.access_token").as_deref(), Some("abc"));
        assert_eq!(value_at(&doc, "data.access_token").as_deref(), Some("abc"));
        assert_eq!(value_at(&doc, "$.data.tokens.0.value").as_deref(), Some("first"));
        assert_eq!(value_at(&doc, "$.n").as_deref(), Some("7"));
        assert_eq!(value_at(&doc, "$.missing"), None);
        assert_eq!(value_at(&doc, "$.data.tokens.9.value"), None);
        assert_eq!(value_at(&doc, ""), None);
    }

    /// A stand-in for the keychain that records how often it was consulted —
    /// which is the thing these tests are really about on macOS, where each
    /// consultation is a password prompt.
    fn counted(value: Option<&str>) -> (impl Fn(&str) -> Option<String>, Arc<AtomicUsize>) {
        let owned = value.map(str::to_string);
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        (
            move |_reference: &str| {
                counter.fetch_add(1, Ordering::SeqCst);
                owned.clone()
            },
            reads,
        )
    }

    #[tokio::test]
    async fn bearer_uses_the_stored_secret() {
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(
            AuthConfig::Bearer {
                secret_ref: "sec-1:token".into(),
            },
            "https://example.com",
        );

        let (lookup, reads) = counted(Some("  tok123  "));

        let header = header_for(&state, &http, &section, &lookup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(header.name, "Authorization");
        assert_eq!(header.value, "Bearer tok123");
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        // The point of the exercise: a second send composes the same header
        // without going back to the keychain, so macOS has nothing to prompt
        // about.
        let again = header_for(&state, &http, &section, &lookup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.value, "Bearer tok123");
        assert_eq!(reads.load(Ordering::SeqCst), 1, "the cache should have served it");

        // And once the token is invalidated, it is read again.
        state.invalidate(&section.id);
        header_for(&state, &http, &section, &lookup).await.unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bearer_without_a_stored_secret_is_an_error() {
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(
            AuthConfig::Bearer {
                secret_ref: "sec-1:token".into(),
            },
            "https://example.com",
        );

        let missing = header_for(&state, &http, &section, &counted(None).0).await;
        assert!(matches!(missing, Err(AuthError::MissingSecret)));
    }

    /// The credential a browser session left behind is cached the same way, and
    /// for the same reason: it used to be re-read on every single send.
    #[tokio::test]
    async fn browser_credentials_are_read_once() {
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(
            AuthConfig::Browser {
                login_url: "https://example.com/login".into(),
                capture: CaptureKind::Cookie,
                capture_key: "sid".into(),
                capture_path: String::new(),
                header: "Cookie".into(),
                prefix: String::new(),
                ttl_seconds: 0,
                secret_ref: "sec-1:auth".into(),
            },
            "https://example.com",
        );
        let (lookup, reads) = counted(Some("sid=abc"));

        for _ in 0..3 {
            let header = header_for(&state, &http, &section, &lookup)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(header.name, "Cookie");
            assert_eq!(header.value, "sid=abc");
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1, "three sends, one keychain read");
    }

    /// Serves `/login`, handing out a new token each time it's called, so tests
    /// can tell a cached token from a freshly fetched one.
    async fn login_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));

        let counter = calls.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    let nth = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    let body = format!("{{\"data\":{{\"access_token\":\"tok{nth}\"}}}}");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });

        (format!("http://{addr}"), calls)
    }

    #[tokio::test]
    async fn logs_in_once_and_caches_the_token() {
        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(60), &base);
        let (secret, _reads) = counted(Some(r#"{"user":"me","password":"hunter2"}"#));

        let first = header_for(&state, &http, &section, &secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.value, "Bearer tok1");

        // Second call must reuse the cache, not hit the login endpoint again.
        let second = header_for(&state, &http, &section, &secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.value, "Bearer tok1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Which is exactly what a 401 undoes.
        state.invalidate(&section.id);
        let third = header_for(&state, &http, &section, &secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(third.value, "Bearer tok2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn zero_ttl_caches_until_invalidated() {
        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(0), &base);
        let (secret, _) = counted(Some("{}"));

        header_for(&state, &http, &section, &secret).await.unwrap();
        header_for(&state, &http, &section, &secret).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "ttl 0 means cache until 401");
    }

    #[tokio::test]
    async fn elapsed_ttl_forces_a_new_login() {
        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(60), &base);
        let (secret, _) = counted(Some("{}"));

        let first = header_for(&state, &http, &section, &secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.value, "Bearer tok1");

        // Rewind the expiry rather than sleeping for a minute.
        state
            .tokens
            .lock()
            .unwrap()
            .get_mut(&section.id)
            .unwrap()
            .expires_at = Some(Instant::now() - Duration::from_secs(1));

        let second = header_for(&state, &http, &section, &secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.value, "Bearer tok2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn surfaces_a_failed_login() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await;
            let _ = socket.flush().await;
        });

        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(60), &format!("http://{addr}"));

        let outcome = header_for(&state, &http, &section, &counted(Some("{}")).0).await;
        assert!(matches!(outcome, Err(AuthError::Status(403))), "{outcome:?}");
    }
}
