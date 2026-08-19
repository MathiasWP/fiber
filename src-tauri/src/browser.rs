//! Browser session capture.
//!
//! Some login flows can't be reproduced by a request: an emailed verification
//! code, an SDK that mints the token inside the page, a session cookie the
//! server marks HttpOnly. So we open a real webview, let the user sign in
//! normally, and take the credential out afterwards.
//!
//! Two things make this work where a JavaScript-only approach wouldn't:
//!
//! - `cookies_for_url` returns **HttpOnly** cookies. The page can't read them;
//!   we can. That's the entire cookie-session case.
//! - `eval_with_callback` runs as host-evaluated script, so the page's CSP
//!   doesn't get a say in whether we can read `localStorage`.
//!
//! This is the one module that must know about Tauri — webviews are Tauri. The
//! rest of the auth path stays headless so the MCP server can use a stored
//! credential (it just can't capture a new one, having no UI).

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::auth::{AuthConfig, CaptureKind};
use crate::store::Section;

/// How long a single `localStorage` read may take before we give up.
const EVAL_TIMEOUT: Duration = Duration::from_secs(5);
/// Silent re-capture gives the identity provider this long to settle.
const SILENT_ATTEMPTS: usize = 20;
const SILENT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    /// True when the page's own JavaScript could not have read this.
    pub http_only: bool,
}

/// Everything we could find in the signed-in session, for the user to pick from.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub local_storage: Vec<StorageEntry>,
    pub cookies: Vec<CookieEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("this section is not set up for browser sign-in")]
    NotConfigured,
    #[error("the sign-in window isn't open")]
    NoWindow,
    #[error("could not open the sign-in window: {0}")]
    Open(String),
    #[error("timed out reading the sign-in window")]
    Timeout,
    #[error("could not read the sign-in window: {0}")]
    Eval(String),
    #[error("invalid URL `{0}`")]
    Url(String),
    #[error("nothing matched the capture rule — sign in again")]
    NothingCaptured,
}

impl Serialize for BrowserError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

fn window_label(section_id: &str) -> String {
    // Labels allow alphanumerics plus `-`, `/`, `:` and `_`.
    format!("auth-{}", section_id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-"))
}

fn browser_config(section: &Section) -> Result<(&str, CaptureKind, &str, &str), BrowserError> {
    match &section.auth {
        AuthConfig::Browser {
            login_url,
            capture,
            capture_key,
            capture_path,
            ..
        } => Ok((login_url, *capture, capture_key, capture_path)),
        _ => Err(BrowserError::NotConfigured),
    }
}

/// Opens the sign-in window, or focuses it if it's already open. `visible`
/// is false for silent re-capture, where we hope the user never sees it.
pub fn open(app: &AppHandle, section: &Section, visible: bool) -> Result<WebviewWindow, BrowserError> {
    let (login_url, ..) = browser_config(section)?;
    let label = window_label(&section.id);

    if let Some(existing) = app.get_webview_window(&label) {
        if visible {
            let _ = existing.show();
            let _ = existing.set_focus();
        }
        return Ok(existing);
    }

    let url = login_url
        .parse()
        .map_err(|_| BrowserError::Url(login_url.to_string()))?;

    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(format!("Sign in — {}", section.name))
        .inner_size(480.0, 720.0)
        .visible(visible)
        .build()
        .map_err(|err| BrowserError::Open(err.to_string()))
}

pub fn close(app: &AppHandle, section_id: &str) {
    if let Some(window) = app.get_webview_window(&window_label(section_id)) {
        let _ = window.close();
    }
}

const SNAPSHOT_JS: &str = r#"
(() => {
  try {
    const items = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      items.push({ key, value: localStorage.getItem(key) ?? "" });
    }
    return JSON.stringify(items);
  } catch (error) {
    return JSON.stringify([]);
  }
})()
"#;

