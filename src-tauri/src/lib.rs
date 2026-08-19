mod auth;
mod browser;
mod history;
mod loader;
pub mod mcp;
mod migrate;
mod openapi;
mod http;
mod secrets;
mod store;

use std::path::PathBuf;
use std::sync::Arc;

use auth::{AuthConfig, AuthState};
use browser::{BrowserError, Snapshot};
use history::{HistoryError, HistoryRecord, HistoryStore};
use loader::{LoaderError, LoaderRun};
use http::{HttpError, HttpState, RequestSpec, ResponseData};
use secrets::SecretError;
use store::{Section, StoreError};
use tauri::{AppHandle, Manager, State};

/// How many history entries the UI gets on startup.
const HISTORY_PAGE: usize = 500;

/// Resolved once at startup so every command agrees on where collections live.
struct Paths {
    sections: PathBuf,
    loaders: PathBuf,
}

/// Sends, and records the outcome. History is written here rather than by the
/// frontend so the body never has to travel back down the IPC bridge, and so an
/// entry exists even if the window dies mid-flight.
#[tauri::command]
async fn send_request(
    app: AppHandle,
    state: State<'_, Arc<HttpState>>,
    log: State<'_, HistoryStore>,
    paths: State<'_, Paths>,
    auth_state: State<'_, Arc<AuthState>>,
    spec: RequestSpec,
) -> Result<ResponseData, HttpError> {
    let section = spec
        .section_id
        .as_deref()
        .and_then(|id| store::load_one(&paths.sections, id).ok().flatten());

    let at = history::now_millis();
    let url = spec.url.clone();
    let outcome = send_authenticated(
        state.inner().as_ref(),
        auth_state.inner().as_ref(),
        section.as_ref(),
        spec.clone(),
        &secrets::get,
        Some(&app),
    )
    .await;

    // A history failure must not fail the request the user actually made.
    if let Err(err) = log.record(&spec, at, &url, &outcome) {
        ::log::warn!("could not record history: {err}");
    }

    outcome
}

