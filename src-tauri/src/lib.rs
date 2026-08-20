// The GUI build (and the tests) exercise the whole store/history/openapi API;
// the headless MCP binary uses a subset, so the rest is "unused" only in that
// configuration. Silence it there rather than scattering cfg attributes across
// modules that are entirely live under `gui`.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

mod auth;
#[cfg(feature = "gui")]
mod browser;
mod history;
mod loader;
pub mod mcp;
mod migrate;
mod openapi;
mod http;
mod secrets;
mod send;
mod store;

// The authenticated-send core is shared by the Tauri commands and the MCP
// server, so it lives at the crate root regardless of which front end is built.
pub(crate) use send::send_authenticated;

#[cfg(feature = "gui")]
pub use gui::run;

/// The Tauri app: commands and window setup. Compiled only for the desktop
/// build — the headless `fiber mcp` binary needs none of it, and gating it here
/// is what lets that binary build without Tauri or a webview at all.
#[cfg(feature = "gui")]
mod gui {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::auth::AuthState;
    use crate::browser::{BrowserError, BrowserRecapture, Snapshot};
    use crate::history::{self, HistoryError, HistoryRecord, HistoryStore};
    use crate::http::{HttpError, HttpState, RequestSpec, ResponseData};
    use crate::loader::{self, LoaderError, LoaderRun};
    use crate::migrate;
    use crate::openapi;
    use crate::secrets::{self, SecretError};
    use crate::send::send_authenticated;
    use crate::store::{self, Section, StoreError};
    use tauri::{AppHandle, Manager, State};

    /// How many history entries the UI gets on startup.
    const HISTORY_PAGE: usize = 500;

    /// Resolved once at startup so every command agrees on where collections live.
    struct Paths {
        sections: PathBuf,
        loaders: PathBuf,
    }

    /// Sends, and records the outcome. History is written here rather than by the
    /// frontend so the body never has to travel back down the IPC bridge, and so
    /// an entry exists even if the window dies mid-flight.
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
        let recapture = BrowserRecapture::new(app.clone());
        let outcome = send_authenticated(
            state.inner().as_ref(),
            auth_state.inner().as_ref(),
            section.as_ref(),
            spec.clone(),
            &secrets::get,
            Some(&recapture),
        )
        .await;

        // A history failure must not fail the request the user actually made.
        if let Err(err) = log.record(&spec, at, &url, &outcome) {
            ::log::warn!("could not record history: {err}");
        }

        outcome
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
        crate::browser::open(&app, &section, true).map(|_| ())
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
        crate::browser::snapshot(&app, &section).await
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
        let found = crate::browser::snapshot(&app, &section).await?;
        let value =
            crate::browser::extract(&found, &section).ok_or(BrowserError::NothingCaptured)?;

        if let Some(reference) = section.auth.secret_ref() {
            secrets::set(reference, &value).map_err(|err| BrowserError::Eval(err.to_string()))?;
        }
        auth_state.invalidate(&section_id);
        crate::browser::close(&app, &section_id);
        Ok(())
    }

    #[tauri::command]
    fn browser_close(app: AppHandle, section_id: String) {
        crate::browser::close(&app, &section_id);
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
                    headers: vec![crate::http::Header {
                        name: "Accept".into(),
                        value: "application/json".into(),
                    }],
                    body: None,
                    timeout_ms: Some(30_000),
                    follow_redirects: true,
                    accept_invalid_certs: false,
                };

                let recapture = BrowserRecapture::new(app);
                let response = send_authenticated(
                    &http,
                    &auth,
                    Some(&section),
                    spec,
                    &secrets::get,
                    Some(&recapture),
                )
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
    fn history_body(
        log: State<'_, HistoryStore>,
        id: String,
    ) -> Result<Option<String>, HistoryError> {
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
        ::log::info!(
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
            .plugin(tauri_plugin_opener::init())
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init())
            // Size and position are restored on launch. Without this an update
            // restart drops the window back to its default geometry.
            .plugin(tauri_plugin_window_state::Builder::default().build())
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

                // An update relaunches the app while the old process is still
                // exiting, and macOS hands focus back to whatever was behind it
                // — leaving the new window open but buried, so it looked like
                // nothing had happened until you clicked the dock icon.
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }

                Ok(())
            })
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}