/// Reads `localStorage` and every cookie visible to the session.
pub async fn snapshot(
    app: &AppHandle,
    section: &Section,
) -> Result<Snapshot, BrowserError> {
    let (login_url, ..) = browser_config(section)?;
    let window = app
        .get_webview_window(&window_label(&section.id))
        .ok_or(BrowserError::NoWindow)?;

    let local_storage = read_local_storage(&window).await?;

    // Cookies for both origins: the API we'll be calling, and the identity
    // provider we just signed in to. Often they differ.
    let mut cookies = Vec::new();
    for candidate in [section.base_url.as_str(), login_url] {
        let Ok(url) = candidate.trim().parse() else {
            continue;
        };
        if let Ok(found) = window.cookies_for_url(url) {
            for cookie in found {
                let name = cookie.name().to_string();
                if cookies
                    .iter()
                    .any(|existing: &CookieEntry| existing.name == name)
                {
                    continue;
                }
                cookies.push(CookieEntry {
                    name,
                    value: cookie.value().to_string(),
                    domain: cookie.domain().unwrap_or_default().to_string(),
                    http_only: cookie.http_only().unwrap_or(false),
                });
            }
        }
    }

    Ok(Snapshot {
        local_storage,
        cookies,
    })
}

async fn read_local_storage(window: &WebviewWindow) -> Result<Vec<StorageEntry>, BrowserError> {
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));

    window
        .eval_with_callback(SNAPSHOT_JS, move |raw| {
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(raw);
            }
        })
        .map_err(|err| BrowserError::Eval(err.to_string()))?;

    let raw = tokio::time::timeout(EVAL_TIMEOUT, receiver)
        .await
        .map_err(|_| BrowserError::Timeout)?
        .map_err(|_| BrowserError::Timeout)?;

    Ok(parse_entries(&raw))
}

/// The callback hands back the evaluation result already JSON-encoded, and our
/// script returns a JSON string — so what arrives is usually double-encoded.
/// Tolerate both shapes rather than depending on which.
fn parse_entries(raw: &str) -> Vec<StorageEntry> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let inner = match value {
        serde_json::Value::String(text) => match serde_json::from_str(&text) {
            Ok(parsed) => parsed,
            Err(_) => return Vec::new(),
        },
        other => other,
    };
    serde_json::from_value(inner).unwrap_or_default()
}

/// Applies a section's capture rule to a snapshot.
pub fn extract(snapshot: &Snapshot, section: &Section) -> Option<String> {
    let (_, capture, key, path) = browser_config(section).ok()?;
    if key.trim().is_empty() {
        return None;
    }

    match capture {
        CaptureKind::Cookie => snapshot
            .cookies
            .iter()
            .find(|cookie| cookie.name == key)
            .map(|cookie| format!("{}={}", cookie.name, cookie.value)),

        CaptureKind::LocalStorage => {
            let entry = snapshot
                .local_storage
                .iter()
                // Auth0's key embeds a client id and audience, so an exact
                // match is brittle across environments; a prefix isn't.
                .find(|entry| entry.key == key)
                .or_else(|| {
                    snapshot
                        .local_storage
                        .iter()
                        .find(|entry| entry.key.starts_with(key))
                })?;

            if path.trim().is_empty() {
                return Some(entry.value.clone());
            }
            let parsed: serde_json::Value = serde_json::from_str(&entry.value).ok()?;
            crate::auth::value_at(&parsed, path)
        }
    }
}

