//! Browser session capture.
//!
//! Some login flows can't be reproduced by a request: an emailed verification
//! code, an SDK that mints the token inside the page, a session cookie the
//! server marks HttpOnly. So we open a real webview, let the user sign in
//! normally, and take the credential out afterwards.
//!
//! Three storage locations are read, because SDKs disagree about where a
//! session belongs: cookies, `localStorage`, and IndexedDB (Firebase's default).
//!
//! Two things make this work where a JavaScript-only approach wouldn't:
//!
//! - `cookies_for_url` returns **HttpOnly** cookies. The page can't read them;
//!   we can. That's the entire cookie-session case.
//! - `eval_with_callback` runs as host-evaluated script, so the page's CSP
//!   doesn't get a say in whether we can read storage.
//!
//! This is the one module that must know about Tauri — webviews are Tauri. The
//! rest of the auth path stays headless so the MCP server can use a stored
//! credential (it just can't capture a new one, having no UI).

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::auth::{AuthConfig, CaptureKind};
use crate::send::Recapturer;
use crate::store::Section;

/// How long a single `localStorage` read may take before we give up.
const EVAL_TIMEOUT: Duration = Duration::from_secs(5);
/// Silent re-capture gives the identity provider this long to settle.
const SILENT_ATTEMPTS: usize = 20;
const SILENT_INTERVAL: Duration = Duration::from_millis(500);
/// How long a window we opened ourselves gets to load before we trust an empty
/// reading.
const LOAD_ATTEMPTS: usize = 8;
const LOAD_INTERVAL: Duration = Duration::from_millis(400);
/// IndexedDB reads and MSAL decryption both run asynchronously in the page and
/// park their result, so we poll for it.
const PARKED_ATTEMPTS: usize = 10;
const PARKED_INTERVAL: Duration = Duration::from_millis(250);

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

/// One record from an IndexedDB object store. Identified by `database/store/key`
/// so a capture rule can name it the same way a localStorage key is named.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedEntry {
    pub database: String,
    pub store: String,
    pub key: String,
    pub value: String,
}

impl IndexedEntry {
    pub fn identity(&self) -> String {
        format!("{}/{}/{}", self.database, self.store, self.key)
    }
}

/// Everything we could find in the signed-in session, for the user to pick from.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub local_storage: Vec<StorageEntry>,
    pub cookies: Vec<CookieEntry>,
    pub indexed_db: Vec<IndexedEntry>,
}

