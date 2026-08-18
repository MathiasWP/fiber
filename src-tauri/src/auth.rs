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
}

impl AuthConfig {
    pub fn secret_ref(&self) -> Option<&str> {
        match self {
            AuthConfig::None => None,
            AuthConfig::Bearer { secret_ref } | AuthConfig::Login { secret_ref, .. } => {
                Some(secret_ref)
            }
        }
    }

    /// Whether a 401 is worth retrying. A static token won't have changed.
    pub fn can_refresh(&self) -> bool {
        matches!(self, AuthConfig::Login { .. })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials stored for this section")]
    MissingSecret,
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

    fn store(&self, section_id: &str, value: String, ttl_seconds: u64) {
        let expires_at = (ttl_seconds > 0).then(|| Instant::now() + Duration::from_secs(ttl_seconds));
        self.tokens.lock().unwrap().insert(
            section_id.to_string(),
            CachedToken { value, expires_at },
        );
    }
}

/// The header to add to an outgoing request, if the section calls for one.
///
/// `secret` is whatever the keychain holds for `section.auth.secret_ref()`.
pub async fn header_for(
    state: &AuthState,
    http: &HttpState,
    section: &Section,
    secret: Option<String>,
) -> Result<Option<Header>, AuthError> {
    match &section.auth {
        AuthConfig::None => Ok(None),

        AuthConfig::Bearer { .. } => {
            let token = secret.ok_or(AuthError::MissingSecret)?;
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
                    let body = secret.ok_or(AuthError::MissingSecret)?;
                    let token =
                        log_in(http, section, method, url, &body, token_path).await?;
                    state.store(&section.id, token.clone(), *ttl_seconds);
                    token
                }
            };

            let name = if header.trim().is_empty() {
                "Authorization".to_string()
            } else {
                header.trim().to_string()
            };
            let value = match prefix.trim() {
                "" => token,
                prefix => format!("{prefix} {token}"),
            };
            Ok(Some(Header { name, value }))
        }
    }
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
    extract(&parsed, token_path).ok_or_else(|| AuthError::NoToken(token_path.to_string()))
}

/// Pulls a value out of a JSON document by dotted path.
///
/// Accepts an optional `$.` prefix, and numeric segments index into arrays:
/// `$.data.tokens.0.value`. Deliberately not a full JSONPath — the extra syntax
/// buys nothing for reading one field out of a login response.
fn extract(value: &Value, path: &str) -> Option<String> {
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn section(auth: AuthConfig, base_url: &str) -> Section {
        Section {
            id: "sec-1".into(),
            name: "Test".into(),
            base_url: base_url.into(),
            collapsed: false,
            auth,
            requests: vec![],
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

        assert_eq!(extract(&doc, "$.data.access_token").as_deref(), Some("abc"));
        assert_eq!(extract(&doc, "data.access_token").as_deref(), Some("abc"));
        assert_eq!(extract(&doc, "$.data.tokens.0.value").as_deref(), Some("first"));
        assert_eq!(extract(&doc, "$.n").as_deref(), Some("7"));
        assert_eq!(extract(&doc, "$.missing"), None);
        assert_eq!(extract(&doc, "$.data.tokens.9.value"), None);
        assert_eq!(extract(&doc, ""), None);
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

        let header = header_for(&state, &http, &section, Some("  tok123  ".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(header.name, "Authorization");
        assert_eq!(header.value, "Bearer tok123");

        let missing = header_for(&state, &http, &section, None).await;
        assert!(matches!(missing, Err(AuthError::MissingSecret)));
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
        use std::sync::atomic::Ordering;

        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(60), &base);
        let secret = Some(r#"{"user":"me","password":"hunter2"}"#.to_string());

        let first = header_for(&state, &http, &section, secret.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.value, "Bearer tok1");

        // Second call must reuse the cache, not hit the login endpoint again.
        let second = header_for(&state, &http, &section, secret.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.value, "Bearer tok1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Which is exactly what a 401 undoes.
        state.invalidate(&section.id);
        let third = header_for(&state, &http, &section, secret)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(third.value, "Bearer tok2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn zero_ttl_caches_until_invalidated() {
        use std::sync::atomic::Ordering;

        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(0), &base);
        let secret = Some("{}".to_string());

        header_for(&state, &http, &section, secret.clone()).await.unwrap();
        header_for(&state, &http, &section, secret.clone()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "ttl 0 means cache until 401");
    }

    #[tokio::test]
    async fn elapsed_ttl_forces_a_new_login() {
        use std::sync::atomic::Ordering;

        let (base, calls) = login_server().await;
        let state = AuthState::default();
        let http = HttpState::default();
        let section = section(login_auth(60), &base);
        let secret = Some("{}".to_string());

        let first = header_for(&state, &http, &section, secret.clone())
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

        let second = header_for(&state, &http, &section, secret)
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

        let outcome = header_for(&state, &http, &section, Some("{}".into())).await;
        assert!(matches!(outcome, Err(AuthError::Status(403))), "{outcome:?}");
    }
}