/// Applies the section's auth, and — the whole point of auth-as-a-request —
/// treats a 401 as "the cached token aged out": drop it, log in again, retry
/// once. Exactly once, so a genuinely unauthorised request can't loop.
///
/// `lookup` resolves a keychain reference. Injected so the retry logic can be
/// tested without touching the real keychain.
pub(crate) async fn send_authenticated<F>(
    http_state: &HttpState,
    auth_state: &AuthState,
    section: Option<&Section>,
    spec: RequestSpec,
    lookup: &F,
    app: Option<&AppHandle>,
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

    ::log::info!("401 from {}, re-authenticating and retrying once", spec.url);
    auth_state.invalidate(&section.id);

    // A browser-captured credential can't be re-fetched by replaying a request,
    // so try to lift a fresh one out of a hidden webview instead. If the
    // identity provider's own session is still alive this is invisible.
    if let (AuthConfig::Browser { .. }, Some(app)) = (&section.auth, app) {
        match browser::silent_recapture(app, section).await {
            Ok(value) => {
                if let Some(reference) = section.auth.secret_ref() {
                    if let Err(err) = secrets::set(reference, &value) {
                        ::log::warn!("could not store re-captured credential: {err}");
                    }
                }
            }
            // The window is now visible for the user to sign in; the original
            // 401 is the honest answer for this request.
            Err(err) => {
                ::log::info!("silent re-capture failed: {err}");
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
    let secret = section.auth.secret_ref().and_then(lookup);
    let header = auth::header_for(auth_state, http_state, section, secret)
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

#[tauri::command]
fn set_secret(reference: String, value: String) -> Result<(), SecretError> {
    secrets::set(&reference, &value)
}

/// The UI can ask whether a secret exists; it can never read one back.
#[tauri::command]
fn has_secret(reference: String) -> bool {
    secrets::has(&reference)
}

#[tauri::command]
fn delete_secret(reference: String) -> Result<(), SecretError> {
    secrets::delete(&reference)
}

/// Forces the next send for this section to log in again.
#[tauri::command]
fn forget_token(auth_state: State<'_, Arc<AuthState>>, section_id: String) {
    auth_state.invalidate(&section_id);
}

fn section_by_id(paths: &Paths, id: &str) -> Result<Section, BrowserError> {
    store::load_one(&paths.sections, id)
        .ok()
        .flatten()
        .ok_or(BrowserError::NotConfigured)
}

/// Opens a real browser window at the section's login page. The user signs in
/// there exactly as they normally would — verification codes and all.
#[tauri::command]
fn browser_sign_in(
    app: AppHandle,
    paths: State<'_, Paths>,
    section_id: String,
) -> Result<(), BrowserError> {
    let section = section_by_id(&paths, &section_id)?;
    browser::open(&app, &section, true).map(|_| ())
}

/// Everything the signed-in session holds, for the user to pick their
/// credential out of. Cookies include HttpOnly ones the page can't read.
#[tauri::command]
async fn browser_snapshot(
    app: AppHandle,
    paths: State<'_, Paths>,
    section_id: String,
) -> Result<Snapshot, BrowserError> {
    let section = section_by_id(&paths, &section_id)?;
    browser::snapshot(&app, &section).await
}

/// Applies the section's saved capture rule and stores what it finds.
#[tauri::command]
async fn browser_capture(
    app: AppHandle,
    paths: State<'_, Paths>,
    auth_state: State<'_, Arc<AuthState>>,
    section_id: String,
) -> Result<(), BrowserError> {
    let section = section_by_id(&paths, &section_id)?;
    let found = browser::snapshot(&app, &section).await?;
    let value = browser::extract(&found, &section).ok_or(BrowserError::NothingCaptured)?;

    if let Some(reference) = section.auth.secret_ref() {
        secrets::set(reference, &value).map_err(|err| BrowserError::Eval(err.to_string()))?;
    }
    auth_state.invalidate(&section_id);
    browser::close(&app, &section_id);
    Ok(())
}

#[tauri::command]
fn browser_close(app: AppHandle, section_id: String) {
    browser::close(&app, &section_id);
}

/// Builds the fetcher a loader uses. Requests go through exactly the same path
/// as one you'd send by hand — same base URL, same auth, same 401 refresh —
/// because a discovery endpoint is usually behind the same login as everything
/// it describes.
fn loader_fetcher(
    app: &AppHandle,
    http_state: &Arc<HttpState>,
    auth_state: &Arc<AuthState>,
    section: &Section,
) -> loader::Fetcher {
    let http = http_state.clone();
    let auth = auth_state.clone();
    let app = app.clone();
    let section = section.clone();

    Arc::new(move |request: loader::LoaderRequest| {
        let http = http.clone();
        let auth = auth.clone();
        let app = app.clone();
        let section = section.clone();

        Box::pin(async move {
            let spec = RequestSpec {
                id: format!("loader:{}", section.id),
                request_id: format!("loader:{}", section.id),
                section_id: Some(section.id.clone()),
                method: request.method,
                url: store::join_url(&section.base_url, &request.url),
                headers: vec![http::Header {
                    name: "Accept".into(),
                    value: "application/json".into(),
                }],
                body: None,
                timeout_ms: Some(30_000),
                follow_redirects: true,
                accept_invalid_certs: false,
            };

            let response =
                send_authenticated(&http, &auth, Some(&section), spec, &secrets::get, Some(&app))
                    .await
                    .map_err(|err| err.to_string())?;

            Ok(loader::LoaderResponse {
                status: response.status,
                body: response.body,
            })
        })
    })
}

fn section_for_loader(paths: &Paths, id: &str) -> Result<Section, LoaderError> {
    store::load_one(&paths.sections, id)
        .ok()
        .flatten()
        .ok_or(LoaderError::NoUrl)
}

/// Runs a section's loader and caches what it reported.
#[tauri::command]
async fn run_loader(
    app: AppHandle,
    http_state: State<'_, Arc<HttpState>>,
    auth_state: State<'_, Arc<AuthState>>,
    paths: State<'_, Paths>,
    section_id: String,
) -> Result<LoaderRun, LoaderError> {
    let section = section_for_loader(&paths, &section_id)?;
    let config = section.loader.clone().ok_or(LoaderError::NoUrl)?;
    let fetcher = loader_fetcher(&app, http_state.inner(), auth_state.inner(), &section);

    let (endpoints, pages) = loader::run(&config, fetcher).await?;

    let previous = loader::read_cache(&paths.loaders, &section_id).unwrap_or_default();
    let (added, removed) = loader::diff(&previous.endpoints, &endpoints);
    let loaded_at = history::now_millis();

    if let Err(err) = loader::write_cache(
        &paths.loaders,
        &section_id,
        &loader::LoaderCache {
            loaded_at,
            endpoints: endpoints.clone(),
        },
    ) {
        ::log::warn!("could not cache loader output: {err}");
    }

    Ok(LoaderRun {
        endpoints,
        added,
        removed,
        loaded_at,
        pages,
    })
}

/// Fetches the manifest and returns it untouched, for the filter editor to
/// preview against. Separate from running the filter so typing re-maps the
/// document that's already in hand instead of hammering the API.
#[tauri::command]
async fn loader_probe(
    app: AppHandle,
    http_state: State<'_, Arc<HttpState>>,
    auth_state: State<'_, Arc<AuthState>>,
    paths: State<'_, Paths>,
    section_id: String,
) -> Result<serde_json::Value, LoaderError> {
    let section = section_for_loader(&paths, &section_id)?;
    let config = section.loader.clone().ok_or(LoaderError::NoUrl)?;
    if config.url.trim().is_empty() {
        return Err(LoaderError::NoUrl);
    }

    let fetcher = loader_fetcher(&app, http_state.inner(), auth_state.inner(), &section);
    let method = match config.method.trim() {
        "" => "GET".to_string(),
        method => method.to_string(),
    };

    let response = fetcher(loader::LoaderRequest {
        url: config.url.trim().to_string(),
        method,
    })
    .await
    .map_err(LoaderError::Fetch)?;

    if !(200..300).contains(&response.status) {
        return Err(LoaderError::Status(response.status));
    }
    serde_json::from_str(&response.body).map_err(|err| LoaderError::NotJson(err.to_string()))
}

/// Applies a filter to a document already in hand. Pure — no network, no
/// section, no state — which is what lets the editor re-run it on every
/// keystroke, and what makes it safe to hand to an agent.
#[tauri::command]
fn loader_preview(
    document: serde_json::Value,
    query: String,
) -> Result<Vec<loader::LoadedEndpoint>, LoaderError> {
    loader::to_endpoints(&loader::apply(&query, &document)?)
}

/// The last successful run, read from disk. Lets the UI show endpoints
/// immediately — and while offline — without waiting on the network.
#[tauri::command]
fn loader_cache(paths: State<'_, Paths>, section_id: String) -> loader::LoaderCache {
    loader::read_cache(&paths.loaders, &section_id).unwrap_or_default()
}

/// Parses an OpenAPI or Swagger document. Pure — the frontend decides what to
/// do with the result, and nothing is written until the user confirms.
#[tauri::command]
fn parse_openapi(text: String) -> Result<openapi::Import, openapi::ImportError> {
    openapi::parse(&text)
}

#[tauri::command]
fn default_loader() -> loader::LoaderConfig {
    loader::LoaderConfig::default()
}

/// Worked filters for the manifest shapes people actually hit.
#[tauri::command]
fn loader_examples() -> Vec<(String, String)> {
    loader::EXAMPLES
        .iter()
        .map(|(name, query)| (name.to_string(), query.to_string()))
        .collect()
}

#[tauri::command]
fn history_list(log: State<'_, HistoryStore>) -> Result<Vec<HistoryRecord>, HistoryError> {
    let records = log.list(HISTORY_PAGE)?;
    ::log::info!("loaded {} history entr(ies)", records.len());
    Ok(records)
}

/// Bodies are fetched per entry so listing history stays cheap.
#[tauri::command]
fn history_body(log: State<'_, HistoryStore>, id: String) -> Result<Option<String>, HistoryError> {
    log.body(&id)
}

#[tauri::command]
fn history_delete(log: State<'_, HistoryStore>, id: String) -> Result<(), HistoryError> {
    log.delete(&id)
}

#[tauri::command]
fn history_clear_request(
    log: State<'_, HistoryStore>,
    request_id: String,
) -> Result<(), HistoryError> {
    log.clear_request(&request_id)
}

#[tauri::command]
fn history_clear_all(log: State<'_, HistoryStore>) -> Result<(), HistoryError> {
    log.clear_all()
}

#[tauri::command]
fn cancel_request(state: State<'_, Arc<HttpState>>, id: String) -> bool {
    state.cancel(&id)
}

#[tauri::command]
fn list_sections(paths: State<'_, Paths>) -> Result<Vec<Section>, StoreError> {
    let sections = store::load_all(&paths.sections)?;
    log::info!(
        "loaded {} section(s) from {}",
        sections.len(),
        paths.sections.display()
    );
    Ok(sections)
}

#[tauri::command]
fn save_section(paths: State<'_, Paths>, section: Section) -> Result<(), StoreError> {
    store::save(&paths.sections, &section)
}

#[tauri::command]
fn delete_section(paths: State<'_, Paths>, id: String) -> Result<(), StoreError> {
    // The loader cache is derived data; it has no business outliving its section.
    loader::forget_cache(&paths.loaders, &id);
    store::delete(&paths.sections, &id)
}

/// The UI previews and sends the string this returns, so what you see is
/// exactly what goes out.
#[tauri::command]
fn resolve_url(base: String, path: String) -> String {
    store::join_url(&base, &path)
}

/// Where the collection files are, for the "reveal in Finder" affordance.
#[tauri::command]
fn sections_path(paths: State<'_, Paths>) -> String {
    paths.sections.display().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(HttpState::default()))
        .manage(Arc::new(AuthState::default()))
        .invoke_handler(tauri::generate_handler![
            send_request,
            cancel_request,
            list_sections,
            save_section,
            delete_section,
            resolve_url,
            sections_path,
            history_list,
            history_body,
            history_delete,
            history_clear_request,
            history_clear_all,
            set_secret,
            has_secret,
            delete_secret,
            forget_token,
            browser_sign_in,
            browser_snapshot,
            browser_capture,
            browser_close,
            run_loader,
            loader_probe,
            loader_preview,
            loader_cache,
            default_loader,
            parse_openapi,
            loader_examples
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_data_dir = app.path().app_data_dir()?;
            // The app used to be called Fetch. Carry its collections and
            // credentials across before anything reads them.
            if migrate::data_dir(&app_data_dir) {
                let sections = store::sections_dir(&app_data_dir);
                migrate::secrets(migrate::references(&sections).into_iter());
            }

            app.manage(Paths {
                sections: store::sections_dir(&app_data_dir),
                loaders: loader::loaders_dir(&app_data_dir),
            });
            app.manage(HistoryStore::open(&app_data_dir)?);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use auth::AuthConfig;
    use http::Header;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        let primed = apply_auth(
            &http_state,
            &auth_state,
            &section,
            spec_for(&base),
            &secret,
        )
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

        let response = send_authenticated(&http_state, &auth_state, None, spec_for(&base), &secret, None)
            .await
            .unwrap();

        assert_eq!(response.status, 401);
        assert_eq!(calls.logins.load(Ordering::SeqCst), 0, "never logged in");
    }
}
