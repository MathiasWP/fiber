//! The MCP server.
//!
//! `fiber mcp` speaks MCP over stdio using the same core as the app and the
//! same files on disk. No window, no running app — which is why every module
//! under it is free of Tauri types.
//!
//! # Why this is the cautious part
//!
//! A tool that can send arbitrary HTTP with your production credentials is a
//! confused deputy. The agent asking is not the one being authenticated. So:
//!
//! - **Sections opt in**, one at a time, and the default is off. A section the
//!   user hasn't exposed is invisible here — not merely read-only.
//! - **Writes opt in separately.** Anything but GET/HEAD/OPTIONS needs a second
//!   switch on that section.
//! - **Credentials never come back out.** Auth headers are redacted from every
//!   response, so a token can't be laundered through a tool result.
//! - **Bodies are truncated**, with a jq filter available to query the rest.
//!   Dumping 200KB of JSON into a context window helps nobody.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};

use crate::auth::AuthState;
use crate::history::HistoryStore;
// `redact` lives in `http` because history persistence needs the same list;
// two lists would drift, and the one that drifted would be the one that leaks.
use crate::http::{redact_with, BodyKind, FormField, Header, HttpState, RequestSpec};
use crate::loader;
use crate::secrets;
use crate::store::{self, Section};

/// Response bodies beyond this are cut short; `query_response` reaches the rest.
const MAX_BODY_CHARS: usize = 8_000;
/// Manifests are shown to help write a filter, not to be read in full.
const MAX_MANIFEST_CHARS: usize = 12_000;
const DEFAULT_SEARCH_LIMIT: usize = 50;
const MAX_SEARCH_LIMIT: usize = 200;
const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_REQUESTS: usize = 16;
const MAX_CONCURRENT_LOADERS: usize = 2;

/// One pass over the body: find the byte where character `limit + 1` would
/// start and cut there. Counting the characters first and *then* collecting
/// walked a possibly-32MB body twice to keep 8KB of it.
fn truncate(body: &str, limit: usize) -> (String, bool) {
    match body.char_indices().nth(limit) {
        Some((cut, _)) => (body[..cut].to_string(), true),
        None => (body.to_string(), false),
    }
}

/// An absolute `path` normally bypasses the section outright — `join_url`'s
/// escape hatch for a person with one endpoint that lives elsewhere. Here the
/// path comes from an agent and the section's credential rides along with the
/// request, so an absolute URL that leaves the section's scheme+host+port
/// would hand the token to whatever host the prompt named — an attacker's
/// server, or a cloud metadata endpoint. Erroring, rather than quietly
/// stripping the auth, is what the agent can actually act on.
fn require_same_origin(base_url: &str, path: &str) -> Result<(), McpError> {
    store::join_url_scoped(base_url, path)
        .map(|_| ())
        .map_err(|err| {
            McpError::invalid_params(format!("{err}. Use a relative path instead."), None)
        })
}