/// Tries to refresh a browser-captured credential without bothering the user.
///
/// Opens the sign-in page hidden. If the identity provider's own session is
/// still alive — usually it is, for days — the page re-authenticates on its own
/// and we lift the new credential out without a window ever appearing. If it
/// isn't, we show the window so the user can sign in, and report failure.
pub async fn silent_recapture(
    app: &AppHandle,
    section: &Section,
) -> Result<String, BrowserError> {
    // If a window is already open the user is probably mid-login; don't disturb it.
    let already_open = app
        .get_webview_window(&window_label(&section.id))
        .is_some();
    let window = open(app, section, already_open)?;

    for _ in 0..SILENT_ATTEMPTS {
        tokio::time::sleep(SILENT_INTERVAL).await;
        if let Ok(found) = snapshot(app, section).await {
            if let Some(value) = extract(&found, section) {
                if !already_open {
                    let _ = window.close();
                }
                return Ok(value);
            }
        }
    }

    // Silent path failed — surface the window so signing in is one click away.
    let _ = window.show();
    let _ = window.set_focus();
    Err(BrowserError::NothingCaptured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthConfig;

    fn section(capture: CaptureKind, key: &str, path: &str) -> Section {
        Section {
            id: "sec-1".into(),
            name: "Test".into(),
            base_url: "https://api.example.com".into(),
            collapsed: false,
            auth: AuthConfig::Browser {
                login_url: "https://login.example.com".into(),
                capture,
                capture_key: key.into(),
                capture_path: path.into(),
                header: "Authorization".into(),
                prefix: "Bearer".into(),
                ttl_seconds: 0,
                secret_ref: "sec-1:auth".into(),
            },
            requests: vec![],
        }
    }

    fn auth0_snapshot() -> Snapshot {
        Snapshot {
            local_storage: vec![
                StorageEntry {
                    key: "unrelated".into(),
                    value: "noise".into(),
                },
                StorageEntry {
                    // The shape Auth0's SPA SDK actually writes.
                    key: "@@auth0spajs@@::abc123::https://api.example.com::openid profile".into(),
                    value: r#"{"body":{"access_token":"tok-abc","expires_in":86400},"expiresAt":1}"#
                        .into(),
                },
            ],
            cookies: vec![
                CookieEntry {
                    name: "sid".into(),
                    value: "session-xyz".into(),
                    domain: "api.example.com".into(),
                    http_only: true,
                },
                CookieEntry {
                    name: "theme".into(),
                    value: "dark".into(),
                    domain: "api.example.com".into(),
                    http_only: false,
                },
            ],
        }
    }

    #[test]
    fn pulls_a_token_out_of_the_auth0_blob() {
        let section = section(
            CaptureKind::LocalStorage,
            "@@auth0spajs@@::abc123::https://api.example.com::openid profile",
            "body.access_token",
        );
        assert_eq!(
            extract(&auth0_snapshot(), &section).as_deref(),
            Some("tok-abc")
        );
    }

    #[test]
    fn matches_an_auth0_key_by_prefix() {
        // Client id and audience move between environments; the prefix doesn't.
        let section = section(CaptureKind::LocalStorage, "@@auth0spajs@@", "body.access_token");
        assert_eq!(
            extract(&auth0_snapshot(), &section).as_deref(),
            Some("tok-abc")
        );
    }

    #[test]
    fn an_empty_path_takes_the_whole_value() {
        let section = section(CaptureKind::LocalStorage, "unrelated", "");
        assert_eq!(extract(&auth0_snapshot(), &section).as_deref(), Some("noise"));
    }

    #[test]
    fn captures_a_cookie_as_a_name_value_pair() {
        let section = section(CaptureKind::Cookie, "sid", "");
        assert_eq!(
            extract(&auth0_snapshot(), &section).as_deref(),
            Some("sid=session-xyz")
        );
    }

    #[test]
    fn reports_nothing_when_the_rule_does_not_match() {
        assert_eq!(extract(&auth0_snapshot(), &section(CaptureKind::Cookie, "absent", "")), None);
        assert_eq!(
            extract(&auth0_snapshot(), &section(CaptureKind::LocalStorage, "", "")),
            None
        );
        assert_eq!(
            extract(
                &auth0_snapshot(),
                &section(CaptureKind::LocalStorage, "@@auth0spajs@@", "body.missing")
            ),
            None
        );
    }

    #[test]
    fn tolerates_double_encoded_eval_results() {
        let items = r#"[{"key":"a","value":"1"}]"#;
        // Returned as a bare JSON array…
        assert_eq!(parse_entries(items).len(), 1);
        // …or as a JSON-encoded string containing that array.
        let wrapped = serde_json::to_string(items).unwrap();
        assert_eq!(parse_entries(&wrapped).len(), 1);
        // …or as something unusable.
        assert!(parse_entries("not json").is_empty());
    }
}