impl Snapshot {
    fn is_empty(&self) -> bool {
        self.local_storage.is_empty() && self.cookies.is_empty() && self.indexed_db.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("this section is not set up for browser sign-in")]
    NotConfigured,
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

/// Reads the session, opening the sign-in window if it isn't already up.
///
/// The webview's storage is persistent, so a session survives closing the
/// window — which means the user shouldn't have to keep it open just to pick a
/// credential out of it. If we open the window here we open it hidden and close
/// it again, so this stays invisible.
pub async fn snapshot(app: &AppHandle, section: &Section) -> Result<Snapshot, BrowserError> {
    let existing = app.get_webview_window(&window_label(&section.id));
    let borrowed = existing.is_some();
    let window = match existing {
        Some(window) => window,
        None => open(app, section, false)?,
    };

    let found = if borrowed {
        read_session(&window, section).await
    } else {
        // Freshly opened: give the page time to load before believing an empty
        // result, but stop as soon as there's anything there.
        let mut last = Ok(Snapshot::default());
        for _ in 0..LOAD_ATTEMPTS {
            tokio::time::sleep(LOAD_INTERVAL).await;
            last = read_session(&window, section).await;
            if matches!(&last, Ok(found) if !found.is_empty()) {
                break;
            }
        }
        let _ = window.close();
        last
    };

    found
}

/// Reading IndexedDB is asynchronous, and a script that returns a Promise is no
/// use to `eval_with_callback` — the Promise itself is what would come back. So
/// the first call starts the read and parks the result on `window`; later calls
/// return it once it's there. `""` means "still working".
///
/// `indexedDB.databases()` enumerates them where it exists; well-known names are
/// tried regardless, since that call is comparatively recent.
const INDEXED_DB_JS: &str = r#"
(() => {
  const SLOT = "__fetchIdbSnapshot";
  const known = ["firebaseLocalStorageDb"];
  const MAX_ROWS_PER_STORE = 50;
  const MAX_VALUE_CHARS = 20000;

  const parked = window[SLOT];
  if (parked && parked.done) {
    // Clear on collection so the *next* read starts fresh. The same webview is
    // read again and again — the silent re-capture poll and the picker's
    // Refresh both reuse this window — and a session's IndexedDB changes
    // between those reads. A sticky result would pin the first reading forever,
    // hiding a credential that only appeared later (the whole point of polling).
    delete window[SLOT];
    return JSON.stringify(parked.items);
  }
  if (parked && parked.running) return "";
  window[SLOT] = { running: true, done: false, items: [] };

  const request = (req) =>
    new Promise((resolve) => {
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    });

  const read = async () => {
    const items = [];
    let names = [];
    try {
      if (indexedDB.databases) {
        names = (await indexedDB.databases()).map((entry) => entry.name).filter(Boolean);
      }
    } catch (error) {
      names = [];
    }
    for (const name of known) if (!names.includes(name)) names.push(name);

    for (const name of names) {
      let db = null;
      try {
        db = await request(indexedDB.open(name));
        if (!db) continue;
        for (const store of Array.from(db.objectStoreNames)) {
          try {
            const transaction = db.transaction(store, "readonly");
            const target = transaction.objectStore(store);
            const rows = (await request(target.getAll())) || [];
            const keys = (await request(target.getAllKeys())) || [];
            rows.slice(0, MAX_ROWS_PER_STORE).forEach((row, index) => {
              let value = "";
              try {
                value = typeof row === "string" ? row : JSON.stringify(row);
              } catch (error) {
                return;
              }
              if (typeof value !== "string") return;
              items.push({
                database: name,
                store,
                key: String(keys[index] ?? index),
                value: value.slice(0, MAX_VALUE_CHARS)
              });
            });
          } catch (error) {
            // Unreadable store; the rest may still be fine.
          }
        }
      } catch (error) {
        // Unopenable database; likewise.
      } finally {
        try { if (db) db.close(); } catch (error) {}
      }
    }
    return items;
  };

  read()
    .then((items) => { window[SLOT] = { running: false, done: true, items }; })
    .catch(() => { window[SLOT] = { running: false, done: true, items: [] }; });

  return "";
})()
"#;

/// MSAL v4 encrypts its localStorage cache unless the user ticked "keep me
/// signed in", which would otherwise leave a Microsoft-authenticated section
/// with nothing to capture.
///
/// The scheme is documented by its own source: the base key lives in a
/// `msal.cache.encryption` session cookie as `{"id","key"}` with the key
/// url-safe base64; each entry is `{"id","nonce","data"}`; the per-entry key is
/// HKDF-SHA256 over the base key with the 16-byte nonce as salt and the app's
/// clientId as info; the cipher is AES-GCM-256 with a zero 12-byte IV.
///
/// We don't know the clientId, but MSAL only uses it as context when the cache
/// key contains it — so it's always either empty or a GUID inside the key
/// itself. Trying that short list is enough, and AES-GCM authentication rejects
/// a wrong guess cleanly rather than yielding garbage.
///
/// MSAL is explicit that this encryption exists to limit how long artifacts
/// persist, not to secure them. Decrypting here reads the user's own session on
/// their own machine, in the same browser profile that wrote it.
const MSAL_DECRYPT_JS: &str = r#"
(() => {
  const SLOT = "__fetchMsalSnapshot";
  const parked = window[SLOT];
  if (parked && parked.done) {
    // See INDEXED_DB_JS: clear so a re-read (silent re-capture, Refresh) sees a
    // freshly decrypted cache rather than the first reading pinned in place.
    delete window[SLOT];
    return JSON.stringify(parked.items);
  }
  if (parked && parked.running) return "";
  window[SLOT] = { running: true, done: false, items: [] };

  const fromBase64Url = (text) => {
    const standard = text.replace(/-/g, "+").replace(/_/g, "/");
    const padding = (4 - (standard.length % 4)) % 4;
    const binary = atob(standard + "=".repeat(padding));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  };

  const cookie = (name) => {
    for (const part of (document.cookie || "").split(";")) {
      const at = part.indexOf("=");
      if (at < 0) continue;
      if (part.slice(0, at).trim() === name) {
        return decodeURIComponent(part.slice(at + 1).trim());
      }
    }
    return null;
  };

  const GUID = /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g;

  const run = async () => {
    const raw = cookie("msal.cache.encryption");
    if (!raw) return [];
    let envelope;
    try { envelope = JSON.parse(raw); } catch (error) { return []; }
    if (!envelope || typeof envelope.key !== "string") return [];

    const baseKey = await crypto.subtle.importKey(
      "raw", fromBase64Url(envelope.key), "HKDF", false, ["deriveKey"]
    );

    const items = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      const value = localStorage.getItem(key);
      if (!value || value.charAt(0) !== "{") continue;

      let blob;
      try { blob = JSON.parse(value); } catch (error) { continue; }
      if (!blob || typeof blob.nonce !== "string" || typeof blob.data !== "string") continue;

      for (const context of [""].concat(key.match(GUID) || [])) {
        try {
          const derived = await crypto.subtle.deriveKey(
            {
              name: "HKDF",
              hash: "SHA-256",
              salt: fromBase64Url(blob.nonce),
              info: new TextEncoder().encode(context)
            },
            baseKey,
            { name: "AES-GCM", length: 256 },
            false,
            ["decrypt"]
          );
          const plain = await crypto.subtle.decrypt(
            { name: "AES-GCM", iv: new Uint8Array(12) },
            derived,
            fromBase64Url(blob.data)
          );
          items.push({ key, value: new TextDecoder().decode(plain) });
          break;
        } catch (error) {
          // Wrong context — authentication failed, try the next candidate.
        }
      }
    }
    return items;
  };

  run()
    .then((items) => { window[SLOT] = { running: false, done: true, items }; })
    .catch(() => { window[SLOT] = { running: false, done: true, items: [] }; });

  return "";
})()
"#;

