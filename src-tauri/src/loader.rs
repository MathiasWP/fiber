//! Dynamic endpoint loaders.
//!
//! A section can point at an endpoint that publishes the API's own route
//! manifest, and describe how to turn that document into an endpoint list.
//!
//! The description is a **jq filter**, not a program. A loader is a JSON→JSON
//! transformation, which is a solved problem with an established language, and
//! jq buys three things a scripting engine didn't:
//!
//! - It can't do anything but transform. No I/O, no host access, nothing to
//!   sandbox — which matters most once the MCP server can trigger a refresh.
//! - Being pure, a filter can be re-run against an already-fetched document
//!   instantly, so the editor shows the result as you type instead of making
//!   you run a script and read a stack trace.
//! - Most people writing API tooling already know it.
//!
//! The one thing jq can't do is make a second request, so pagination is a
//! separate declarative field rather than a reason to embed a language.
//!
//! Free of Tauri and of the HTTP stack — the fetcher is injected — so the MCP
//! server can run a loader headlessly.

use std::sync::Arc;
use std::time::Duration;

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::Val;
use serde::{Deserialize, Serialize};

/// Ceiling on one refresh, however many pages it walks.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
/// A `next` pointer that never goes null shouldn't fetch forever.
const MAX_PAGES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoaderConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Where the manifest lives. Relative to the section's base URL.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub method: String,
    /// jq filter producing `{method, path, name?, description?}` objects.
    #[serde(default)]
    pub query: String,
    /// jq filter yielding the next page's URL, or null when done. Empty to
    /// fetch a single page.
    #[serde(default)]
    pub next: String,
    /// 0 means "only when asked".
    #[serde(default)]
    pub ttl_seconds: u64,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "/openapi.json".to_string(),
            method: "GET".to_string(),
            query: DEFAULT_QUERY.to_string(),
            next: String::new(),
            ttl_seconds: 0,
        }
    }
}

/// Starting points for common manifest shapes, offered in the editor.
///
/// OpenAPI leads because it is the one an API is most likely to already
/// publish, and it is what a new loader starts with.
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "OpenAPI",
        ".paths | to_entries | map(.key as $path | .value | to_entries | map({method: .key, path: $path, name: $path})) | flatten",
    ),
    (
        "Array of routes",
        ".routes | map({method: .verb, path: .url, name: .handler})",
    ),
    (
        "Top-level array",
        "map({method: .method, path: .path, name: .name})",
    ),
    (
        "Skip deprecated",
        ".routes | map(select(.deprecated | not) | {method: .verb, path: .url, name: .handler})",
    ),
];

/// Taken from the list rather than written out again, so the filter a new
/// loader starts with and the one the editor offers first cannot drift apart.
pub const DEFAULT_QUERY: &str = TEMPLATES[0].1;

/// One endpoint as reported by a loader. Never persisted as the section's
/// endpoint list — see the overlay model in §6.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadedEndpoint {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A JSON body to start from. Derived from the manifest when it is an
    /// OpenAPI document; `default` so a cache written before this existed, and
    /// a filter that says nothing about bodies, both still read.
    #[serde(default)]
    pub body: String,
}

impl LoadedEndpoint {
    /// The stable identity a user's saved body and history hang off. Deliberately
    /// derived from method and path rather than a generated id, so a refresh
    /// re-attaches rather than orphaning.
    pub fn key(&self) -> String {
        format!("{} {}", self.method.trim().to_uppercase(), self.path.trim())
    }

