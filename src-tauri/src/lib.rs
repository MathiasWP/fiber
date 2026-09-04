// The GUI build (and the tests) exercise the whole store/history/openapi API;
// the headless MCP binary uses a subset, so the rest is "unused" only in that
// configuration. Silence it there rather than scattering cfg attributes across
// modules that are entirely live under `gui`.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

mod auth;
#[cfg(feature = "gui")]
mod browser;
#[cfg(feature = "gui")]
mod clients;
mod history;
mod http;
mod loader;
pub mod mcp;
mod openapi;
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
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use crate::auth::AuthState;
    use crate::browser::{BrowserError, BrowserRecapture, Snapshot};
    use crate::clients::{self, ClientError, Status};
    use crate::history::{self, HistoryError, HistoryRecord, HistoryStore};
    use crate::http::{BodyEvent, ChunkSink, HttpError, HttpState, RequestSpec, ResponseData};
    use crate::loader::{self, LoaderError, LoaderRun};
    use crate::mcp;
    use crate::openapi;
    use crate::secrets::{self, SecretError};
    use crate::send::{send_authenticated, send_authenticated_streaming};
    use crate::store::{self, Section, StoreError};
    use tauri::ipc::Channel;
    use tauri::{AppHandle, Emitter, Manager, State};

    /// How many history entries the UI gets on startup.
    const HISTORY_PAGE: usize = 500;
    /// Response bodies larger than this have already been streamed to the
    /// window; repeating them on the command result doubles the IPC cost of a
    /// large reply. Smaller bodies still travel with the result so settle has
    /// an authoritative copy even if a chunk was dropped.
    const STREAMED_BODY_IPC: usize = 64 * 1024;

    /// Resolved once at startup so every command agrees on where collections live.
    struct Paths {
        sections: PathBuf,
        loaders: PathBuf,
        /// The root, not just the two directories under it: the credential
        /// file a containerised MCP server reads lives here too. See
        /// mcp::sync_secrets_file.
        data: PathBuf,
    }

    /// Parsed loader caches, keyed by section id.
    ///
    /// `loader_schema` used to deserialize the whole on-disk file — every
    /// expanded schema included — on each endpoint click. The first read (or
    /// the last successful run) lives here so later clicks are a map lookup.
    struct LoaderMem {
        inner: Mutex<HashMap<String, loader::LoaderCache>>,
    }

    impl LoaderMem {
        fn new() -> Self {
            Self {
                inner: Mutex::new(HashMap::new()),
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, loader::LoaderCache>> {
            self.inner.lock().unwrap_or_else(|err| err.into_inner())
        }

        fn fill<'a>(
            map: &'a mut HashMap<String, loader::LoaderCache>,
            dir: &Path,
            section_id: &str,
        ) -> &'a loader::LoaderCache {
            if !map.contains_key(section_id) {
                map.insert(
                    section_id.to_string(),
                    loader::read_cache(dir, section_id).unwrap_or_default(),
                );
            }
            map.get(section_id).expect("just inserted")
        }

        fn view(&self, dir: &Path, section_id: &str) -> LoaderCacheView {
            let mut map = self.lock();
            let cache = Self::fill(&mut map, dir, section_id);
            LoaderCacheView {
                loaded_at: cache.loaded_at,
                endpoints: cache.endpoints.clone(),
            }
        }

        fn schema(&self, dir: &Path, section_id: &str, endpoint_id: &str) -> EndpointSchemas {
            let mut map = self.lock();
            let cache = Self::fill(&mut map, dir, section_id);
            EndpointSchemas {
                request: cache.schemas.get(endpoint_id).cloned(),
                response: cache.response_schemas.get(endpoint_id).cloned(),
            }
        }

        fn endpoints(&self, dir: &Path, section_id: &str) -> Vec<loader::LoadedEndpoint> {
            let mut map = self.lock();
            Self::fill(&mut map, dir, section_id).endpoints.clone()
        }

        fn remember(&self, section_id: &str, cache: loader::LoaderCache) {
            self.lock().insert(section_id.to_string(), cache);
        }

        fn forget(&self, section_id: &str) {
            self.lock().remove(section_id);
        }
    }

    /// Parsed collections, keyed by id.
    ///
    /// Every send used to deserialize the whole section file — every saved body
    /// included — just to read auth and HTTP settings. The list at startup, and
    /// each save after that, live here so later sends are a map lookup.
    struct SectionMem {
        inner: Mutex<HashMap<String, Arc<store::Section>>>,
    }

    impl SectionMem {
        fn new() -> Self {
            Self {
                inner: Mutex::new(HashMap::new()),
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<store::Section>>> {
            self.inner.lock().unwrap_or_else(|err| err.into_inner())
        }

        fn replace(&self, sections: &[store::Section]) {
            let mut map = self.lock();
            map.clear();
            for section in sections {
                map.insert(section.id.clone(), Arc::new(section.clone()));
            }
        }

        fn remember(&self, section: store::Section) {
            self.lock().insert(section.id.clone(), Arc::new(section));
        }

        fn forget(&self, id: &str) {
            self.lock().remove(id);
        }

        fn get_or_load(
            &self,
            dir: &Path,
            id: &str,
        ) -> Result<Option<Arc<store::Section>>, store::StoreError> {
            {
                let map = self.lock();
                if let Some(section) = map.get(id) {
                    return Ok(Some(section.clone()));
                }
            }
            let loaded = store::load_one(dir, id)?;
            Ok(loaded.map(|section| {
                let held = Arc::new(section);
                self.lock().insert(id.to_string(), held.clone());
                held
            }))
        }
    }

    /// The cache data the sidebar needs at startup. Schemas stay on disk until
    /// their endpoint is selected, rather than making every large collection's
    /// initial IPC payload carry hundreds of duplicated OpenAPI components.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct LoaderCacheView {
        loaded_at: i64,
        endpoints: Vec<loader::LoadedEndpoint>,
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EndpointSchemas {
        request: Option<serde_json::Value>,
        response: Option<serde_json::Value>,
    }

    /// Sends, and records the outcome. History is written here rather than by the
    /// frontend so the body never has to travel back down the IPC bridge, and so
    /// an entry exists even if the window dies mid-flight.
    #[expect(
        clippy::too_many_arguments,
        reason = "Tauri injects each command state dependency as its own argument"
    )]
    #[tauri::command]
    async fn send_request(
        app: AppHandle,
        state: State<'_, Arc<HttpState>>,
        log: State<'_, HistoryStore>,
        paths: State<'_, Paths>,
        mem: State<'_, SectionMem>,
        auth_state: State<'_, Arc<AuthState>>,
        spec: RequestSpec,
        on_body: Channel<BodyEvent>,
    ) -> Result<ResponseData, HttpError> {
        let mut spec = spec;
        // A section file that exists but no longer parses fails the send. The
        // old `.ok().flatten()` here treated corrupt as absent — and sent the
        // request anyway, with the section's auth silently missing.
        let section = match spec.section_id.as_deref() {
            Some(id) => mem
                .get_or_load(&paths.sections, id)
                .map_err(|err| HttpError::Section(err.to_string()))?,
            None => None,
        };
        spec.sensitive_header = section
            .as_deref()
            .and_then(|section| section.auth.header_name())
            .map(str::to_owned);

        let at = history::now_millis();
        let url = spec.url.clone();
        let recapture = BrowserRecapture::new(app.clone(), paths.data.clone());

        // The channel is the only reason the body is on the bridge at all
        // mid-flight. A send that fails means the window has gone; the request
        // itself carries on, because history is written here and is worth
        // finishing either way.
        let sink: ChunkSink = Arc::new(move |event| {
            let _ = on_body.send(event);
        });

        let mut outcome = send_authenticated_streaming(
            state.inner().as_ref(),
            auth_state.inner().as_ref(),
            section.as_deref(),
            spec.clone(),
            &secrets::get,
            Some(&recapture),
            Some(&sink),
        )
        .await;

        // A history failure must not fail the request the user actually made.
        if let Err(err) = log.record(&spec, at, &url, &outcome) {
            ::log::warn!("could not record history: {err}");
        }

        // History has the full body. The window already received it in chunks
        // when it is large enough to be worth not sending again.
        if let Ok(response) = &mut outcome {
            if response.body_streamed && response.body.len() > STREAMED_BODY_IPC {
                response.body.clear();
            }
        }

        outcome
    }

    // These three are `async` for one reason: a synchronous Tauri command runs on
    // the main thread, and every one of them talks to the keychain. A keychain
    // call can block — on an ad-hoc signed build it can raise an authorization
    // dialog, and it will wait behind one that is already up — and a blocking
    // call on the thread that pumps the event loop is a frozen window. Nothing
    // here needs to await; `async` is what moves the work off that thread.
    #[tauri::command]
    async fn set_secret(
        paths: State<'_, Paths>,
        reference: String,
        value: String,
    ) -> Result<(), SecretError> {
        secrets::set(&reference, &value)?;
        // The keychain is the record; this only mirrors the change into the
        // file a containerised server reads, and only if one has been set up.
        mcp::sync_secrets_file(&paths.data, &reference, Some(&value));
        Ok(())
    }

    /// The UI can ask whether a secret exists; it can never read one back.
    #[tauri::command]
    async fn has_secret(reference: String) -> bool {
        secrets::has(&reference)
    }

    #[tauri::command]
    async fn delete_secret(paths: State<'_, Paths>, reference: String) -> Result<(), SecretError> {
        secrets::delete(&reference)?;
        mcp::sync_secrets_file(&paths.data, &reference, None);
        Ok(())
    }

    /// Forces the next send for this section to log in again.
    #[tauri::command]
    fn forget_token(auth_state: State<'_, Arc<AuthState>>, section_id: String) {
        auth_state.invalidate(&section_id);
    }

    fn section_by_id(
        paths: &Paths,
        mem: &SectionMem,
        id: &str,
    ) -> Result<Arc<Section>, BrowserError> {
        mem.get_or_load(&paths.sections, id)
            .map_err(|err| BrowserError::Section(err.to_string()))?
            .ok_or(BrowserError::NotConfigured)
    }

    /// Opens a real browser window at the section's login page. The user signs in
    /// there exactly as they normally would — verification codes and all.
    #[tauri::command]
    fn browser_sign_in(
        app: AppHandle,
        paths: State<'_, Paths>,
        mem: State<'_, SectionMem>,
        section_id: String,
    ) -> Result<(), BrowserError> {
        let section = section_by_id(&paths, &mem, &section_id)?;
        crate::browser::open(&app, &section, true).map(|_| ())
    }

    /// Everything the signed-in session holds, for the user to pick their
    /// credential out of. Cookies include HttpOnly ones the page can't read.
    #[tauri::command]
    async fn browser_snapshot(
        app: AppHandle,
        paths: State<'_, Paths>,
        mem: State<'_, SectionMem>,
        section_id: String,
    ) -> Result<Snapshot, BrowserError> {
        let section = section_by_id(&paths, &mem, &section_id)?;
        crate::browser::snapshot(&app, &section).await
    }

    /// Applies the section's saved capture rule and stores what it finds.
    #[tauri::command]
    async fn browser_capture(
        app: AppHandle,
        paths: State<'_, Paths>,
        mem: State<'_, SectionMem>,
        auth_state: State<'_, Arc<AuthState>>,
        section_id: String,
    ) -> Result<(), BrowserError> {
        let section = section_by_id(&paths, &mem, &section_id)?;
        let found = crate::browser::snapshot(&app, &section).await?;
        let value =
            crate::browser::extract(&found, &section).ok_or(BrowserError::NothingCaptured)?;

        if let Some(reference) = section.auth.secret_ref() {
            secrets::set(reference, &value).map_err(|err| BrowserError::Eval(err.to_string()))?;
            // Signing in again is exactly the case a container used to miss:
            // the keychain got the new credential and the running server went
            // on presenting the expired one.
            mcp::sync_secrets_file(&paths.data, reference, Some(&value));
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
        data: &Path,
        section: &Section,
    ) -> loader::Fetcher {
        let http = http_state.clone();
        let auth = auth_state.clone();
        let app = app.clone();
        let data = data.to_path_buf();
        let section = section.clone();

        Arc::new(move |request: loader::LoaderRequest| {
            let http = http.clone();
            let auth = auth.clone();
            let app = app.clone();
            let data = data.clone();
            let section = section.clone();

            Box::pin(async move {
                let url = store::join_url_scoped(&section.base_url, &request.url)
                    .map_err(|err| format!("loader URL rejected: {err}"))?;
                let requested_url = url.clone();
                let spec = RequestSpec {
                    id: loader::request_id(&section.id),
                    request_id: format!("loader:{}", section.id),
                    section_id: Some(section.id.clone()),
                    method: request.method,
                    url,
                    headers: vec![crate::http::Header {
                        name: "Accept".into(),
                        value: "application/json".into(),
                    }],
                    body: None,
                    timeout_ms: Some(30_000),
                    follow_redirects: true,
                    accept_invalid_certs: false,
                    sensitive_header: section.auth.header_name().map(str::to_owned),
                    ..Default::default()
                };

                let recapture = BrowserRecapture::new(app, data);
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
                    requested_url,
                    final_url: response.final_url,
                })
            })
        })
    }

    fn section_for_loader(
        paths: &Paths,
        sections: &SectionMem,
        id: &str,
    ) -> Result<Arc<Section>, LoaderError> {
        sections
            .get_or_load(&paths.sections, id)
            .map_err(|err| LoaderError::Section(err.to_string()))?
            .ok_or(LoaderError::NoUrl)
    }

    /// Runs a section's loader and caches what it reported.
    #[tauri::command]
    async fn run_loader(
        app: AppHandle,
        http_state: State<'_, Arc<HttpState>>,
        auth_state: State<'_, Arc<AuthState>>,
        paths: State<'_, Paths>,
        mem: State<'_, LoaderMem>,
        sections: State<'_, SectionMem>,
        section_id: String,
    ) -> Result<LoaderRun, LoaderError> {
        let section = section_for_loader(&paths, &sections, &section_id)?;
        let config = section.loader.clone().ok_or(LoaderError::NoUrl)?;
        let fetcher = loader_fetcher(
            &app,
            http_state.inner(),
            auth_state.inner(),
            &paths.data,
            &section,
        );

        let (endpoints, schemas, response_schemas, pages) = loader::run(&config, fetcher).await?;

        let previous = mem.endpoints(&paths.loaders, &section_id);
        let (added, removed) = loader::diff(&previous, &endpoints);
        let loaded_at = history::now_millis();

        let cache = loader::LoaderCache {
            loaded_at,
            endpoints: endpoints.clone(),
            schemas,
            response_schemas,
        };
        if let Err(err) = loader::write_cache(&paths.loaders, &section_id, &cache) {
            ::log::warn!("could not cache loader output: {err}");
        }
        mem.remember(&section_id, cache);

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
        sections: State<'_, SectionMem>,
        section_id: String,
    ) -> Result<serde_json::Value, LoaderError> {
        let section = section_for_loader(&paths, &sections, &section_id)?;
        let config = section.loader.clone().ok_or(LoaderError::NoUrl)?;
        if config.url.trim().is_empty() {
            return Err(LoaderError::NoUrl);
        }

        let fetcher = loader_fetcher(
            &app,
            http_state.inner(),
            auth_state.inner(),
            &paths.data,
            &section,
        );
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
            return Err(loader::rejected(&response));
        }
        serde_json::from_str(&response.body).map_err(|err| LoaderError::NotJson(err.to_string()))
    }

    /// Applies a filter to a document already in hand. Pure — no network, no
    /// section, no state — which is what lets the editor re-run it on every
    /// keystroke, and what makes it safe to hand to an agent.
    #[tauri::command]
    async fn loader_preview(
        document: serde_json::Value,
        query: String,
    ) -> Result<Vec<loader::LoadedEndpoint>, LoaderError> {
        loader::to_endpoints(&loader::apply(&query, &document)?)
    }

    /// The last successful run, read from disk. Lets the UI show endpoints
    /// immediately — and while offline — without waiting on the network.
    ///
    /// `async`, like every command below that touches the disk or real work:
    /// a synchronous command runs on the thread that pumps the event loop (see
    /// the note on `set_secret`), and a slow read there is a frozen window.
    /// The `Result` is Tauri's price for `async` + borrowed `State`.
    #[tauri::command]
    async fn loader_cache(
        paths: State<'_, Paths>,
        mem: State<'_, LoaderMem>,
        section_id: String,
    ) -> Result<LoaderCacheView, LoaderError> {
        Ok(mem.view(&paths.loaders, &section_id))
    }

    /// Retrieves the request and response schemas for one endpoint, only when
    /// it is opened — a large OpenAPI document repeats the same components
    /// hundreds of times, and the editor only needs the open one.
    #[tauri::command]
    async fn loader_schema(
        paths: State<'_, Paths>,
        mem: State<'_, LoaderMem>,
        section_id: String,
        endpoint_id: String,
    ) -> Result<EndpointSchemas, LoaderError> {
        Ok(mem.schema(&paths.loaders, &section_id, &endpoint_id))
    }

    /// Native file picker. The path is stored on the request and read at send
    /// time, so large files never cross the IPC bridge.
    #[tauri::command]
    async fn parse_openapi(text: String) -> Result<openapi::Import, openapi::ImportError> {
        openapi::parse(&text)
    }

    #[tauri::command]
    fn default_loader() -> loader::LoaderConfig {
        loader::LoaderConfig::default()
    }

    /// Worked filters for the manifest shapes people actually hit.
    #[tauri::command]
    fn loader_templates() -> Vec<(String, String)> {
        loader::TEMPLATES
            .iter()
            .map(|(name, query)| (name.to_string(), query.to_string()))
            .collect()
    }

    #[tauri::command]
    async fn history_list(
        log: State<'_, HistoryStore>,
    ) -> Result<Vec<HistoryRecord>, HistoryError> {
        let records = log.list(HISTORY_PAGE)?;
        ::log::info!("loaded {} history entr(ies)", records.len());
        Ok(records)
    }

    /// Bodies are fetched per entry so listing history stays cheap. A single
    /// body can still be 32MB from a spill file, which is why this must not
    /// run on the event-loop thread.
    #[tauri::command]
    async fn history_body(
        log: State<'_, HistoryStore>,
        id: String,
    ) -> Result<Option<String>, HistoryError> {
        log.body(&id)
    }

    #[tauri::command]
    async fn history_delete(log: State<'_, HistoryStore>, id: String) -> Result<(), HistoryError> {
        log.delete(&id)
    }

    #[tauri::command]
    async fn history_clear_request(
        log: State<'_, HistoryStore>,
        request_id: String,
        section_id: Option<String>,
    ) -> Result<(), HistoryError> {
        log.clear_request(&request_id, section_id.as_deref())
    }

    #[tauri::command]
    async fn history_clear_all(log: State<'_, HistoryStore>) -> Result<(), HistoryError> {
        log.clear_all()
    }

    #[tauri::command]
    fn cancel_request(state: State<'_, Arc<HttpState>>, id: String) -> bool {
        state.cancel(&id)
    }

    /// Sections, plus the files that failed to load. The UI shows the failures
    /// by name — a corrupt file that only reached the log read exactly like
    /// the section having been deleted.
    #[tauri::command]
    async fn list_sections(
        paths: State<'_, Paths>,
        sections: State<'_, SectionMem>,
    ) -> Result<store::SectionLoad, StoreError> {
        let load = store::load_all_reporting(&paths.sections)?;
        sections.replace(&load.sections);
        ::log::info!(
            "loaded {} section(s), {} unreadable, from {}",
            load.sections.len(),
            load.errors.len(),
            paths.sections.display()
        );
        Ok(load)
    }

    #[tauri::command]
    async fn save_section(
        paths: State<'_, Paths>,
        sections: State<'_, SectionMem>,
        section: Section,
    ) -> Result<(), StoreError> {
        store::save(&paths.sections, &section)?;
        mcp::sync_section_sharing(&paths.data, &section);
        sections.remember(section);
        Ok(())
    }

    #[tauri::command]
    async fn delete_section(
        paths: State<'_, Paths>,
        mem: State<'_, LoaderMem>,
        sections: State<'_, SectionMem>,
        id: String,
    ) -> Result<(), StoreError> {
        // The loader cache is derived data; it has no business outliving its section.
        loader::forget_cache(&paths.loaders, &id);
        // Same for the credential file a container reads: the app writes
        // `<sectionId>:auth`, so the reference is derivable without the section
        // that is about to go.
        mcp::sync_secrets_file(&paths.data, &format!("{id}:auth"), None);
        mem.forget(&id);
        sections.forget(&id);
        store::delete(&paths.sections, &id)
    }

    /// Same join as the TypeScript preview. Kept so MCP and any other caller
    /// can resolve without duplicating the slash-normalisation rules.
    #[tauri::command]
    fn resolve_url(base: String, path: String) -> String {
        store::join_url(&base, &path)
    }

    /// Where the collection files are, for the "reveal in Finder" affordance.
    /// Every AI client Fiber knows how to install itself into, and what each
    /// one's config says right now. Read on demand — the MCP tab asks when it
    /// opens, so a session that never opens it never touches these files.
    #[tauri::command]
    fn mcp_clients() -> Vec<Status> {
        clients::statuses()
    }

    /// The path a client should spawn: this binary. Shown beside the snippet
    /// for the clients Fiber cannot write to.
    #[tauri::command]
    fn mcp_binary() -> String {
        clients::binary()
    }

    #[tauri::command]
    fn mcp_install(id: String) -> Result<Status, ClientError> {
        clients::install(&id)
    }

    #[tauri::command]
    fn mcp_uninstall(id: String) -> Result<Status, ClientError> {
        clients::uninstall(&id)
    }

    #[tauri::command]
    fn sections_path(paths: State<'_, Paths>) -> String {
        paths.sections.display().to_string()
    }

    /// The frontend's reply to the `flush-before-exit` event: its debounced
    /// saves are on disk, so quitting may proceed. `app.exit` carries an exit
    /// code, which is what tells the run loop below not to intercept it again.
    #[tauri::command]
    fn flush_complete(app: AppHandle) {
        app.exit(0);
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
                loader_schema,
                default_loader,
                parse_openapi,
                loader_templates,
                mcp_clients,
                mcp_binary,
                mcp_install,
                mcp_uninstall,
                flush_complete
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
                app.manage(Paths {
                    sections: store::sections_dir(&app_data_dir),
                    loaders: loader::loaders_dir(&app_data_dir),
                    data: app_data_dir.clone(),
                });
                app.manage(LoaderMem::new());
                app.manage(SectionMem::new());
                // `open_or_recover`: a corrupt history database is moved aside
                // and replaced. Plain `open` here fed the `.expect` below —
                // one bad file meant a panic on every launch thereafter.
                app.manage(HistoryStore::open_or_recover(&app_data_dir)?);

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
            .build(tauri::generate_context!())
            .expect("error while running tauri application")
            .run(|app_handle, event| {
                // The frontend debounces its saves, so a Cmd+Q can land inside
                // the 400ms where edits exist only in the webview. `code` is
                // `None` exactly when the quit came from user interaction; our
                // own `app.exit(0)` — `flush_complete`, or the fallback below
                // — carries `Some`, and sails through this match untouched.
                if let tauri::RunEvent::ExitRequested {
                    code: None, api, ..
                } = &event
                {
                    use std::sync::atomic::{AtomicBool, Ordering};
                    static FLUSH_ASKED: AtomicBool = AtomicBool::new(false);
                    // Asked once already: either the flush is in flight and
                    // the user pressed Cmd+Q again, or something re-requested
                    // exit. Letting it through beats a quit that can't quit.
                    if FLUSH_ASKED.swap(true, Ordering::SeqCst) {
                        return;
                    }

                    api.prevent_exit();
                    let _ = app_handle.emit("flush-before-exit", ());

                    // If the frontend never answers — hung webview, listener
                    // not yet attached — quit anyway. Losing 400ms of edits is
                    // recoverable; an app that refuses to exit is not.
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        handle.exit(0);
                    });
                }
            });
    }
}
