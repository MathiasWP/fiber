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

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};

use crate::auth::AuthState;
use crate::history::HistoryStore;
use crate::http::{Header, HttpState, RequestSpec};
use crate::loader;
use crate::secrets;
use crate::store::{self, Section};

/// Response bodies beyond this are cut short; `query_response` reaches the rest.
const MAX_BODY_CHARS: usize = 8_000;
/// Manifests are shown to help write a filter, not to be read in full.
const MAX_MANIFEST_CHARS: usize = 12_000;

/// Headers that must never travel back to a model.
fn is_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization" | "x-api-key"
    )
}

fn redact(headers: &[Header]) -> Vec<Header> {
    headers
        .iter()
        .map(|header| Header {
            name: header.name.clone(),
            value: if is_credential(&header.name) {
                "<redacted>".into()
            } else {
                header.value.clone()
            },
        })
        .collect()
}

fn truncate(body: &str, limit: usize) -> (String, bool) {
    if body.chars().count() <= limit {
        return (body.to_string(), false);
    }
    (body.chars().take(limit).collect(), true)
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
    /// Matched against endpoint name, method and path. Empty lists everything.
    #[serde(default)]
    query: String,
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

#[derive(Clone)]
pub struct FiberMcp {
    sections_dir: std::path::PathBuf,
    loaders_dir: std::path::PathBuf,
    http: Arc<HttpState>,
    auth: Arc<AuthState>,
    history: Arc<HistoryStore>,
    #[expect(dead_code, reason = "the tool_handler macro reads this field")]
    tool_router: ToolRouter<Self>,
}

impl FiberMcp {
    /// Only sections the user has explicitly exposed. Everything else is
    /// invisible, not merely read-only.
    fn exposed(&self) -> Vec<Section> {
        store::load_all(&self.sections_dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|section| section.mcp.enabled)
            .collect()
    }

    fn exposed_section(&self, id: &str) -> Result<Section, McpError> {
        self.exposed()
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
                loaded: false,
            })
            .collect();

        if let Some(cache) = loader::read_cache(&self.loaders_dir, &section.id) {
            found.extend(cache.endpoints.into_iter().map(|endpoint| EndpointSummary {
                section_id: section.id.clone(),
                section: section.name.clone(),
                key: endpoint.key(),
                method: endpoint.method,
                path: endpoint.path,
                name: endpoint.name,
                loaded: true,
            }));
        }
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
                let spec = RequestSpec {
                    id: format!("mcp-loader:{}", section.id),
                    request_id: format!("loader:{}", section.id),
                    section_id: Some(section.id.clone()),
                    method: request.method,
                    url: store::join_url(&section.base_url, &request.url),
                    headers: vec![Header {
                        name: "Accept".into(),
                        value: "application/json".into(),
                    }],
                    body: None,
                    timeout_ms: Some(30_000),
                    follow_redirects: true,
                    accept_invalid_certs: false,
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
        let response = (self.fetcher(section))(loader::LoaderRequest {
            url: config.url.clone(),
            method,
        })
        .await
        .map_err(|err| McpError::internal_error(err, None))?;

        serde_json::from_str(&response.body)
            .map_err(|err| McpError::internal_error(format!("manifest wasn't JSON: {err}"), None))
    }
}

fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(value)
            .map_err(|err| McpError::internal_error(err.to_string(), None))?,
    )]))
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

        let summaries: Vec<Summary> = self
            .exposed()
            .iter()
            .map(|section| Summary {
                id: section.id.clone(),
                name: section.name.clone(),
                base_url: section.base_url.clone(),
                endpoints: self.endpoints_of(section).len(),
                allows_writes: section.mcp.allow_writes,
                has_loader: section.loader.is_some(),
            })
            .collect();

        ok_json(&summaries)
    }

    #[tool(
        description = "Search endpoints across shared collections by name, method or path. Start here to find what to call."
    )]
    async fn search_endpoints(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let needle = args.query.trim().to_lowercase();
        let mut found: Vec<EndpointSummary> = self
            .exposed()
            .iter()
            .flat_map(|section| self.endpoints_of(section))
            .filter(|endpoint| {
                needle.is_empty()
                    || format!("{} {} {}", endpoint.name, endpoint.method, endpoint.path)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect();

        found.truncate(200);
        ok_json(&found)
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

        let id = format!("mcp-{}-{}", section.id, crate::history::now_millis());
        let url = store::join_url(&section.base_url, &args.path);
        let body = args.body.filter(|body| !body.trim().is_empty());

        let spec = RequestSpec {
            id: id.clone(),
            request_id: format!("mcp:{}", section.id),
            section_id: Some(section.id.clone()),
            method,
            url: url.clone(),
            headers: match &body {
                Some(_) => vec![Header {
                    name: "Content-Type".into(),
                    value: "application/json".into(),
                }],
                None => vec![],
            },
            body,
            timeout_ms: Some(60_000),
            follow_redirects: true,
            accept_invalid_certs: false,
        };

        let at = crate::history::now_millis();
        let outcome =
            crate::send_authenticated(&self.http, &self.auth, Some(&section), spec.clone(), &secrets::get, None)
                .await;
        let _ = self.history.record(&spec, at, &url, &outcome);

        let response = outcome.map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let (body, truncated) = truncate(&response.body, MAX_BODY_CHARS);

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply {
            response_id: String,
            status: u16,
            status_text: String,
            headers: Vec<Header>,
            size_bytes: u64,
            body: String,
            truncated: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            hint: Option<String>,
        }

        ok_json(&Reply {
            response_id: id,
            status: response.status,
            status_text: response.status_text,
            headers: redact(&response.headers),
            size_bytes: response.size_bytes,
            body,
            truncated,
            hint: truncated.then(|| {
                "Body truncated. Use query_response with a jq filter to read the rest.".to_string()
            }),
        })
    }

    #[tool(
        description = "Query a stored response with a jq filter, for reading a large body without pulling all of it into context."
    )]
    async fn query_response(
        &self,
        Parameters(args): Parameters<QueryResponseArgs>,
    ) -> Result<CallToolResult, McpError> {
        let body = self
            .history
            .body(&args.response_id)
            .ok()
            .flatten()
            .ok_or_else(|| {
                McpError::invalid_params("No stored response with that id.".to_string(), None)
            })?;

        if args.query.trim().is_empty() {
            let (body, truncated) = truncate(&body, MAX_BODY_CHARS);
            return ok_json(&serde_json::json!({ "body": body, "truncated": truncated }));
        }

        let document: serde_json::Value = serde_json::from_str(&body)
            .map_err(|err| McpError::invalid_params(format!("that response isn't JSON: {err}"), None))?;
        let result = loader::apply(&args.query, &document)
            .map_err(|err| McpError::invalid_params(err.to_string(), None))?;

        let (rendered, truncated) = truncate(&result.to_string(), MAX_BODY_CHARS);
        ok_json(&serde_json::json!({ "result": rendered, "truncated": truncated }))
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

        let (endpoints, pages) = loader::run(&config, self.fetcher(&section))
            .await
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;

        let previous = loader::read_cache(&self.loaders_dir, &section.id).unwrap_or_default();
        let (added, removed) = loader::diff(&previous.endpoints, &endpoints);
        let _ = loader::write_cache(
            &self.loaders_dir,
            &section.id,
            &loader::LoaderCache {
                loaded_at: crate::history::now_millis(),
                endpoints: endpoints.clone(),
            },
        );

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

        match loader::apply(&args.query, &document).and_then(|value| loader::to_endpoints(&value)) {
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
             search_endpoints to find an endpoint, then send_request to call it — the \
             collection's base URL and credentials are applied for you and never returned. \
             Only GET, HEAD and OPTIONS are available unless a collection permits writes.",
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

/// Serves MCP over stdio until the client disconnects.
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let data = app_data_dir();
    // Whichever of the app or this server starts first does the rename
    // migration; the other finds it already done.
    if crate::migrate::data_dir(&data) {
        let sections = store::sections_dir(&data);
        crate::migrate::secrets(crate::migrate::references(&sections).into_iter());
    }
    let server = FiberMcp {
        sections_dir: store::sections_dir(&data),
        loaders_dir: loader::loaders_dir(&data),
        http: Arc::new(HttpState::default()),
        auth: Arc::new(AuthState::default()),
        history: Arc::new(HistoryStore::open(&data)?),
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
            }],
            overlay: vec![],
        }
    }

    fn server(dir: &std::path::Path) -> FiberMcp {
        FiberMcp {
            sections_dir: dir.to_path_buf(),
            loaders_dir: dir.join("loaders"),
            http: Arc::new(HttpState::default()),
            auth: Arc::new(AuthState::default()),
            history: Arc::new(HistoryStore::open(dir).unwrap()),
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
        assert!(mcp.exposed().is_empty(), "not shared means not listed");

        store::save(&dir, &section(true, false)).unwrap();
        assert_eq!(mcp.exposed().len(), 1);
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

        let safe = redact(&headers);
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
                }],
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
    }
}
