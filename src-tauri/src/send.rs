//! Authenticated send, with the 401 refresh-and-retry built in.
//!
//! Split out of the Tauri command layer so it stays free of Tauri types: both
//! the app and the headless MCP server drive the same logic, and the headless
//! build (no webview) must compile without Tauri at all.
//!
//! The one thing that genuinely needs a window — re-capturing a browser
//! credential on 401 — is reached through the [`Recapturer`] hook rather than a
//! Tauri handle. The app supplies one; the MCP server passes `None`, because it
//! has no window to sign in through.

use std::future::Future;
use std::pin::Pin;

use crate::auth::{self, AuthConfig, AuthState};
use crate::http::{self, HttpError, HttpState, RequestSpec, ResponseData};
use crate::secrets;
use crate::store::Section;

/// Lifts a fresh browser-captured credential, for the one auth kind a replayed
/// request can't refresh. Implemented by the GUI over a hidden webview; absent
/// under the MCP server, where a browser-captured credential can be used but not
/// re-captured. `Send + Sync` so the retry future stays `Send`.
pub(crate) trait Recapturer: Send + Sync {
    fn recapture<'a>(
        &'a self,
        section: &'a Section,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;
}

/// Applies the section's auth, and — the whole point of auth-as-a-request —
/// treats a 401 as "the cached token aged out": drop it, log in again, retry
/// once. Exactly once, so a genuinely unauthorised request can't loop.
///
/// `lookup` resolves a keychain (or injected) reference. Injected so the retry
/// logic can be tested without touching the real keychain.
pub(crate) async fn send_authenticated<F>(
    http_state: &HttpState,
    auth_state: &AuthState,
    section: Option<&Section>,
    spec: RequestSpec,
    lookup: &F,
    recapture: Option<&dyn Recapturer>,
) -> Result<ResponseData, HttpError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(section) = section else {
        return http::send(http_state, spec).await;
    };

    let prepared = apply_auth(http_state, auth_state, section, spec.clone(), lookup).await?;
    let first = http::send(http_state, prepared).await;

    let should_retry = matches!(&first, Ok(response) if response.status == 401)
        && section.auth.can_refresh();
    if !should_retry {
        return first;
    }

    log::info!("401 from {}, re-authenticating and retrying once", spec.url);
    auth_state.invalidate(&section.id);

    // A browser-captured credential can't be re-fetched by replaying a request,
    // so ask the host to lift a fresh one out of a hidden webview instead. If
    // the identity provider's own session is still alive this is invisible. The
    // MCP server passes no recapturer, so this branch is simply skipped there.
    if let (AuthConfig::Browser { .. }, Some(recapture)) = (&section.auth, recapture) {
        match recapture.recapture(section).await {
            Ok(value) => {
                if let Some(reference) = section.auth.secret_ref() {
                    if let Err(err) = secrets::set(reference, &value) {
                        log::warn!("could not store re-captured credential: {err}");
                    }
                }
                // Seed the cache from the value in hand. Without this the retry
                // below would read straight back out of the keychain the thing
                // we just wrote into it — a second prompt for no new information.
                auth_state.store(&section.id, value, 0);
            }
            // The window is now visible for the user to sign in; the original
            // 401 is the honest answer for this request.
            Err(err) => {
                log::info!("silent re-capture failed: {err}");
                return first;
            }
        }
    }

    match apply_auth(http_state, auth_state, section, spec, lookup).await {
        Ok(retry) => http::send(http_state, retry).await,
        // Re-authentication failed, so the original 401 is the honest answer.
        Err(_) => first,
    }
}