fn is_read_only(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointSummary {
    section_id: String,
    section: String,
    key: String,
    method: String,
    path: String,
    name: String,
    description: String,
    tag: String,
    parameters: Vec<crate::openapi::SpecParam>,
    /// True when a loader reported this rather than a person writing it.
    loaded: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SectionArgs {
    /// The section's id, as returned by `list_sections`.
    section_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// Matched against endpoint name, method, path, tag and description. Empty lists everything.
    #[serde(default)]
    query: String,
    /// Optional collection id to narrow the search.
    #[serde(default)]
    section_id: Option<String>,
    /// Optional HTTP method to narrow the search.
    #[serde(default)]
    method: Option<String>,
    /// Zero-based result offset.
    #[serde(default)]
    offset: usize,
    /// Results to return (default 50, maximum 200).
    #[serde(default = "default_search_limit")]
    limit: usize,
}

fn default_search_limit() -> usize {
    DEFAULT_SEARCH_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendArgs {
    section_id: String,
    /// GET unless the section allows writes.
    #[serde(default)]
    method: String,
    /// Relative to the section's base URL, e.g. `/user/42`.
    path: String,
    /// JSON body, for methods that take one.
    #[serde(default)]
    body: Option<String>,
    /// Additional request headers. Collection authentication still applies.
    #[serde(default)]
    headers: Vec<Header>,
    /// JSON, text, form, or multipart. File bodies are not available to agents.
    #[serde(default)]
    body_kind: BodyKind,
    /// Form fields used by form and multipart bodies. File fields are rejected.
    #[serde(default)]
    form: Vec<FormField>,
    /// Values for `{name}` placeholders in the path.
    #[serde(default)]
    path_params: Vec<Header>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryResponseArgs {
    /// The `responseId` from a previous `send_request`.
    response_id: String,
    /// jq filter applied to the stored body, e.g. `.items | length`.
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TryFilterArgs {
    section_id: String,
    /// A candidate jq filter to test against the section's manifest.
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EndpointArgs {
    section_id: String,
    /// Endpoint key returned by `search_endpoints`.
    key: String,
}

struct CachedSections {
    stamp: Vec<(String, u128, u64)>,
    sections: Arc<Vec<Arc<Section>>>,
    warnings: Arc<Vec<String>>,
}

struct CachedManifest {
    key: String,
    fetched_at: Instant,
    document: Arc<serde_json::Value>,
}

struct CachedLoader {
    stamp: Option<(u128, u64)>,
    cache: Arc<loader::LoaderCache>,
}

type SectionSnapshot = (Arc<Vec<Arc<Section>>>, Arc<Vec<String>>);

fn file_stamp(path: &std::path::Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((modified, meta.len()))
}

/// mtime + size of every collection file. Cheap enough to recompute on each
/// tool call, and enough to notice a save without parsing the bodies.
fn sections_stamp(dir: &std::path::Path) -> std::io::Result<Vec<(String, u128, u64)>> {
    let mut stamp = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(stamp),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        let meta = entry.metadata()?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        stamp.push((name, mtime, meta.len()));
    }
    stamp.sort();
    Ok(stamp)
}

#[derive(Clone)]
pub struct FiberMcp {
    sections_dir: std::path::PathBuf,
    loaders_dir: std::path::PathBuf,
    http: Arc<HttpState>,
    auth: Arc<AuthState>,
    history: Arc<HistoryStore>,
    /// `exposed()` used to re-parse every collection file — bodies included —
    /// on every tool call. The stamp is the mtime/size of those files, so a
    /// save is noticed and a no-op call is a clone of what we already hold.
    sections: Arc<Mutex<Option<CachedSections>>>,
    manifests: Arc<Mutex<HashMap<String, CachedManifest>>>,
    endpoint_caches: Arc<Mutex<HashMap<String, CachedLoader>>>,
    request_ids: Arc<AtomicU64>,
    requests: Arc<tokio::sync::Semaphore>,
    loaders: Arc<tokio::sync::Semaphore>,
    #[expect(dead_code, reason = "the tool_handler macro reads this field")]
    tool_router: ToolRouter<Self>,
}

impl FiberMcp {
    fn next_response_id(&self) -> String {
        format!(
            "mcp-{}-{}-{}",
            crate::history::now_millis(),
            std::process::id(),
            self.request_ids.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn loader_cache_of(&self, section_id: &str) -> Arc<loader::LoaderCache> {
        let path = self.loaders_dir.join(format!("{section_id}.json"));
        let stamp = file_stamp(&path);
        {
            let caches = self
                .endpoint_caches
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(cached) = caches.get(section_id) {
                if cached.stamp == stamp {
                    return cached.cache.clone();
                }
            }
        }
        let cache = Arc::new(loader::read_cache(&self.loaders_dir, section_id).unwrap_or_default());
        self.endpoint_caches
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                section_id.to_string(),
                CachedLoader {
                    stamp,
                    cache: cache.clone(),
                },
            );
        cache
    }

    fn all_sections(&self) -> Result<SectionSnapshot, McpError> {
        let stamp = sections_stamp(&self.sections_dir).map_err(|err| {
            McpError::internal_error(format!("could not inspect collections: {err}"), None)
        })?;
        {
            let cache = self.sections.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(cached) = cache.as_ref() {
                if cached.stamp == stamp {
                    return Ok((cached.sections.clone(), cached.warnings.clone()));
                }
            }
        }
        let load = store::load_all_reporting(&self.sections_dir)
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let sections: Arc<Vec<Arc<Section>>> =
            Arc::new(load.sections.into_iter().map(Arc::new).collect());
        let warnings: Arc<Vec<String>> = Arc::new(
            load.errors
                .into_iter()
                .map(|error| format!("{}: {}", error.file, error.message))
                .collect(),
        );
        *self.sections.lock().unwrap_or_else(|err| err.into_inner()) = Some(CachedSections {
            stamp,
            sections: sections.clone(),
            warnings: warnings.clone(),
        });
        Ok((sections, warnings))
    }

    /// Only sections the user has explicitly exposed. Everything else is
    /// invisible, not merely read-only.
    fn exposed(&self) -> Result<Vec<Arc<Section>>, McpError> {
        Ok(self
            .all_sections()?
            .0
            .iter()
            .filter(|section| section.mcp.enabled)
            .cloned()
            .collect())
    }

    fn exposed_section(&self, id: &str) -> Result<Arc<Section>, McpError> {
        self.exposed()?
            .into_iter()
            .find(|section| section.id == id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    "No such section, or it isn't shared with MCP. Enable it in Section settings."
                        .to_string(),
                    None,
                )
            })
    }

    /// Every endpoint an exposed section has, hand-written or loaded.
    fn endpoints_of(&self, section: &Section) -> Vec<EndpointSummary> {
        let mut found: Vec<EndpointSummary> = section
            .requests
            .iter()
            .map(|request| EndpointSummary {
                section_id: section.id.clone(),
                section: section.name.clone(),
                key: request.id.clone(),
                method: request.method.clone(),
                path: request.path.clone(),
                name: request.name.clone(),
                description: request.description.clone(),
                tag: request.tag.clone(),
                parameters: Vec::new(),
                loaded: false,
            })
            .collect();

        let cache = self.loader_cache_of(&section.id);
        found.extend(cache.endpoints.iter().map(|endpoint| EndpointSummary {
            section_id: section.id.clone(),
            section: section.name.clone(),
            key: endpoint.key(),
            method: endpoint.method.clone(),
            path: endpoint.path.clone(),
            name: endpoint.name.clone(),
            description: endpoint.description.clone(),
            tag: endpoint.tag.clone(),
            parameters: endpoint.parameters.clone(),
            loaded: true,
        }));
        found
    }

    fn fetcher(&self, section: &Section) -> loader::Fetcher {
        let http = self.http.clone();
        let auth = self.auth.clone();
        let section = section.clone();

        Arc::new(move |request: loader::LoaderRequest| {
            let http = http.clone();
            let auth = auth.clone();
            let section = section.clone();

            Box::pin(async move {
                let url = store::join_url_scoped(&section.base_url, &request.url)
                    .map_err(|err| format!("loader URL rejected: {err}"))?;
                let spec = RequestSpec {
                    id: format!("mcp-loader:{}", section.id),
                    request_id: format!("loader:{}", section.id),
                    section_id: Some(section.id.clone()),
                    method: request.method,
                    url,
                    headers: vec![Header {
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

                // `None` for the app handle: there's no window here, so a
                // browser-captured credential can be used but not re-captured.
                let response = crate::send_authenticated(
                    &http,
                    &auth,
                    Some(&section),
                    spec,
                    &secrets::get,
                    None,
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

    async fn manifest_of(&self, section: &Section) -> Result<serde_json::Value, McpError> {
        let config = section.loader.clone().ok_or_else(|| {
            McpError::invalid_params("That section has no loader.".to_string(), None)
        })?;

        // Default a blank method to GET, exactly as the app's loader_probe and
        // loader::run do — the two must never disagree about the same loader.
        let method = match config.method.trim() {
            "" => "GET".to_string(),
            method => method.to_string(),
        };
        let cache_key = format!("{method}\n{}", config.url.trim());
        {
            let manifests = self.manifests.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(cached) = manifests.get(&section.id) {
                if cached.key == cache_key && cached.fetched_at.elapsed() < MANIFEST_CACHE_TTL {
                    return Ok((*cached.document).clone());
                }
            }
        }
        let _permit = self.loaders.acquire().await.map_err(|_| {
            McpError::internal_error("loader concurrency limiter closed".to_string(), None)
        })?;
        let response = (self.fetcher(section))(loader::LoaderRequest {
            url: config.url.clone(),
            method,
        })
        .await
        .map_err(|err| McpError::internal_error(err, None))?;
        if !(200..300).contains(&response.status) {
            return Err(McpError::internal_error(
                format!("manifest request returned {}", response.status),
                None,
            ));
        }

        let document: serde_json::Value = serde_json::from_str(&response.body).map_err(|err| {
            McpError::internal_error(format!("manifest wasn't JSON: {err}"), None)
        })?;
        self.manifests
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                section.id.clone(),
                CachedManifest {
                    key: cache_key,
                    fetched_at: Instant::now(),
                    document: Arc::new(document.clone()),
                },
            );
        Ok(document)
    }
}

fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let value = serde_json::to_value(value)
        .map_err(|err| McpError::internal_error(err.to_string(), None))?;
    Ok(CallToolResult::structured(value))
}

#[tool_router]
impl FiberMcp {
    /// Collections shared with MCP.
    #[tool(description = "List the API collections shared with MCP, with their base URLs.")]
    async fn list_sections(&self) -> Result<CallToolResult, McpError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Summary {
            id: String,
            name: String,
            base_url: String,
            endpoints: usize,
            allows_writes: bool,
            has_loader: bool,
        }

        let (all, warnings) = self.all_sections()?;
        let summaries: Vec<Summary> = all
            .iter()
            .filter(|section| section.mcp.enabled)
            .map(|section| Summary {
                id: section.id.clone(),
                name: section.name.clone(),
                base_url: section.base_url.clone(),
                endpoints: self.endpoints_of(section).len(),
                allows_writes: section.mcp.allow_writes,
                has_loader: section.loader.is_some(),
            })
            .collect();

        ok_json(&serde_json::json!({
            "sections": summaries,
            "warnings": warnings.as_ref(),
        }))
    }

    #[tool(
        description = "Search endpoints across shared collections by name, method, path, tag or description. Start here to find what to call."
    )]
    async fn search_endpoints(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let needle = args.query.trim().to_lowercase();
        let section_filter = args.section_id.as_deref();
        let method_filter = args
            .method
            .as_deref()
            .map(|method| method.trim().to_uppercase());
        let found: Vec<EndpointSummary> = self
            .exposed()?
            .iter()
            .filter(|section| section_filter.is_none_or(|id| section.id == id))
            .flat_map(|section| self.endpoints_of(section))
            .filter(|endpoint| {
                method_filter
                    .as_deref()
                    .is_none_or(|method| endpoint.method.eq_ignore_ascii_case(method))
                    && (needle.is_empty()
                        || [
                            &endpoint.name,
                            &endpoint.method,
                            &endpoint.path,
                            &endpoint.description,
                            &endpoint.tag,
                        ]
                        .iter()
                        .any(|field| field.to_lowercase().contains(&needle)))
            })
            .collect();

        let total = found.len();
        let limit = args.limit.clamp(1, MAX_SEARCH_LIMIT);
        let items: Vec<_> = found.into_iter().skip(args.offset).take(limit).collect();
        let next_offset = (args.offset + items.len() < total).then_some(args.offset + items.len());
        ok_json(&serde_json::json!({
            "items": items,
            "total": total,
            "offset": args.offset,
            "limit": limit,
            "nextOffset": next_offset,
            "truncated": next_offset.is_some(),
        }))
    }

    #[tool(
        description = "Get one endpoint's complete request template and schemas. Use the sectionId and key returned by search_endpoints."
    )]
    async fn get_endpoint(
        &self,
        Parameters(args): Parameters<EndpointArgs>,
    ) -> Result<CallToolResult, McpError> {
        let section = self.exposed_section(&args.section_id)?;
        if let Some(request) = section
            .requests
            .iter()
            .find(|request| request.id == args.key)
        {
            return ok_json(&serde_json::json!({
                "sectionId": section.id,
                "key": request.id,
                "loaded": false,
                "method": request.method,
                "path": request.path,
                "name": request.name,
                "description": request.description,
                "tag": request.tag,
                "headers": request.headers,
                "body": request.body,
                "bodyKind": request.body_kind,
                "form": request.form,
                "pathParams": request.path_params,
                "parameters": [],
                "requestSchema": null,
                "responseSchema": null,
            }));
        }

        let cache = self.loader_cache_of(&section.id);
        let endpoint = cache
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key() == args.key)
            .ok_or_else(|| {
                McpError::invalid_params(
                    "No endpoint with that key in this shared collection.".to_string(),
                    None,
                )
            })?;
        ok_json(&serde_json::json!({
            "sectionId": section.id,
            "key": endpoint.key(),
            "loaded": true,
            "method": endpoint.method,
            "path": endpoint.path,
            "name": endpoint.name,
            "description": endpoint.description,
            "tag": endpoint.tag,
            "headers": [],
            "body": endpoint.body,
            "bodyKind": endpoint.body_kind,
            "form": endpoint.form,
            "pathParams": [],
            "parameters": endpoint.parameters,
            "requestSchema": cache.schemas.get(&args.key),
            "responseSchema": cache.response_schemas.get(&args.key),
        }))
    }

    #[tool(
        description = "Send a request through a shared collection. The collection's base URL and credentials are applied automatically; credentials are never returned."
    )]
    async fn send_request(
        &self,
        Parameters(args): Parameters<SendArgs>,
    ) -> Result<CallToolResult, McpError> {
        let section = self.exposed_section(&args.section_id)?;
        let method = match args.method.trim() {
            "" => "GET".to_string(),
            method => method.to_uppercase(),
        };

        if !is_read_only(&method) && !section.mcp.allow_writes {
            return Err(McpError::invalid_params(
                format!(
                    "`{method}` is not allowed for this collection. Only GET, HEAD and OPTIONS are \
                     permitted unless writes are enabled for it in Section settings."
                ),
                None,
            ));
        }

        require_same_origin(&section.base_url, &args.path)?;
        if args.body_kind == BodyKind::File {
            return Err(McpError::invalid_params(
                "File bodies are not exposed to MCP because a file path could read arbitrary host data."
                    .to_string(),
                None,
            ));
        }
        if args
            .form
            .iter()
            .any(|field| field.is_file || !field.file.trim().is_empty())
        {
            return Err(McpError::invalid_params(
                "File fields are not exposed to MCP. Use text form fields only.".to_string(),
                None,
            ));
        }
        let _permit = self.requests.acquire().await.map_err(|_| {
            McpError::internal_error("request concurrency limiter closed".to_string(), None)
        })?;

        let id = self.next_response_id();
        let url = store::join_url(&section.base_url, &args.path);
        let body = args.body.filter(|body| !body.trim().is_empty());
        let mut headers = args.headers;
        if body.is_some()
            && args.body_kind == BodyKind::Json
            && !headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type"))
        {
            headers.push(Header {
                name: "Content-Type".into(),
                value: "application/json".into(),
            });
        }

        let spec = RequestSpec {
            id: id.clone(),
            request_id: format!("mcp:{}", section.id),
            section_id: Some(section.id.clone()),
            method,
            url: url.clone(),
            headers,
            body,
            body_kind: args.body_kind,
            form: args.form,
            path_params: args.path_params,
            timeout_ms: Some(60_000),
            follow_redirects: true,
            accept_invalid_certs: false,
            sensitive_header: section.auth.header_name().map(str::to_owned),
            ..Default::default()
        };

        let at = crate::history::now_millis();
        let outcome = crate::send_authenticated(
            &self.http,
            &self.auth,
            Some(&section),
            spec.clone(),
            &secrets::get,
            None,
        )
        .await;
        let history = self.history.clone();
        let record_spec = spec.clone();
        let record_url = url.clone();
        let (outcome, recorded) = tokio::task::spawn_blocking(move || {
            let recorded = history.record(&record_spec, at, &record_url, &outcome);
            (outcome, recorded)
        })
        .await
        .map_err(|err| {
            McpError::internal_error(format!("response persistence task failed: {err}"), None)
        })?;

        let response = outcome.map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let (body, context_truncated) = truncate(&response.body, MAX_BODY_CHARS);
        let query_available = recorded.is_ok();
        let mut hints = Vec::new();
        if context_truncated && query_available {
            hints.push("Body preview truncated. Use query_response with a jq filter.");
        }
        if response.truncated {
            hints.push("The HTTP capture reached its 32 MiB safety limit; bytes beyond it are unavailable.");
        }
        if let Err(err) = &recorded {
            hints.push("The response could not be persisted, so query_response is unavailable.");
            log::warn!("could not record MCP response {id}: {err}");
        }
        if response.status == 401 && matches!(section.auth, crate::auth::AuthConfig::Browser { .. })
        {
            hints.push(
                "Browser credentials cannot refresh headlessly. Sign in again in Fiber, then restart the MCP server or re-export container secrets.",
            );
        }

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply {
            #[serde(skip_serializing_if = "Option::is_none")]
            response_id: Option<String>,
            status: u16,
            status_text: String,
            final_url: String,
            headers: Vec<Header>,
            size_bytes: u64,
            body: String,
            truncated: bool,
            capture_truncated: bool,
            is_binary: bool,
            timing: crate::http::Timing,
            query_available: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            hint: Option<String>,
        }

        ok_json(&Reply {
            response_id: query_available.then_some(id),
            status: response.status,
            status_text: response.status_text,
            final_url: response.final_url,
            headers: redact_with(&response.headers, section.auth.header_name()),
            size_bytes: response.size_bytes,
            body,
            truncated: context_truncated,
            capture_truncated: response.truncated,
            is_binary: response.is_binary,
            timing: response.timing,
            query_available,
            hint: (!hints.is_empty()).then(|| hints.join(" ")),
        })
    }

    #[tool(
        description = "Query a stored response with a jq filter, for reading a large body without pulling all of it into context."
    )]
    async fn query_response(
        &self,
        Parameters(args): Parameters<QueryResponseArgs>,
    ) -> Result<CallToolResult, McpError> {
        let history = self.history.clone();
        let response_id = args.response_id.clone();
        let (section_id, body) =
            tokio::task::spawn_blocking(move || history.mcp_body(&response_id))
                .await
                .map_err(|err| {
                    McpError::internal_error(format!("response read task failed: {err}"), None)
                })?
                .map_err(|err| McpError::internal_error(err.to_string(), None))?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        "No MCP response with that id is available.".to_string(),
                        None,
                    )
                })?;
        // Sharing is live authorization, not a one-time grant. Turning a
        // collection off also revokes access to its old response bodies.
        let _section = self.exposed_section(&section_id)?;

        if args.query.trim().is_empty() {
            let (body, truncated) = truncate(&body, MAX_BODY_CHARS);
            return ok_json(&serde_json::json!({ "body": body, "truncated": truncated }));
        }

        if args.query.len() > 4_096 {
            return Err(McpError::invalid_params(
                "jq filters are limited to 4096 characters.".to_string(),
                None,
            ));
        }
        let document: serde_json::Value = serde_json::from_str(&body).map_err(|err| {
            McpError::invalid_params(format!("that response isn't JSON: {err}"), None)
        })?;
        let query = args.query;
        let _permit = self.loaders.acquire().await.map_err(|_| {
            McpError::internal_error("filter concurrency limiter closed".to_string(), None)
        })?;
        let result = tokio::task::spawn_blocking(move || loader::apply(&query, &document))
            .await
            .map_err(|err| McpError::internal_error(format!("filter task failed: {err}"), None))?
            .map_err(|err| McpError::invalid_params(err.to_string(), None))?;

        let rendered = result.to_string();
        let (preview, truncated) = truncate(&rendered, MAX_BODY_CHARS);
        if truncated {
            ok_json(&serde_json::json!({
                "resultPreview": preview,
                "truncated": true,
                "hint": "Refine the jq filter to return a smaller value.",
            }))
        } else {
            ok_json(&serde_json::json!({ "result": result, "truncated": false }))
        }
    }

    #[tool(description = "Re-run a collection's loader so its endpoint list is current.")]
    async fn refresh_endpoints(
        &self,
        Parameters(args): Parameters<SectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        let section = self.exposed_section(&args.section_id)?;
        let config = section.loader.clone().ok_or_else(|| {
            McpError::invalid_params("That section has no loader.".to_string(), None)
        })?;

        let _permit = self.loaders.acquire().await.map_err(|_| {
            McpError::internal_error("loader concurrency limiter closed".to_string(), None)
        })?;
        let (endpoints, schemas, response_schemas, pages) =
            loader::run(&config, self.fetcher(&section))
                .await
                .map_err(|err| McpError::internal_error(err.to_string(), None))?;

        let previous = self.loader_cache_of(&section.id);
        let (added, removed) = loader::diff(&previous.endpoints, &endpoints);
        let cache = loader::LoaderCache {
            loaded_at: crate::history::now_millis(),
            endpoints: endpoints.clone(),
            schemas,
            response_schemas,
        };
        loader::write_cache(&self.loaders_dir, &section.id, &cache).map_err(|err| {
            McpError::internal_error(format!("could not persist loader cache: {err}"), None)
        })?;
        self.endpoint_caches
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                section.id.clone(),
                CachedLoader {
                    stamp: file_stamp(&self.loaders_dir.join(format!("{}.json", section.id))),
                    cache: Arc::new(cache),
                },
            );
        self.manifests
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&section.id);

        ok_json(&serde_json::json!({
            "endpoints": endpoints.len(),
            "pages": pages,
            "added": added,
            "removed": removed,
        }))
    }

    #[tool(
        description = "Fetch a collection's raw endpoint manifest, to work out the jq filter its loader needs."
    )]
    async fn loader_manifest(
        &self,
        Parameters(args): Parameters<SectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        let section = self.exposed_section(&args.section_id)?;
        let document = self.manifest_of(&section).await?;
        let (rendered, truncated) = truncate(
            &serde_json::to_string_pretty(&document).unwrap_or_default(),
            MAX_MANIFEST_CHARS,
        );

        ok_json(&serde_json::json!({
            "manifest": rendered,
            "truncated": truncated,
            "currentQuery": section.loader.as_ref().map(|loader| loader.query.clone()),
            "wanted": "A jq filter returning [{method, path, name?}]",
        }))
    }

    #[tool(
        description = "Try a candidate jq filter against a collection's manifest and see the endpoints it would produce. Use this to write or debug a loader filter."
    )]
    async fn try_loader_filter(
        &self,
        Parameters(args): Parameters<TryFilterArgs>,
    ) -> Result<CallToolResult, McpError> {
        let section = self.exposed_section(&args.section_id)?;
        let document = self.manifest_of(&section).await?;
        if args.query.len() > 4_096 {
            return Err(McpError::invalid_params(
                "jq filters are limited to 4096 characters.".to_string(),
                None,
            ));
        }
        let query = args.query;
        let mapped = tokio::task::spawn_blocking(move || {
            loader::apply(&query, &document).and_then(|value| loader::to_endpoints(&value))
        })
        .await
        .map_err(|err| McpError::internal_error(format!("filter task failed: {err}"), None))?;

        match mapped {
            Ok(endpoints) => ok_json(&serde_json::json!({
                "ok": true,
                "count": endpoints.len(),
                "endpoints": endpoints.iter().take(50).collect::<Vec<_>>(),
            })),
            // A filter that doesn't work yet is the normal case here, so report
            // it as a result to iterate on rather than as a failed call.
            Err(err) => ok_json(&serde_json::json!({
                "ok": false,
                "error": err.to_string(),
            })),
        }
    }
}