    fn tidy(mut self) -> Self {
        self.method = self.method.trim().to_uppercase();
        self.path = self.path.trim().to_string();
        if self.name.trim().is_empty() {
            self.name = self.path.clone();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoaderCache {
    /// Epoch millis of the last successful run.
    pub loaded_at: i64,
    pub endpoints: Vec<LoadedEndpoint>,
}

/// What a run produced, including what changed since last time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoaderRun {
    pub endpoints: Vec<LoadedEndpoint>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub loaded_at: i64,
    pub pages: usize,
}

/// A request the loader makes, before the section's base URL and auth apply.
#[derive(Debug, Clone)]
pub struct LoaderRequest {
    pub url: String,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct LoaderResponse {
    pub status: u16,
    pub body: String,
}

/// How the host performs the loader's request. Injected so this module stays
/// free of both Tauri and the HTTP stack.
pub type Fetcher = Arc<
    dyn Fn(
            LoaderRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<LoaderResponse, String>> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    #[error("no query — describe how to turn the response into endpoints")]
    NoQuery,
    #[error("no URL — point the loader at the endpoint that lists your routes")]
    NoUrl,
    #[error("the manifest request failed: {0}")]
    Fetch(String),
    #[error("the manifest request returned {0}")]
    Status(u16),
    #[error("the response was not JSON: {0}")]
    NotJson(String),
    #[error("that filter isn't valid jq: {0}")]
    BadQuery(String),
    #[error("the filter failed: {0}")]
    QueryFailed(String),
    #[error("the filter produced something other than a list of endpoints: {0}")]
    BadShape(String),
    #[error("loader took longer than {}s", RUN_TIMEOUT.as_secs())]
    Timeout,
}

impl Serialize for LoaderError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Runs a jq filter over one JSON document, returning the single output value.
///
/// Pure: no I/O, no host access, no way to reach anything but the input. This
/// is what makes live preview possible and what makes a shared collection safe
/// to open.
pub fn apply(query: &str, input: &serde_json::Value) -> Result<serde_json::Value, LoaderError> {
    if query.trim().is_empty() {
        return Err(LoaderError::NoQuery);
    }

    let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
    // jq's `env` builtin returns the whole process environment. A filter has no
    // business reading it, and every reason not to: filters get shared inside
    // collections and, from step 6, authored by an agent. Dropping it is the
    // difference between "pure transformation" and "pure transformation, except
    // it can read your secrets".
    let funs = jaq_core::funs()
        .chain(jaq_std::funs().filter(|fun| fun.0 != "env"))
        .chain(jaq_json::funs());

    let arena = Arena::default();
    let modules = Loader::new(defs)
        .load(&arena, File { code: query, path: () })
        .map_err(|errors| LoaderError::BadQuery(describe_load(errors)))?;

    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errors| LoaderError::BadQuery(describe_compile(errors)))?;

    // jaq's value type bridges to serde_json through JSON text, which keeps us
    // off its internal representation.
    let encoded = serde_json::to_string(input).map_err(|err| LoaderError::NotJson(err.to_string()))?;
    let input = jaq_json::read::parse_single(encoded.as_bytes())
        .map_err(|err| LoaderError::NotJson(err.to_string()))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut outputs = filter.id.run((ctx, input)).map(unwrap_valr);

    let first = outputs
        .next()
        .ok_or_else(|| LoaderError::QueryFailed("the filter produced no output".into()))?
        .map_err(|err| LoaderError::QueryFailed(err.to_string()))?;

    serde_json::from_str(&first.to_string()).map_err(|err| LoaderError::NotJson(err.to_string()))
}

/// jq reports errors as spans into the source; the message alone is what a
/// person can act on.
fn describe_load<T: std::fmt::Debug>(errors: T) -> String {
    format!("{errors:?}")
}

fn describe_compile<T: std::fmt::Debug>(errors: T) -> String {
    format!("{errors:?}")
}

/// Turns a filter's output into endpoints, with messages aimed at whoever wrote
/// the filter rather than at whoever wrote this file.
pub fn to_endpoints(value: &serde_json::Value) -> Result<Vec<LoadedEndpoint>, LoaderError> {
    let items = value.as_array().ok_or_else(|| {
        LoaderError::BadShape(format!(
            "expected an array, got {}. A filter usually ends in `map({{...}})`.",
            kind_of(value)
        ))
    })?;

    let mut endpoints = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            LoaderError::BadShape(format!("item {index} is {}, not an object", kind_of(item)))
        })?;
        for required in ["method", "path"] {
            if !object.get(required).is_some_and(|value| value.is_string()) {
                return Err(LoaderError::BadShape(format!(
                    "item {index} needs a string `{required}`"
                )));
            }
        }
        let endpoint: LoadedEndpoint = serde_json::from_value(item.clone())
            .map_err(|err| LoaderError::BadShape(format!("item {index}: {err}")))?;
        endpoints.push(endpoint.tidy());
    }

    Ok(endpoints)
}

fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Fetches the manifest — following `next` while it yields a URL — and maps
/// every page through the filter.
pub async fn run(
    config: &LoaderConfig,
    fetcher: Fetcher,
) -> Result<(Vec<LoadedEndpoint>, usize), LoaderError> {
    if config.url.trim().is_empty() {
        return Err(LoaderError::NoUrl);
    }
    if config.query.trim().is_empty() {
        return Err(LoaderError::NoQuery);
    }

    tokio::time::timeout(RUN_TIMEOUT, async move {
        let method = match config.method.trim() {
            "" => "GET".to_string(),
            method => method.to_string(),
        };

        let mut url = config.url.trim().to_string();
        let mut endpoints = Vec::new();
        let mut pages = 0;

        loop {
            let document = fetch_json(&fetcher, &url, &method).await?;
            let mut mapped = to_endpoints(&apply(&config.query, &document)?)?;
            fill_bodies(&document, &mut mapped);
            endpoints.extend(mapped);
            pages += 1;

            if config.next.trim().is_empty() || pages >= MAX_PAGES {
                break;
            }
            match apply(&config.next, &document)? {
                serde_json::Value::String(next) if !next.trim().is_empty() => url = next,
                // Null, or anything that isn't a URL, means the last page.
                _ => break,
            }
        }

        Ok((endpoints, pages))
    })
    .await
    .map_err(|_| LoaderError::Timeout)?
}

async fn fetch_json(
    fetcher: &Fetcher,
    url: &str,
    method: &str,
) -> Result<serde_json::Value, LoaderError> {
    let response = fetcher(LoaderRequest {
        url: url.to_string(),
        method: method.to_string(),
    })
    .await
    .map_err(LoaderError::Fetch)?;

    if !(200..300).contains(&response.status) {
        return Err(LoaderError::Status(response.status));
    }
    serde_json::from_str(&response.body).map_err(|err| LoaderError::NotJson(err.to_string()))
}

/// Fills in request bodies, when the manifest turns out to be OpenAPI.
///
/// A jq filter maps a document to endpoints, and that is all it should do:
/// walking a JSON Schema into an example is not something anyone should have to
/// write in jq. So the body comes from here instead, off the same document the
/// filter just read, through the importer's own schema walk — which means a
/// collection filled by a loader and one filled by importing the file end up
/// with the same bodies.
///
/// Anything else — a routes array, a bespoke manifest — has no `paths`, so this
/// does nothing and every endpoint keeps the empty body it arrived with.
fn fill_bodies(document: &serde_json::Value, endpoints: &mut [LoadedEndpoint]) {
    let Some(paths) = document.get("paths").and_then(|paths| paths.as_object()) else {
        return;
    };

    for endpoint in endpoints {
        // A filter that supplies its own body outranks anything derived.
        if !endpoint.body.is_empty() {
            continue;
        }
        let operation = paths
            .get(&endpoint.path)
            .and_then(|item| item.get(endpoint.method.to_ascii_lowercase()))
            .and_then(|operation| operation.as_object());

        if let Some(operation) = operation {
            endpoint.body = crate::openapi::request_body(document, operation);
        }
    }
}

/// `<app data>/loaders`
pub fn loaders_dir(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join("loaders")
}

fn cache_path(dir: &std::path::Path, section_id: &str) -> Option<std::path::PathBuf> {
    // Same guard as section files: an id becomes a file name.
    let safe = !section_id.is_empty()
        && section_id.len() <= 128
        && section_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| dir.join(format!("{section_id}.json")))
}