async fn apply_auth<F>(
    http_state: &HttpState,
    auth_state: &AuthState,
    section: &Section,
    mut spec: RequestSpec,
    lookup: &F,
) -> Result<RequestSpec, HttpError>
where
    F: Fn(&str) -> Option<String>,
{
    // The lookup goes in as a closure: `header_for` only calls it when its cache
    // has nothing, which is what keeps a keychain prompt to once per app run
    // rather than once per request.
    let header = auth::header_for(auth_state, http_state, section, lookup)
        .await
        .map_err(|err| HttpError::Auth(err.to_string()))?;

    if let Some(header) = header {
        // A header typed on the request wins — that's the escape hatch for
        // "just this once, use a different token".
        let already_set = spec
            .headers
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&header.name));
        if !already_set {
            spec.headers.push(header);
        }
    }

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Header;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Calls {
        logins: AtomicUsize,
        protected: AtomicUsize,
    }

    /// An API whose tokens go stale: `/login` mints tok1, tok2, …, and
    /// `/me` only accepts the most recently minted one.
    async fn stale_token_api(always_reject: bool) -> (String, Arc<Calls>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(Calls {
            logins: AtomicUsize::new(0),
            protected: AtomicUsize::new(0),
        });

        let counters = calls.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let counters = counters.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let read = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();

                    let response = if request.starts_with("POST /login") {
                        let nth = counters.logins.fetch_add(1, Ordering::SeqCst) + 1;
                        let body = format!("{{\"data\":{{\"access_token\":\"tok{nth}\"}}}}");
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    } else {
                        counters.protected.fetch_add(1, Ordering::SeqCst);
                        let issued = counters.logins.load(Ordering::SeqCst);
                        let accepted = format!("Bearer tok{issued}");
                        if !always_reject && request.contains(&accepted) {
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\ntrue".to_string()
                        } else {
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_string()
                        }
                    };

                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });

        (format!("http://{addr}"), calls)
    }

    fn section_with_login(base_url: &str) -> Section {
        Section {
            id: "sec-1".into(),
            name: "Test".into(),
            base_url: base_url.into(),
            collapsed: false,
            order: 0,
            auth: AuthConfig::Login {
                method: "POST".into(),
                url: "/login".into(),
                token_path: "$.data.access_token".into(),
                header: "Authorization".into(),
                prefix: "Bearer".into(),
                ttl_seconds: 0,
                secret_ref: "sec-1:login".into(),
            },
            loader: None,
            mcp: Default::default(),
            requests: vec![],
            overlay: vec![],
        }
    }

    fn spec_for(base_url: &str) -> RequestSpec {
        RequestSpec {
            id: "send-1".into(),
            request_id: "req-1".into(),
            section_id: Some("sec-1".into()),
            method: "GET".into(),
            url: format!("{base_url}/me"),
            headers: vec![],
            body: None,
            timeout_ms: Some(5_000),
            follow_redirects: true,
            accept_invalid_certs: false,
        }
    }

    fn secret(_reference: &str) -> Option<String> {
        Some(r#"{"user":"me"}"#.to_string())
    }

    /// The headline behaviour: a stale token produces a 401, which is silently
    /// re-authenticated and retried — the caller only ever sees the 200.
    #[tokio::test]
    async fn refreshes_and_retries_once_on_401() {
        let (base, calls) = stale_token_api(false).await;
        let http_state = HttpState::default();
        let auth_state = AuthState::default();
        let section = section_with_login(&base);

        // Prime the cache with a token that the API will have moved past.
        auth_state.invalidate(&section.id);
        let primed = apply_auth(&http_state, &auth_state, &section, spec_for(&base), &secret)
            .await
            .unwrap();
        assert!(primed.headers.iter().any(|h| h.value == "Bearer tok1"));

        // A second login (from another client, say) makes tok1 stale.
        let _ = http::send(
            &http_state,
            RequestSpec {
                id: "extra-login".into(),
                url: format!("{base}/login"),
                method: "POST".into(),
                ..spec_for(&base)
            },
        )
        .await;

        let response = send_authenticated(
            &http_state,
            &auth_state,
            Some(&section),
            spec_for(&base),
            &secret,
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.status, 200, "the retry should have succeeded");
        assert_eq!(
            calls.protected.load(Ordering::SeqCst),
            2,
            "one rejected attempt, then one retry"
        );
    }

    /// A genuine 401 must not loop: exactly one retry, then give up.
    #[tokio::test]
    async fn retries_at_most_once() {
        let (base, calls) = stale_token_api(true).await;
        let http_state = HttpState::default();
        let auth_state = AuthState::default();
        let section = section_with_login(&base);

        let response = send_authenticated(
            &http_state,
            &auth_state,
            Some(&section),
            spec_for(&base),
            &secret,
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.status, 401, "still unauthorised, honestly reported");
        assert_eq!(calls.protected.load(Ordering::SeqCst), 2, "no retry storm");
    }

    /// A header typed on the request is the escape hatch and must win.
    #[tokio::test]
    async fn an_explicit_header_is_not_overwritten() {
        let (base, _) = stale_token_api(false).await;
        let http_state = HttpState::default();
        let auth_state = AuthState::default();
        let section = section_with_login(&base);

        let mut spec = spec_for(&base);
        spec.headers.push(Header {
            name: "authorization".into(),
            value: "Bearer mine".into(),
        });

        let prepared = apply_auth(&http_state, &auth_state, &section, spec, &secret)
            .await
            .unwrap();

        let auth_headers: Vec<_> = prepared
            .headers
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case("authorization"))
            .collect();
        assert_eq!(auth_headers.len(), 1);
        assert_eq!(auth_headers[0].value, "Bearer mine");
    }

    /// No section means no auth machinery at all.
    #[tokio::test]
    async fn sections_are_optional() {
        let (base, calls) = stale_token_api(true).await;
        let http_state = HttpState::default();
        let auth_state = AuthState::default();

        let response =
            send_authenticated(&http_state, &auth_state, None, spec_for(&base), &secret, None)
                .await
                .unwrap();

        assert_eq!(response.status, 401);
        assert_eq!(calls.logins.load(Ordering::SeqCst), 0, "never logged in");
    }
}