/// Runs a script that parks its result on `window`, polling until it lands.
/// `""` means "still working".
async fn read_parked<T: serde::de::DeserializeOwned>(
    window: &WebviewWindow,
    js: &str,
) -> Vec<T> {
    for attempt in 0..PARKED_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(PARKED_INTERVAL).await;
        }
        let Ok(raw) = eval_json(window, js).await else {
            return Vec::new();
        };
        if raw.trim().is_empty() || raw.trim() == "\"\"" {
            continue;
        }
        return parse_json_list(&raw);
    }
    Vec::new()
}

/// Swaps decrypted values in for the ciphertext blobs they came from, so
/// everything downstream sees ordinary readable storage entries.
fn merge_decrypted(entries: &mut Vec<StorageEntry>, decrypted: Vec<StorageEntry>) {
    for entry in decrypted {
        match entries.iter_mut().find(|existing| existing.key == entry.key) {
            Some(existing) => existing.value = entry.value,
            None => entries.push(entry),
        }
    }
}

/// Reads `localStorage` and every cookie visible to an open window.
async fn read_session(
    window: &WebviewWindow,
    section: &Section,
) -> Result<Snapshot, BrowserError> {
    let (login_url, ..) = browser_config(section)?;

    let mut local_storage = read_local_storage(window).await?;
    // MSAL entries arrive as ciphertext; put the plaintext in their place.
    merge_decrypted(&mut local_storage, read_parked(window, MSAL_DECRYPT_JS).await);

    // Cookies for both origins: the API we'll be calling, and the identity
    // provider we just signed in to. Often they differ.
    // The whole cookie store, not just the two origins we happen to know about.
    // A sign-in commonly ends up setting the session cookie on a host that is
    // neither the API base nor the login URL — a staging subdomain, an apex
    // domain, the identity provider — and asking only about known URLs makes
    // those invisible.
    let mut cookies = Vec::new();
    let mut collect = |found: Vec<tauri::webview::Cookie<'static>>| {
        for cookie in found {
            let name = cookie.name().to_string();
            let domain = cookie.domain().unwrap_or_default().to_string();
            // Keyed by name *and* domain: the same name on two hosts is two
            // different cookies, and only one of them is the one you want.
            if cookies
                .iter()
                .any(|existing: &CookieEntry| existing.name == name && existing.domain == domain)
            {
                continue;
            }
            cookies.push(CookieEntry {
                name,
                value: cookie.value().to_string(),
                domain,
                http_only: cookie.http_only().unwrap_or(false),
            });
        }
    };

    if let Ok(found) = window.cookies() {
        collect(found);
    }
    // Belt and braces: ask about the known origins too, in case the whole-store
    // read under-reports on some platform.
    for candidate in [section.base_url.as_str(), login_url] {
        let Ok(url) = candidate.trim().parse() else {
            continue;
        };
        if let Ok(found) = window.cookies_for_url(url) {
            collect(found);
        }
    }

    Ok(Snapshot {
        local_storage,
        cookies,
        indexed_db: read_parked(window, INDEXED_DB_JS).await,
    })
}