/// The last successful run. Loader output is a cache, never the source of
/// truth, so a missing or unreadable file is simply "nothing loaded yet".
pub fn read_cache(dir: &std::path::Path, section_id: &str) -> Option<LoaderCache> {
    let path = cache_path(dir, section_id)?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_cache(
    dir: &std::path::Path,
    section_id: &str,
    cache: &LoaderCache,
) -> std::io::Result<()> {
    let Some(path) = cache_path(dir, section_id) else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let encoded = serde_json::to_string_pretty(cache)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    std::fs::write(path, encoded)
}

pub fn forget_cache(dir: &std::path::Path, section_id: &str) {
    if let Some(path) = cache_path(dir, section_id) {
        let _ = std::fs::remove_file(path);
    }
}

/// What changed between two runs, by stable key.
pub fn diff(previous: &[LoadedEndpoint], next: &[LoadedEndpoint]) -> (Vec<String>, Vec<String>) {
    let before: Vec<String> = previous.iter().map(LoadedEndpoint::key).collect();
    let after: Vec<String> = next.iter().map(LoadedEndpoint::key).collect();

    let added = after
        .iter()
        .filter(|key| !before.contains(key))
        .cloned()
        .collect();
    let removed = before
        .iter()
        .filter(|key| !after.contains(key))
        .cloned()
        .collect();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(query: &str) -> LoaderConfig {
        LoaderConfig {
            enabled: true,
            url: "/internal/endpoints".into(),
            method: "GET".into(),
            query: query.into(),
            next: String::new(),
            ttl_seconds: 0,
        }
    }

    /// Answers every page with the same document, recording what was asked for.
    fn answering(body: &'static str) -> Fetcher {
        Arc::new(move |request: LoaderRequest| {
            Box::pin(async move {
                Ok(LoaderResponse {
                    status: 200,
                    body: body.replace("{{url}}", &request.url),
                })
            })
        })
    }

    #[tokio::test]
    async fn maps_a_route_manifest() {
        let routes = TEMPLATES.iter().find(|(name, _)| *name == "Array of routes").unwrap().1;
        let (endpoints, pages) = run(
            &config(routes),
            answering(
                r#"{"routes":[
                    {"verb":"get","url":"/user/42","handler":"getUser"},
                    {"verb":"post","url":"/user","handler":"createUser"}
                ]}"#,
            ),
        )
        .await
        .unwrap();

        assert_eq!(pages, 1);
        assert_eq!(endpoints.len(), 2);
        // Methods are normalised, so a manifest saying "get" keys the same as a
        // hand-written GET.
        assert_eq!(endpoints[0].key(), "GET /user/42");
        assert_eq!(endpoints[0].name, "getUser");
        assert_eq!(endpoints[1].key(), "POST /user");
    }

    #[test]
    fn maps_openapi_without_a_dedicated_importer() {
        // Object-keyed rather than an array — the shape a fixed field-mapping
        // schema could never express, and the reason for a real query language.
        let document = json!({
            "paths": {
                "/users": {
                    "get": { "operationId": "listUsers" },
                    "post": { "operationId": "createUser" }
                },
                "/users/{id}": {
                    "get": { "operationId": "getUser" }
                }
            }
        });

        let query = TEMPLATES.iter().find(|(name, _)| *name == "OpenAPI").unwrap().1;
        let mut endpoints = to_endpoints(&apply(query, &document).unwrap()).unwrap();
        endpoints.sort_by_key(|endpoint| endpoint.key());

        let keys: Vec<String> = endpoints.iter().map(LoadedEndpoint::key).collect();
        assert_eq!(
            keys,
            vec!["GET /users", "GET /users/{id}", "POST /users"]
        );
        assert_eq!(endpoints[0].name, "/users");
    }

    #[test]
    fn filters_with_select() {
        let document = json!({
            "routes": [
                {"verb": "GET", "url": "/live", "handler": "live", "deprecated": false},
                {"verb": "GET", "url": "/old", "handler": "old", "deprecated": true}
            ]
        });

        let query = TEMPLATES.iter().find(|(name, _)| *name == "Skip deprecated").unwrap().1;
        let endpoints = to_endpoints(&apply(query, &document).unwrap()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].path, "/live");
    }

    #[test]
    fn a_missing_name_falls_back_to_the_path() {
        let document = json!([{ "method": "GET", "path": "/thing" }]);
        let endpoints =
            to_endpoints(&apply("map({method, path})", &document).unwrap()).unwrap();
        assert_eq!(endpoints[0].name, "/thing");
    }

    #[tokio::test]
    async fn follows_pagination_until_it_runs_out() {
        // Page 1 points at page 2; page 2's `next` is null.
        let fetcher: Fetcher = Arc::new(move |request: LoaderRequest| {
            Box::pin(async move {
                let body = if request.url.contains("page=2") {
                    r#"{"routes":[{"verb":"GET","url":"/second"}],"links":{"next":null}}"#
                } else {
                    r#"{"routes":[{"verb":"GET","url":"/first"}],"links":{"next":"/routes?page=2"}}"#
                };
                Ok(LoaderResponse {
                    status: 200,
                    body: body.to_string(),
                })
            })
        });

        let mut settings = config(".routes | map({method: .verb, path: .url})");
        settings.next = ".links.next".to_string();

        let (endpoints, pages) = run(&settings, fetcher).await.unwrap();
        assert_eq!(pages, 2);
        assert_eq!(
            endpoints.iter().map(LoadedEndpoint::key).collect::<Vec<_>>(),
            vec!["GET /first", "GET /second"]
        );
    }

    #[tokio::test]
    async fn a_next_pointer_that_never_ends_is_capped() {
        // Always points at itself, which without a cap would fetch forever.
        let fetcher: Fetcher = Arc::new(|_| {
            Box::pin(async {
                Ok(LoaderResponse {
                    status: 200,
                    body: r#"{"routes":[{"verb":"GET","url":"/loop"}],"links":{"next":"/again"}}"#
                        .to_string(),
                })
            })
        });

        let mut settings = config(".routes | map({method: .verb, path: .url})");
        settings.next = ".links.next".to_string();

        let (endpoints, pages) = run(&settings, fetcher).await.unwrap();
        assert_eq!(pages, MAX_PAGES);
        assert_eq!(endpoints.len(), MAX_PAGES);
    }


    /// The reported gap: a loader pointed at an OpenAPI document listed the
    /// endpoints but left every body empty, because a jq filter maps shape to
    /// shape and knows nothing about JSON Schema.
    #[tokio::test]
    async fn an_openapi_manifest_fills_in_bodies() {
        let manifest = r##"{
            "openapi": "3.1.1",
            "paths": {
                "/activity/backfill-activity": {
                    "post": {
                        "operationId": "activity_backfill_activity",
                        "requestBody": {
                            "required": true,
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "offset": { "type": "number" },
                                            "messageIndex": { "type": "number" },
                                            "dryRun": { "type": "boolean" }
                                        },
                                        "additionalProperties": false
                                    }
                                }
                            }
                        }
                    },
                    "get": { "operationId": "activity_status" }
                }
            }
        }"##;

        let openapi = TEMPLATES.iter().find(|(name, _)| *name == "OpenAPI").unwrap().1;
        let (endpoints, _) = run(&config(openapi), answering(manifest)).await.unwrap();

        let post = endpoints.iter().find(|e| e.method == "POST").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&post.body).unwrap(),
            json!({ "offset": 0, "messageIndex": 0, "dryRun": false })
        );

        // A GET declares no body, and gets none.
        let get = endpoints.iter().find(|e| e.method == "GET").unwrap();
        assert_eq!(get.body, "");
    }

    /// A manifest that is not OpenAPI has no `paths`, so nothing is derived and
    /// nothing breaks.
    #[tokio::test]
    async fn a_routes_manifest_is_left_alone() {
        let routes = TEMPLATES.iter().find(|(name, _)| *name == "Array of routes").unwrap().1;
        let (endpoints, _) = run(
            &config(routes),
            answering(r#"{"routes":[{"verb":"post","url":"/user","handler":"createUser"}]}"#),
        )
        .await
        .unwrap();

        assert_eq!(endpoints[0].body, "");
    }

    #[tokio::test]
    async fn reports_a_failed_or_rejected_manifest_request() {
        let failing: Fetcher = Arc::new(|_| Box::pin(async { Err("could not connect".into()) }));
        assert!(matches!(
            run(&config(DEFAULT_QUERY), failing).await,
            Err(LoaderError::Fetch(_))
        ));

        let forbidden: Fetcher = Arc::new(|_| {
            Box::pin(async {
                Ok(LoaderResponse {
                    status: 403,
                    body: String::new(),
                })
            })
        });
        assert!(matches!(
            run(&config(DEFAULT_QUERY), forbidden).await,
            Err(LoaderError::Status(403))
        ));
    }

    #[tokio::test]
    async fn reports_a_response_that_is_not_json() {
        let html: Fetcher = Arc::new(|_| {
            Box::pin(async {
                Ok(LoaderResponse {
                    status: 200,
                    body: "<html>login</html>".to_string(),
                })
            })
        });
        assert!(matches!(
            run(&config(DEFAULT_QUERY), html).await,
            Err(LoaderError::NotJson(_))
        ));
    }

    #[test]
    fn reports_a_filter_that_is_not_valid_jq() {
        match apply(".routes | map({", &json!({})) {
            Err(LoaderError::BadQuery(message)) => assert!(!message.is_empty()),
            other => panic!("expected a query error, got {other:?}"),
        }
        assert!(matches!(apply("   ", &json!({})), Err(LoaderError::NoQuery)));
    }

    #[test]
    fn explains_output_that_is_not_endpoints() {
        let cases: Vec<(&str, &str)> = vec![
            ("42", "expected an array"),
            ("[1]", "not an object"),
            ("[{path: \"/x\"}]", "`method`"),
            ("[{method: \"GET\"}]", "`path`"),
        ];

        for (query, expected) in cases {
            let value = apply(query, &json!({})).unwrap();
            match to_endpoints(&value) {
                Err(LoaderError::BadShape(message)) => {
                    assert!(message.contains(expected), "{message} should mention {expected}");
                }
                other => panic!("expected a shape error for {query}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_filter_cannot_read_the_environment() {
        // jaq-std ships jq's `env`, which hands back every environment variable
        // of this process. It's removed from the builtin set; this is the guard
        // that keeps it removed.
        let document = json!({ "routes": [] });
        for probe in ["env", "$ENV", "env.PATH", "$ENV.PATH"] {
            match apply(probe, &document) {
                // Undefined is the outcome we want.
                Err(LoaderError::BadQuery(_)) => {}
                Ok(value) => assert!(
                    !value.to_string().contains("PATH"),
                    "`{probe}` exposed the environment: {value}"
                ),
                Err(other) => panic!("unexpected error for `{probe}`: {other}"),
            }
        }
    }

    #[test]
    fn a_filter_has_no_other_way_out() {
        // Nothing in jq names a file, a socket or a process, so there is no
        // capability to withhold — unlike an interpreter, where the absence of
        // one has to be arranged.
        let document = json!({ "routes": [] });
        for probe in ["input", "inputs", "$__prog__", "open(\"/etc/passwd\")"] {
            let outcome = apply(probe, &document);
            assert!(
                !matches!(&outcome, Ok(value) if value.to_string().contains("root:")),
                "`{probe}` read something it shouldn't"
            );
        }
    }

    #[test]
    fn diffs_by_stable_key() {
        let before = vec![
            LoadedEndpoint {
                method: "GET".into(),
                path: "/a".into(),
                name: "a".into(),
                description: String::new(),
                body: String::new(),
            },
            LoadedEndpoint {
                method: "GET".into(),
                path: "/gone".into(),
                name: "gone".into(),
                description: String::new(),
                body: String::new(),
            },
        ];
        let after = vec![
            LoadedEndpoint {
                // Renaming an endpoint is not a change of identity.
                method: "GET".into(),
                path: "/a".into(),
                name: "renamed".into(),
                description: String::new(),
                body: String::new(),
            },
            LoadedEndpoint {
                method: "POST".into(),
                path: "/new".into(),
                name: "new".into(),
                description: String::new(),
                body: String::new(),
            },
        ];

        let (added, removed) = diff(&before, &after);
        assert_eq!(added, vec!["POST /new"]);
        assert_eq!(removed, vec!["GET /gone"]);
    }
}