#[tool_handler]
impl ServerHandler for FiberMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "fiber",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Fiber exposes API collections the user has explicitly shared. Start with \
             search_endpoints, use get_endpoint when you need the complete request template, \
             then use send_request. The collection's base URL and credentials are applied for \
             you and credential headers are never returned. Only GET, HEAD and OPTIONS are \
             available unless a collection permits writes.",
            )
    }
}

/// Where the app keeps its data, resolved without Tauri so this works headlessly.
///
/// `FIBER_DATA_DIR` overrides the location outright. That's what lets the MCP
/// server run under a container manager like ToolHive: point it at a mounted
/// collections directory instead of the desktop app's own. Unset in normal use.
pub fn app_data_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("FIBER_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("dev.fiber.app")
}

/// Prints this machine's credentials as the JSON map the container reads from
/// `FIBER_SECRETS`, so setting a containerised server up is a pipe rather than
/// an afternoon of copying tokens by hand:
///
/// ```sh
/// fiber mcp export-secrets | thv secret set fiber-secrets
/// ```
///
/// This is the **one** thing the rest of the app refuses to do — read a secret
/// back out of the keychain. It is here because the alternative was worse: a
/// container cannot reach the keychain, so every one of these values had to be
/// found and pasted somewhere by hand anyway, and a person doing that by hand
/// leaves them in a shell history and a scratch file. Three things keep the
/// blast radius small:
///
/// - only collections already shared over MCP are included, so this exports
///   nothing that an agent could not already use;
/// - the output goes to stdout and nowhere else — it is never written to a file;
/// - it refuses to run into a terminal, so it cannot be dumped into scrollback
///   by someone who only wanted a look. It has to be piped somewhere.
pub fn export_secrets() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{IsTerminal, Write};

    if std::io::stdout().is_terminal() {
        return Err("This prints credentials, so it only writes to a pipe.\n\
                    Try: fiber mcp export-secrets | thv secret set fiber-secrets"
            .into());
    }

    let sections = store::load_all(&store::sections_dir(&app_data_dir()))?;
    let exported = collect_secrets(&sections, crate::secrets::get);

    // How many, not which: stderr is the only channel a person is watching here,
    // and naming the collections would defeat the point of the terminal guard.
    eprintln!(
        "{} credential(s) from {} shared collection(s).",
        exported.len(),
        sections
            .iter()
            .filter(|section| section.mcp.enabled)
            .count()
    );

    let mut out = std::io::stdout();
    serde_json::to_writer(&mut out, &serde_json::Value::Object(exported))?;
    out.flush()?;
    Ok(())
}