/// Runs a script and waits for its result. `eval_with_callback` is one-shot, so
/// the callback hands the value over a channel.
async fn eval_json(window: &WebviewWindow, js: &str) -> Result<String, BrowserError> {
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));

    window
        .eval_with_callback(js, move |raw| {
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(raw);
            }
        })
        .map_err(|err| BrowserError::Eval(err.to_string()))?;

    tokio::time::timeout(EVAL_TIMEOUT, receiver)
        .await
        .map_err(|_| BrowserError::Timeout)?
        .map_err(|_| BrowserError::Timeout)
}

async fn read_local_storage(window: &WebviewWindow) -> Result<Vec<StorageEntry>, BrowserError> {
    let raw = eval_json(window, SNAPSHOT_JS).await?;
    Ok(parse_json_list(&raw))
}

/// The callback hands back the evaluation result already JSON-encoded, and our
/// scripts return a JSON string — so what arrives is usually double-encoded.
/// Tolerate both shapes rather than depending on which.
fn parse_json_list<T: serde::de::DeserializeOwned>(raw: &str) -> Vec<T> {
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

        // Auth0's storage key embeds a client id and audience, and Firebase's
        // embeds an API key, so an exact match is brittle across environments;
        // falling back to a prefix isn't.
        CaptureKind::LocalStorage => {
            let entry = find_by_key(&snapshot.local_storage, key, |entry| &entry.key)?;
            dig(&entry.value, path)
        }

        CaptureKind::IndexedDb => {
            let entry = snapshot
                .indexed_db
                .iter()
                .find(|entry| entry.identity() == key)
                .or_else(|| {
                    snapshot
                        .indexed_db
                        .iter()
                        .find(|entry| entry.identity().starts_with(key))
                })?;
            dig(&entry.value, path)
        }
    }
}

/// Exact key first, then prefix.
fn find_by_key<'a, T>(
    entries: &'a [T],
    key: &str,
    key_of: impl Fn(&'a T) -> &'a String + Copy,
) -> Option<&'a T> {
    entries
        .iter()
        .find(|entry| key_of(entry) == key)
        .or_else(|| entries.iter().find(|entry| key_of(entry).starts_with(key)))
}