/// The selection rule, with the keychain passed in — the same shape auth.rs
/// uses, and for the same reason: it is the part worth testing.
fn collect_secrets(
    sections: &[Section],
    lookup: impl Fn(&str) -> Option<String>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut exported = serde_json::Map::new();
    for section in sections.iter().filter(|section| section.mcp.enabled) {
        let Some(reference) = section.auth.secret_ref() else {
            continue;
        };
        // A shared collection whose credential was never filled in is a normal
        // state, not an error — it is simply nothing to export.
        if let Some(value) = lookup(reference) {
            exported.insert(reference.to_string(), serde_json::Value::String(value));
        }
    }
    exported
}

/// Serves MCP over stdio until the client disconnects.
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let data = app_data_dir();
    secrets::validate_injected()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let server = FiberMcp {
        sections_dir: store::sections_dir(&data),
        loaders_dir: loader::loaders_dir(&data),
        http: Arc::new(HttpState::default()),
        auth: Arc::new(AuthState::default()),
        history: Arc::new(HistoryStore::open_or_recover(&data)?),
        sections: Arc::new(Mutex::new(None)),
        manifests: Arc::new(Mutex::new(HashMap::new())),
        endpoint_caches: Arc::new(Mutex::new(HashMap::new())),
        request_ids: Arc::new(AtomicU64::new(0)),
        requests: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        loaders: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_LOADERS)),
        tool_router: FiberMcp::tool_router(),
    };

    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SavedRequest;

    #[test]
    fn only_shared_collections_have_their_credentials_exported() {
        let mut shared = section(true, false);
        shared.id = "shared".into();
        shared.auth = crate::auth::AuthConfig::Bearer {
            secret_ref: "shared:auth".into(),
        };

        // Hidden from MCP entirely, so the container could not use this even if
        // it had the token. Exporting it would hand out a credential for
        // something the user deliberately did not share.
        let mut hidden = section(false, false);
        hidden.id = "hidden".into();
        hidden.auth = crate::auth::AuthConfig::Bearer {
            secret_ref: "hidden:auth".into(),
        };

        // Shared, but no auth configured — nothing to export, and not an error.
        let mut open = section(true, false);
        open.id = "open".into();

        let exported = collect_secrets(&[shared, hidden, open], |reference| {
            Some(format!("value-for-{reference}"))
        });

        assert_eq!(exported.len(), 1);
        assert_eq!(
            exported.get("shared:auth").and_then(|value| value.as_str()),
            Some("value-for-shared:auth")
        );
    }

    #[test]
    fn a_shared_collection_with_no_credential_yet_is_skipped() {
        let mut waiting = section(true, false);
        waiting.auth = crate::auth::AuthConfig::Bearer {
            secret_ref: "sec-1:auth".into(),
        };

        // The keychain has nothing under that reference — the user set the auth
        // up but never signed in.
        assert!(collect_secrets(&[waiting], |_| None).is_empty());
    }

    fn section(enabled: bool, allow_writes: bool) -> Section {
        Section {
            id: "sec-1".into(),
            name: "Acme".into(),
            base_url: "https://api.acme.com".into(),
            collapsed: false,
            order: 0,
            auth: crate::auth::AuthConfig::None,
            loader: None,
            mcp: crate::store::McpAccess {
                enabled,
                allow_writes,
            },
            requests: vec![SavedRequest {
                id: "req-1".into(),
                name: "Get user".into(),
                method: "GET".into(),
                path: "/user/42".into(),
                body: String::new(),
                headers: vec![],
                ..Default::default()
            }],
            overlay: vec![],
            ..Default::default()
        }
    }

    fn server(dir: &std::path::Path) -> FiberMcp {
        FiberMcp {
            sections_dir: dir.to_path_buf(),
            loaders_dir: dir.join("loaders"),
            http: Arc::new(HttpState::default()),
            auth: Arc::new(AuthState::default()),
            history: Arc::new(HistoryStore::open(dir).unwrap()),
            sections: Arc::new(Mutex::new(None)),
            manifests: Arc::new(Mutex::new(HashMap::new())),
            endpoint_caches: Arc::new(Mutex::new(HashMap::new())),
            request_ids: Arc::new(AtomicU64::new(0)),
            requests: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            loaders: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_LOADERS)),
            tool_router: FiberMcp::tool_router(),
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fetch-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_section_is_invisible_until_it_is_shared() {
        let dir = scratch("visibility");
        store::save(&dir, &section(false, false)).unwrap();
        let mcp = server(&dir);
        assert!(
            mcp.exposed().unwrap().is_empty(),
            "not shared means not listed"
        );

        store::save(&dir, &section(true, false)).unwrap();
        assert_eq!(mcp.exposed().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unshared_section_cannot_be_addressed_directly() {
        // Knowing the id must not be enough — otherwise "invisible" would only
        // mean "unlisted".
        let dir = scratch("addressing");
        store::save(&dir, &section(false, false)).unwrap();
        assert!(server(&dir).exposed_section("sec-1").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_need_their_own_switch() {
        assert!(is_read_only("get"));
        assert!(is_read_only("HEAD"));
        assert!(is_read_only("OPTIONS"));
        for method in ["POST", "PUT", "PATCH", "DELETE", "post"] {
            assert!(!is_read_only(method), "{method} must count as a write");
        }
    }

    #[test]
    fn credentials_never_travel_back() {
        let headers = vec![
            Header {
                name: "Authorization".into(),
                value: "Bearer super-secret".into(),
            },
            Header {
                name: "set-cookie".into(),
                value: "session=super-secret".into(),
            },
            Header {
                name: "X-Api-Key".into(),
                value: "super-secret".into(),
            },
            Header {
                name: "Content-Type".into(),
                value: "application/json".into(),
            },
        ];

        let safe = redact_with(&headers, None);
        let rendered = serde_json::to_string(&safe).unwrap();
        assert!(
            !rendered.contains("super-secret"),
            "a credential leaked: {rendered}"
        );
        // Everything else survives, or the result would be useless.
        assert_eq!(safe[3].value, "application/json");
    }

    #[test]
    fn endpoints_come_from_both_hand_written_and_loaded() {
        let dir = scratch("endpoints");
        let mut acme = section(true, false);
        acme.loader = Some(loader::LoaderConfig::default());
        store::save(&dir, &acme).unwrap();

        let loaders = dir.join("loaders");
        loader::write_cache(
            &loaders,
            "sec-1",
            &loader::LoaderCache {
                loaded_at: 1,
                endpoints: vec![loader::LoadedEndpoint {
                    method: "POST".into(),
                    path: "/orders".into(),
                    name: "createOrder".into(),
                    description: String::new(),
                    body: String::new(),
                    ..Default::default()
                }],
                schemas: Default::default(),
                response_schemas: Default::default(),
            },
        )
        .unwrap();

        let mcp = server(&dir);
        let found = mcp.endpoints_of(&acme);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|e| e.key == "req-1" && !e.loaded));
        assert!(found.iter().any(|e| e.key == "POST /orders" && e.loaded));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_bodies_are_cut_short() {
        let (short, cut) = truncate("hello", 10);
        assert_eq!(short, "hello");
        assert!(!cut);

        let (long, cut) = truncate(&"x".repeat(50), 10);
        assert_eq!(long.len(), 10);
        assert!(cut);

        // The cut counts characters, not bytes — a multi-byte character on the
        // boundary must come through whole or not at all.
        let (accented, cut) = truncate(&"é".repeat(5), 3);
        assert_eq!(accented, "ééé");
        assert!(cut);
    }

    #[test]
    fn response_ids_are_unique_inside_one_millisecond() {
        let dir = scratch("response-ids");
        let mcp = server(&dir);
        let ids: std::collections::HashSet<_> =
            (0..1_000).map(|_| mcp.next_response_id()).collect();
        assert_eq!(ids.len(), 1_000);
        assert!(ids.iter().all(|id| crate::store::is_safe_id(id)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An agent supplies the path and the section supplies the credential — so
    /// an absolute URL is only honoured on the section's own origin. Anywhere
    /// else, "fetch http://attacker/ for me" would carry the token along.
    #[test]
    fn an_absolute_path_may_not_leave_the_sections_origin() {
        let base = "https://api.acme.com";
        assert!(
            require_same_origin(base, "/user/42").is_ok(),
            "relative paths are the normal case, untouched"
        );
        assert!(require_same_origin(base, "https://api.acme.com/user/42").is_ok());
        assert!(
            require_same_origin(base, "https://api.acme.com:443/user/42").is_ok(),
            "spelling out the default port is the same origin"
        );

        assert!(require_same_origin(base, "https://evil.example/collect").is_err());
        assert!(
            require_same_origin(base, "http://api.acme.com/user").is_err(),
            "a scheme downgrade is a different origin"
        );
        assert!(
            require_same_origin(base, "http://169.254.169.254/latest/meta-data/").is_err(),
            "cloud metadata endpoints are exactly what this guard is for"
        );
        assert!(
            require_same_origin("", "https://api.acme.com/x").is_err(),
            "no base URL means nothing to compare against"
        );
    }
}