/// The whole stored value, or one field out of it when a path is given.
fn dig(value: &str, path: &str) -> Option<String> {
    if path.trim().is_empty() {
        return Some(value.to_string());
    }
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    crate::auth::value_at(&parsed, path)
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

    // `read_session` rather than `snapshot`, which would open and close a window
    // of its own on top of the one being polled here.
    for _ in 0..SILENT_ATTEMPTS {
        tokio::time::sleep(SILENT_INTERVAL).await;
        if let Ok(found) = read_session(&window, section).await {
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

/// The GUI's [`Recapturer`]: on a 401 for a browser-captured section, lift a
/// fresh credential out of a hidden webview. This is the one piece of the send
/// path that needs a window, so it lives here rather than in the Tauri-free
/// `send` module.
pub struct BrowserRecapture {
    app: AppHandle,
}

impl BrowserRecapture {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Recapturer for BrowserRecapture {
    fn recapture<'a>(
        &'a self,
        section: &'a Section,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            silent_recapture(&self.app, section)
                .await
                .map_err(|err| err.to_string())
        })
    }
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
            order: 0,
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
            loader: None,
            mcp: Default::default(),
            requests: vec![],
            overlay: vec![],
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
            indexed_db: vec![
                IndexedEntry {
                    database: "firebaseLocalStorageDb".into(),
                    store: "firebaseLocalStorage".into(),
                    key: "firebase:authUser:AIzaKEY:[DEFAULT]".into(),
                    value: r#"{"value":{"stsTokenManager":{"accessToken":"idb-tok"}}}"#.into(),
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
    fn digs_a_token_out_of_indexeddb() {
        // Firebase's JS SDK keeps its session here rather than in localStorage,
        // under firebaseLocalStorageDb/firebaseLocalStorage.
        let section = section(
            CaptureKind::IndexedDb,
            "firebaseLocalStorageDb/firebaseLocalStorage/firebase:authUser:AIzaKEY:[DEFAULT]",
            "value.stsTokenManager.accessToken",
        );
        assert_eq!(
            extract(&auth0_snapshot(), &section).as_deref(),
            Some("idb-tok")
        );
    }

    #[test]
    fn matches_an_indexeddb_record_by_prefix() {
        // The API key in Firebase's record key differs per environment.
        let section = section(
            CaptureKind::IndexedDb,
            "firebaseLocalStorageDb/firebaseLocalStorage/firebase:authUser:",
            "value.stsTokenManager.accessToken",
        );
        assert_eq!(
            extract(&auth0_snapshot(), &section).as_deref(),
            Some("idb-tok")
        );
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
    fn decrypted_msal_entries_replace_their_ciphertext() {
        let mut entries = vec![
            StorageEntry {
                key: "acct.env-accesstoken-cid--scope".into(),
                // What MSAL v4 actually leaves behind: an encrypted envelope.
                value: r#"{"id":"abc","nonce":"n0nce","data":"c1ph3r"}"#.into(),
            },
            StorageEntry {
                key: "untouched".into(),
                value: "plain".into(),
            },
        ];

        merge_decrypted(
            &mut entries,
            vec![
                StorageEntry {
                    key: "acct.env-accesstoken-cid--scope".into(),
                    value: r#"{"secret":"tok-msal"}"#.into(),
                },
                StorageEntry {
                    key: "only-in-decrypted".into(),
                    value: "extra".into(),
                },
            ],
        );

        assert_eq!(entries.len(), 3, "decryption replaces, it doesn't duplicate");
        assert_eq!(entries[0].value, r#"{"secret":"tok-msal"}"#);
        assert_eq!(entries[1].value, "plain");
        assert_eq!(entries[2].key, "only-in-decrypted");

        // And the replaced entry now reads like any other storage value, so the
        // ordinary capture rule finds the token.
        assert_eq!(dig(&entries[0].value, "secret").as_deref(), Some("tok-msal"));
    }

    #[test]
    fn tolerates_double_encoded_eval_results() {
        let items = r#"[{"key":"a","value":"1"}]"#;
        // Returned as a bare JSON array…
        assert_eq!(parse_json_list::<StorageEntry>(items).len(), 1);
        // …or as a JSON-encoded string containing that array.
        let wrapped = serde_json::to_string(items).unwrap();
        assert_eq!(parse_json_list::<StorageEntry>(&wrapped).len(), 1);
        // …or as something unusable.
        assert!(parse_json_list::<StorageEntry>("not json").is_empty());
    }
}
