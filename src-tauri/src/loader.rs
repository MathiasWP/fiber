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

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
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
            // Not 0 ("only when asked"): with focus as a trigger, a new loader
            // that quietly keeps itself current is the more useful default, and
            // five minutes is short enough to be right and long enough not to
            // matter. Set it back to 0 for an API where every call counts.
            ttl_seconds: 300,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    /// Whatever the manifest says about this endpoint beyond the fields above:
    /// every scalar `x-` extension when the manifest is OpenAPI, plus anything
    /// a filter chose to put here for a manifest that isn't.
    ///
    /// Fiber never reads a key of its own out of this. It exists so an access
    /// policy can be written against the vocabulary an API already publishes —
    /// `x-kind`, `x-scope`, `deprecated` — instead of Fiber guessing from the
    /// HTTP method, which for an API where every call is a POST tells you
    /// nothing at all.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub parameters: Vec<crate::openapi::SpecParam>,
    /// A JSON body to start from. Derived from the manifest when it is an
    /// OpenAPI document; `default` so a cache written before this existed, and
    /// a filter that says nothing about bodies, both still read.
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "is_json_kind")]
    pub body_kind: crate::http::BodyKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<crate::http::FormField>,
}

fn is_json_kind(kind: &crate::http::BodyKind) -> bool {
    *kind == crate::http::BodyKind::Json
}

impl Default for LoadedEndpoint {
    fn default() -> Self {
        Self {
            method: String::new(),
            path: String::new(),
            name: String::new(),
            description: String::new(),
            tag: String::new(),
            meta: BTreeMap::new(),
            parameters: Vec::new(),
            body: String::new(),
            body_kind: crate::http::BodyKind::Json,
            form: Vec::new(),
        }
    }
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

/// What the last run found, as `<id>.json`: everything a listing, a search or
/// an access decision needs, and nothing else.
///
/// Schemas used to live in this file too, which put them in the path of every
/// MCP call — `list_sections` needs an endpoint count, and got a 2.5 GB file to
/// parse for it. They are in [`LoaderSchemas`] now, read only when one is
/// asked for. A file written before the split still reads: its `schemas` and
/// `responseSchemas` are skipped rather than kept, since what they hold is the
/// unbounded expansion this format exists to avoid.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoaderCache {
    /// Epoch millis of the last successful run.
    pub loaded_at: i64,
    pub endpoints: Vec<LoadedEndpoint>,
}

/// Request and response schemas for a loader's endpoints, as
/// `<id>.schemas.json`.
///
/// Stored the way the document wrote them, `$ref`s and all, with each
/// definition those refer to held once under `definitions`. An endpoint's
/// schema is made self-contained only when it is asked for — see
/// [`crate::openapi::bundle`] — so the file stays the size of the document,
/// not of its expansion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoaderSchemas {
    /// Keyed by the `$ref` that names each one, e.g.
    /// `#/components/schemas/User`.
    #[serde(default)]
    pub definitions: BTreeMap<String, serde_json::Value>,
    /// Request-body schemas, keyed by endpoint id.
    #[serde(default)]
    pub request: BTreeMap<String, serde_json::Value>,
    /// Successful-response schemas, keyed the same way.
    #[serde(default)]
    pub response: BTreeMap<String, serde_json::Value>,
}

impl LoaderSchemas {
    /// One endpoint's request schema, ready to validate against.
    pub fn request_for(&self, key: &str) -> Option<serde_json::Value> {
        self.request
            .get(key)
            .map(|schema| crate::openapi::bundle(schema, &self.definitions))
    }

    /// One endpoint's response schema, ready to validate against.
    pub fn response_for(&self, key: &str) -> Option<serde_json::Value> {
        self.response
            .get(key)
            .map(|schema| crate::openapi::bundle(schema, &self.definitions))
    }

    fn extend(&mut self, other: LoaderSchemas) {
        self.definitions.extend(other.definitions);
        self.request.extend(other.request);
        self.response.extend(other.response);
    }
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

/// A cancellation handle no other request can collide with.
///
/// `HttpState` keys in-flight requests by `RequestSpec::id`, and inserting a
/// second one under a key already there drops the first's cancel sender —
/// which *is* the cancel signal. Every loader request for a section used the
/// same id, so any two that overlapped killed each other: a "Fetch a sample"
/// while a background refresh was out came back "request cancelled", and which
/// of the two died depended on timing.
///
/// Nothing cancels a loader request by id — only the window does that, for
/// requests a person sent — so the id has no reason to be predictable.
pub fn request_id(section_id: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "loader:{section_id}:{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A request the loader makes, before the section's base URL and auth apply.
#[derive(Debug, Clone)]
pub struct LoaderRequest {
    pub url: String,
    pub method: String,
}

#[derive(Debug, Clone, Default)]
pub struct LoaderResponse {
    pub status: u16,
    pub body: String,
    /// The absolute URL the request was aimed at, once the section's base URL
    /// had been applied. The loader itself only ever holds the relative form,
    /// so the comparison below has to be made against what actually went out.
    pub requested_url: String,
    /// Where the response actually came from, after any redirects. Reported on
    /// a rejection when it left the origin the request was aimed at: a Cookie
    /// or Authorization credential is dropped on a cross-host hop, so "403"
    /// and "403, and by the way you ended up somewhere else" are different
    /// problems with the same status.
    pub final_url: String,
}

/// How the host performs the loader's request. Injected so this module stays
/// free of both Tauri and the HTTP stack.
pub type Fetcher = Arc<
    dyn Fn(
            LoaderRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<LoaderResponse, String>> + Send>,
        > + Send
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
    // The bare status was unactionable: 401 and 403 are what an expired or
    // rejected credential looks like, and the API almost always says which in
    // the body. Carrying a snippet of it is the difference between "returned
    // 403" and "returned 403: CSRF token missing".
    #[error("the manifest request returned {status}{}", detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    Status { status: u16, detail: Option<String> },
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
    // The section file could not be read; its message already names the file.
    #[error("{0}")]
    Section(String),
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

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
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
        .load(
            &arena,
            File {
                code: query,
                path: (),
            },
        )
        .map_err(|errors| LoaderError::BadQuery(describe_load(errors)))?;

    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errors| LoaderError::BadQuery(describe_compile(errors)))?;

    // jaq's value type bridges to serde_json through JSON text, which keeps us
    // off its internal representation.
    let encoded =
        serde_json::to_string(input).map_err(|err| LoaderError::NotJson(err.to_string()))?;
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
    let mut identities = HashSet::with_capacity(items.len());
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
        let endpoint = endpoint.tidy();
        let key = endpoint.key();
        if !identities.insert(key.clone()) {
            return Err(LoaderError::BadShape(format!(
                "item {index} duplicates endpoint `{key}`"
            )));
        }
        endpoints.push(endpoint);
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
) -> Result<(Vec<LoadedEndpoint>, LoaderSchemas, usize), LoaderError> {
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
        let mut schemas = LoaderSchemas::default();
        let mut identities = HashSet::new();
        let mut pages = 0;

        loop {
            let document = fetch_json(&fetcher, &url, &method).await?;
            let mut mapped = to_endpoints(&apply(&config.query, &document)?)?;
            // Adjacent API pages commonly overlap by one item. Identity is
            // METHOD/path, so keep the first rather than emitting duplicate
            // keyed rows into the sidebar and overlay.
            mapped.retain(|endpoint| identities.insert(endpoint.key()));
            schemas.extend(enrich_openapi(&document, &mut mapped));
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

        Ok((endpoints, schemas, pages))
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
        return Err(rejected(&response));
    }
    serde_json::from_str(&response.body).map_err(|err| LoaderError::NotJson(err.to_string()))
}

/// The error for a non-2xx manifest response, body and redirect included.
pub(crate) fn rejected(response: &LoaderResponse) -> LoaderError {
    let detail = match (
        detail_from(&response.body),
        redirect_note(&response.requested_url, &response.final_url),
    ) {
        (Some(body), Some(note)) => Some(format!("{body} ({note})")),
        (Some(body), None) => Some(body),
        (None, note) => note,
    };

    LoaderError::Status {
        status: response.status,
        detail,
    }
}

/// "redirected to <origin>", when the response came from somewhere else.
///
/// Compared by origin rather than by whole URL: following a redirect within the
/// same host is ordinary and says nothing, while leaving the host is the thing
/// that silently drops the credential.
fn redirect_note(requested: &str, final_url: &str) -> Option<String> {
    if final_url.trim().is_empty() {
        return None;
    }
    let (from, to) = (origin_of(requested)?, origin_of(final_url)?);
    (from != to).then(|| format!("redirected to {to}, which the credential is not sent to"))
}

fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    Some(format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default()
    ))
}

/// How much of a rejected manifest body is worth showing.
const DETAIL_LIMIT: usize = 200;

/// A one-line summary of an error body, for the message a person reads.
///
/// APIs answer a rejected request with anything from `{"detail": "..."}` to a
/// whole HTML login page. The first is the answer; the second is noise, and its
/// only useful content is that it *is* a login page — which the status already
/// said. So: pull the usual message fields out of JSON, and otherwise fall back
/// to a flattened, clipped prefix.
pub(crate) fn detail_from(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        // The fields every framework reaches for, in the order they tend to
        // carry the most specific message.
        for key in ["detail", "message", "error_description", "error", "title"] {
            match value.get(key) {
                Some(serde_json::Value::String(found)) if !found.trim().is_empty() => {
                    return Some(clip(found));
                }
                _ => {}
            }
        }
    }

    Some(clip(body))
}

fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(DETAIL_LIMIT) {
        Some((at, _)) => format!("{}…", &flat[..at]),
        None => flat,
    }
}

/// Fills in bodies, tags, descriptions, parameters and schemas when the
/// manifest turns out to be OpenAPI.
///
/// A jq filter maps a document to endpoints, and that is all it should do:
/// walking a JSON Schema into an example is not something anyone should have to
/// write in jq. So the rest comes from here instead, off the same document the
/// filter just read — which means a collection filled by a loader and one
/// filled by importing the file end up with the same endpoints.
///
/// Anything else — a routes array, a bespoke manifest — has no `paths`, so this
/// does nothing and every endpoint keeps what the filter produced.
fn enrich_openapi(document: &serde_json::Value, endpoints: &mut [LoadedEndpoint]) -> LoaderSchemas {
    let Some(paths) = document.get("paths").and_then(|paths| paths.as_object()) else {
        return LoaderSchemas::default();
    };

    let mut schemas = LoaderSchemas::default();

    for endpoint in endpoints {
        let Some(item) = paths.get(&endpoint.path).and_then(|item| item.as_object()) else {
            continue;
        };
        let Some(operation) = item
            .get(&endpoint.method.to_ascii_lowercase())
            .and_then(|operation| operation.as_object())
        else {
            continue;
        };

        if endpoint.description.is_empty() {
            endpoint.description = operation
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
        }
        if endpoint.tag.is_empty() {
            endpoint.tag = crate::openapi::first_tag(Some(operation));
        }
        // The filter had the same document and got there first, so a key it
        // set stands. Anything it didn't mention comes off the operation.
        for (name, value) in crate::openapi::extensions(Some(operation)) {
            endpoint.meta.entry(name).or_insert(value);
        }
        if endpoint.parameters.is_empty() {
            endpoint.parameters = crate::openapi::operation_params(document, item, Some(operation));
        }

        let (body, body_kind, form) = crate::openapi::request_payload(document, operation);
        if endpoint.body.is_empty() && endpoint.form.is_empty() {
            endpoint.body = body;
            endpoint.body_kind = body_kind;
            endpoint.form = form;
        }
        if let Some(schema) = crate::openapi::request_schema(document, operation) {
            crate::openapi::collect_definitions(document, schema, &mut schemas.definitions);
            schemas.request.insert(endpoint.key(), schema.clone());
        }
        if let Some(schema) = crate::openapi::response_schema(document, operation) {
            crate::openapi::collect_definitions(document, schema, &mut schemas.definitions);
            schemas.response.insert(endpoint.key(), schema.clone());
        }
    }

    schemas
}

/// `<app data>/loaders`
pub fn loaders_dir(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join("loaders")
}

fn cache_path(dir: &std::path::Path, section_id: &str) -> Option<std::path::PathBuf> {
    // Same guard as section files: an id becomes a file name. It also keeps
    // the two names below apart, since an id cannot contain a dot.
    crate::store::is_safe_id(section_id).then(|| dir.join(format!("{section_id}.json")))
}

pub fn schemas_path(dir: &std::path::Path, section_id: &str) -> Option<std::path::PathBuf> {
    crate::store::is_safe_id(section_id).then(|| dir.join(format!("{section_id}.schemas.json")))
}

/// The last successful run. Loader output is a cache, never the source of
/// truth, so a missing or unreadable file is simply "nothing loaded yet".
///
/// Streamed, and stops as soon as it has both fields. A file written before
/// schemas moved out holds its gigabytes *after* the endpoint list — the
/// struct wrote its fields in order — so this reads the few hundred kilobytes
/// in front and never the rest, where parsing the whole file took long enough
/// to look like a hang.
pub fn read_cache(dir: &std::path::Path, section_id: &str) -> Option<LoaderCache> {
    let file = std::fs::File::open(cache_path(dir, section_id)?).ok()?;
    let mut found = None;
    let mut reader =
        serde_json::Deserializer::from_reader(std::io::BufReader::with_capacity(1 << 16, file));
    // Stopping early leaves the object unclosed, which serde_json reports as
    // an error once the visitor returns. That error is expected; whether the
    // fields were found is what `found` says.
    let _ = serde::Deserializer::deserialize_map(&mut reader, CacheHead(&mut found));
    found
}

/// Reads `loadedAt` and `endpoints`, and stops.
struct CacheHead<'a>(&'a mut Option<LoaderCache>);

impl<'de> serde::de::Visitor<'de> for CacheHead<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a loader cache")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let (mut loaded_at, mut endpoints) = (None, None);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "loadedAt" => loaded_at = Some(map.next_value()?),
                "endpoints" => endpoints = Some(map.next_value()?),
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            if loaded_at.is_some() && endpoints.is_some() {
                break;
            }
        }
        if let (Some(loaded_at), Some(endpoints)) = (loaded_at, endpoints) {
            *self.0 = Some(LoaderCache {
                loaded_at,
                endpoints,
            });
        }
        Ok(())
    }
}

/// The schemas from the last successful run, when there are any.
pub fn read_schemas(dir: &std::path::Path, section_id: &str) -> Option<LoaderSchemas> {
    let file = std::fs::File::open(schemas_path(dir, section_id)?).ok()?;
    serde_json::from_reader(std::io::BufReader::with_capacity(1 << 16, file)).ok()
}

/// Writes both halves of a run. Schemas first: a reader that sees the new
/// endpoint list then finds schemas at least as new, and a schema for an
/// endpoint that is not listed yet is never asked for.
pub fn write_cache(
    dir: &std::path::Path,
    section_id: &str,
    cache: &LoaderCache,
    schemas: &LoaderSchemas,
) -> std::io::Result<()> {
    let (Some(path), Some(schemas_path)) =
        (cache_path(dir, section_id), schemas_path(dir, section_id))
    else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;

    // Compact: this half is never read by a person, and indenting a deeply
    // nested schema costs more than the schema.
    let encoded = serde_json::to_vec(schemas)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    write_atomic(&schemas_path, &encoded)?;

    let encoded = serde_json::to_vec_pretty(cache)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    write_atomic(&path, &encoded)
}

/// Same write discipline as section files — temp, sync, rename — and the same
/// per-process temp name, because the app and a headless `fiber mcp` can both
/// refresh the same loader.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".tmp-{}", std::process::id()));
    let temp = std::path::PathBuf::from(temp);
    let mut file = std::fs::File::create(&temp)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, path)
}

pub fn forget_cache(dir: &std::path::Path, section_id: &str) {
    for path in [cache_path(dir, section_id), schemas_path(dir, section_id)]
        .into_iter()
        .flatten()
    {
        let _ = std::fs::remove_file(path);
    }
}

/// What changed between two runs, by stable key.
///
/// Keyed lookups rather than `Vec::contains`: a manifest of several hundred
/// endpoints made the old pairwise scan quadratic, run in full on every
/// refresh.
pub fn diff(previous: &[LoadedEndpoint], next: &[LoadedEndpoint]) -> (Vec<String>, Vec<String>) {
    let before: std::collections::HashSet<String> =
        previous.iter().map(LoadedEndpoint::key).collect();
    let after: std::collections::HashSet<String> = next.iter().map(LoadedEndpoint::key).collect();

    let added = after.difference(&before).cloned().collect();
    let removed = before.difference(&after).cloned().collect();
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
                    ..Default::default()
                })
            })
        })
    }

    #[tokio::test]
    async fn maps_a_route_manifest() {
        let routes = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "Array of routes")
            .unwrap()
            .1;
        let (endpoints, _schemas, pages) = run(
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

        let query = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "OpenAPI")
            .unwrap()
            .1;
        let mut endpoints = to_endpoints(&apply(query, &document).unwrap()).unwrap();
        endpoints.sort_by_key(|endpoint| endpoint.key());

        let keys: Vec<String> = endpoints.iter().map(LoadedEndpoint::key).collect();
        assert_eq!(keys, vec!["GET /users", "GET /users/{id}", "POST /users"]);
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

        let query = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "Skip deprecated")
            .unwrap()
            .1;
        let endpoints = to_endpoints(&apply(query, &document).unwrap()).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].path, "/live");
    }

    #[test]
    fn a_missing_name_falls_back_to_the_path() {
        let document = json!([{ "method": "GET", "path": "/thing" }]);
        let endpoints = to_endpoints(&apply("map({method, path})", &document).unwrap()).unwrap();
        assert_eq!(endpoints[0].name, "/thing");
    }

    #[test]
    fn duplicate_endpoint_identities_are_rejected() {
        let document = json!([
            { "method": "get", "path": "/thing", "name": "first" },
            { "method": "GET", "path": "/thing", "name": "second" }
        ]);

        match to_endpoints(&document) {
            Err(LoaderError::BadShape(message)) => {
                assert!(message.contains("duplicates endpoint `GET /thing`"));
            }
            other => panic!("expected a duplicate identity error, got {other:?}"),
        }
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
                    ..Default::default()
                })
            })
        });

        let mut settings = config(".routes | map({method: .verb, path: .url})");
        settings.next = ".links.next".to_string();

        let (endpoints, _schemas, pages) = run(&settings, fetcher).await.unwrap();
        assert_eq!(pages, 2);
        assert_eq!(
            endpoints
                .iter()
                .map(LoadedEndpoint::key)
                .collect::<Vec<_>>(),
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
                    ..Default::default()
                })
            })
        });

        let mut settings = config(".routes | map({method: .verb, path: .url})");
        settings.next = ".links.next".to_string();

        let (endpoints, _schemas, pages) = run(&settings, fetcher).await.unwrap();
        assert_eq!(pages, MAX_PAGES);
        // The cap still stops the bad pointer, while overlapping pages no longer
        // create 50 sidebar rows with the same endpoint identity.
        assert_eq!(endpoints.len(), 1);
    }

    /// A new loader keeps itself current without being told to. 0 still means
    /// "only when asked" — it just is not the default any more.
    #[test]
    fn a_new_loader_refreshes_itself() {
        let config = LoaderConfig::default();
        assert!(config.enabled);
        assert!(
            config.ttl_seconds > 0,
            "a default of 0 would mean a new loader never refreshed on its own"
        );
    }

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

        let openapi = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "OpenAPI")
            .unwrap()
            .1;
        let (endpoints, schemas, _) = run(&config(openapi), answering(manifest)).await.unwrap();

        let post = endpoints.iter().find(|e| e.method == "POST").unwrap();
        // Type names rather than empty values — the editor turns these into
        // fields you tab through.
        assert!(post.body.contains("\"offset\": number"), "{}", post.body);
        assert!(
            post.body.contains("\"messageIndex\": number"),
            "{}",
            post.body
        );
        assert!(post.body.contains("\"dryRun\": boolean"), "{}", post.body);
        assert_eq!(
            schemas
                .request_for("POST /activity/backfill-activity")
                .as_ref()
                .and_then(|schema| schema.pointer("/properties/dryRun/type"))
                .and_then(|kind| kind.as_str()),
            Some("boolean")
        );

        // A GET declares no body, and gets none.
        let get = endpoints.iter().find(|e| e.method == "GET").unwrap();
        assert_eq!(get.body, "");
    }

    /// The reason `meta` exists: an API where the HTTP method says nothing puts
    /// what it does say in an extension, and Fiber has to carry it without
    /// knowing what it means.
    #[tokio::test]
    async fn operation_extensions_survive_into_the_cache() {
        let manifest = r##"{
            "openapi": "3.1.1",
            "paths": {
                "/customers/search": {
                    "post": { "operationId": "searchCustomers", "x-kind": "query" }
                },
                "/orders": {
                    "post": {
                        "operationId": "createOrder",
                        "x-kind": "command",
                        "x-internal": true,
                        "x-owner": { "team": "billing" }
                    }
                }
            }
        }"##;

        let openapi = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "OpenAPI")
            .unwrap()
            .1;
        let (endpoints, _, _) = run(&config(openapi), answering(manifest)).await.unwrap();

        let search = endpoints
            .iter()
            .find(|e| e.path == "/customers/search")
            .unwrap();
        assert_eq!(search.meta.get("x-kind"), Some(&serde_json::json!("query")));

        let orders = endpoints.iter().find(|e| e.path == "/orders").unwrap();
        assert_eq!(
            orders.meta.get("x-kind"),
            Some(&serde_json::json!("command"))
        );
        // Any scalar, not only strings — a policy can compare against `true`.
        assert_eq!(
            orders.meta.get("x-internal"),
            Some(&serde_json::json!(true))
        );
        // An object-valued extension is skipped: this ends up in a cache and in
        // every search result, so it is not a place to carry a document.
        assert!(!orders.meta.contains_key("x-owner"));
        // And nothing that isn't an extension leaks in beside them.
        assert!(!orders.meta.contains_key("operationId"));
    }

    /// A jq filter that already filled in a body used to skip schema extraction
    /// entirely. The body stays as the filter wrote it; the schema still lands
    /// in the cache so the editor can validate against the document.
    #[tokio::test]
    async fn a_filter_supplied_body_still_gets_a_schema() {
        let manifest = r##"{
            "openapi": "3.0.0",
            "paths": {
                "/flags": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "required": ["enabled"],
                                        "properties": { "enabled": { "type": "boolean" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"##;

        let settings = config(
            r#".paths | to_entries | map(.key as $path | .value | to_entries | map({method: .key, path: $path, body: "{\"enabled\": true}"})) | flatten"#,
        );
        let (endpoints, schemas, _) = run(&settings, answering(manifest)).await.unwrap();

        let post = endpoints.iter().find(|e| e.method == "POST").unwrap();
        assert_eq!(post.body, "{\"enabled\": true}");
        assert_eq!(
            schemas
                .request_for("POST /flags")
                .as_ref()
                .and_then(|schema| schema.pointer("/properties/enabled/type"))
                .and_then(|kind| kind.as_str()),
            Some("boolean")
        );
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fiber-loader-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The regression this file layout exists for. Schemas that refer to each
    /// other in a ring, shared by many endpoints, used to be expanded along
    /// every path through the ring and stored once per endpoint: this 20 KB
    /// document made a cache of over 200 MB, and a real one made 2.5 GB that
    /// every MCP call then had to parse.
    #[tokio::test]
    async fn a_cyclic_spec_makes_a_cache_the_size_of_the_spec() {
        let spec = include_str!("../tests/fixtures/cyclic-union.json");
        let openapi = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "OpenAPI")
            .unwrap()
            .1;
        let (endpoints, schemas, _) = run(&config(openapi), answering(spec)).await.unwrap();
        assert_eq!(endpoints.len(), 21);

        let dir = scratch("cyclic");
        let cache = LoaderCache {
            loaded_at: 1,
            endpoints,
        };
        write_cache(&dir, "sec-1", &cache, &schemas).unwrap();

        let size = |name: &str| std::fs::metadata(dir.join(name)).unwrap().len();
        // The endpoint list is all a listing reads. It holds no schema, and
        // each body built from one is bounded — 440 KB apiece here, before.
        assert!(size("sec-1.json") < 1_000_000, "{}", size("sec-1.json"));
        // Every definition once, plus one `$ref` per endpoint and direction.
        assert!(
            size("sec-1.schemas.json") < spec.len() as u64,
            "{}",
            size("sec-1.schemas.json")
        );

        let read = read_cache(&dir, "sec-1").unwrap();
        assert_eq!(read.endpoints.len(), 21);
        let schemas = read_schemas(&dir, "sec-1").unwrap();
        let key = "POST /data-points/concept7/define";
        for bundled in [schemas.request_for(key), schemas.response_for(key)] {
            let bundled = bundled.expect("a schema for both directions");
            assert!(bundled["$defs"]["Operand"]["oneOf"].is_array(), "{bundled}");
            assert!(serde_json::to_string(&bundled).unwrap().len() < 8_000);
        }
        // A body still comes out of the same schema.
        let post = read.endpoints.iter().find(|e| e.key() == key).unwrap();
        assert!(post.body.contains("\"kind\": \"step0\""), "{}", post.body);
        assert!(post.body.len() < 48_000, "{}", post.body.len());

        forget_cache(&dir, "sec-1");
        assert!(read_cache(&dir, "sec-1").is_none());
        assert!(read_schemas(&dir, "sec-1").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A cache written before schemas moved to their own file still lists its
    /// endpoints. What it held under `schemas` is the unbounded expansion, so
    /// it is not served — and not even read: the endpoint list comes first,
    /// and reading stops there.
    #[test]
    fn a_cache_from_before_the_split_is_read_only_as_far_as_its_endpoints() {
        let dir = scratch("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        // Everything after the endpoint list stands in for 2.5 GB: if it were
        // parsed, it would fail.
        let legacy = r#"{
  "loadedAt": 7,
  "endpoints": [{ "method": "POST", "path": "/define", "name": "define" }],
  "schemas": { "POST /define": { "oneOf": [ this is never read"#;
        std::fs::write(dir.join("sec-1.json"), legacy).unwrap();

        let read = read_cache(&dir, "sec-1").unwrap();
        assert_eq!(read.loaded_at, 7);
        assert_eq!(read.endpoints[0].key(), "POST /define");
        assert!(read_schemas(&dir, "sec-1").is_none());

        // Missing either field is still "nothing loaded", not half a cache.
        std::fs::write(dir.join("sec-1.json"), r#"{"endpoints": []}"#).unwrap();
        assert!(read_cache(&dir, "sec-1").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A manifest that is not OpenAPI has no `paths`, so nothing is derived and
    /// nothing breaks.
    #[tokio::test]
    async fn a_routes_manifest_is_left_alone() {
        let routes = TEMPLATES
            .iter()
            .find(|(name, _)| *name == "Array of routes")
            .unwrap()
            .1;
        let (endpoints, _schemas, _) = run(
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
                    ..Default::default()
                })
            })
        });
        assert!(matches!(
            run(&config(DEFAULT_QUERY), forbidden).await,
            Err(LoaderError::Status {
                status: 403,
                detail: None
            })
        ));
    }

    /// The whole point of carrying a body: "returned 403" is unactionable and
    /// "returned 403: CSRF token missing" is not.
    #[tokio::test]
    async fn a_rejected_manifest_explains_itself() {
        let forbidden: Fetcher = Arc::new(|_| {
            Box::pin(async {
                Ok(LoaderResponse {
                    status: 403,
                    body: r#"{"detail": "CSRF token missing"}"#.to_string(),
                    ..Default::default()
                })
            })
        });

        let message = run(&config(DEFAULT_QUERY), forbidden).await.unwrap_err();
        assert_eq!(
            message.to_string(),
            "the manifest request returned 403: CSRF token missing"
        );
    }

    #[test]
    fn a_login_page_is_flattened_rather_than_dumped() {
        let html = format!("<html>\n  <body>{}</body>\n</html>", "sign in ".repeat(80));
        let detail = detail_from(&html).unwrap();
        assert!(
            !detail.contains('\n'),
            "newlines make a one-line message two"
        );
        assert!(detail.ends_with('…'), "a long body is clipped: {detail}");
        assert!(detail.chars().count() <= DETAIL_LIMIT + 1);
    }

    #[test]
    fn an_empty_body_adds_nothing() {
        assert_eq!(detail_from("   "), None);
    }

    /// Leaving the origin is what silently drops a Cookie or Authorization
    /// credential, so a 403 that arrived from somewhere else has to say so —
    /// otherwise it is indistinguishable from the API refusing you outright.
    #[test]
    fn a_rejection_after_a_cross_host_redirect_says_where_it_ended_up() {
        let response = LoaderResponse {
            status: 403,
            body: r#"{"message": "Token is empty"}"#.into(),
            requested_url: "https://staging.example.com/openapi.json".into(),
            final_url: "https://login.example.com/signin".into(),
        };

        assert_eq!(
            rejected(&response).to_string(),
            "the manifest request returned 403: Token is empty (redirected to \
             https://login.example.com, which the credential is not sent to)"
        );
    }

    /// A redirect that stays put is ordinary and says nothing worth saying.
    #[test]
    fn a_same_origin_redirect_is_not_worth_mentioning() {
        let response = LoaderResponse {
            status: 403,
            body: String::new(),
            requested_url: "https://api.example.com/openapi.json".into(),
            final_url: "https://api.example.com/v2/openapi.json".into(),
        };

        assert_eq!(
            rejected(&response).to_string(),
            "the manifest request returned 403"
        );
    }

    /// Two loader requests must not share a cancellation handle: inserting the
    /// second under the first's key drops its sender, which reads as a cancel.
    #[test]
    fn every_loader_request_gets_its_own_cancel_handle() {
        let first = request_id("sec-1");
        let second = request_id("sec-1");
        assert_ne!(first, second);
        assert!(first.starts_with("loader:sec-1:"));
    }

    #[tokio::test]
    async fn reports_a_response_that_is_not_json() {
        let html: Fetcher = Arc::new(|_| {
            Box::pin(async {
                Ok(LoaderResponse {
                    status: 200,
                    body: "<html>login</html>".to_string(),
                    ..Default::default()
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
        assert!(matches!(
            apply("   ", &json!({})),
            Err(LoaderError::NoQuery)
        ));
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
                    assert!(
                        message.contains(expected),
                        "{message} should mention {expected}"
                    );
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
                ..Default::default()
            },
            LoadedEndpoint {
                method: "GET".into(),
                path: "/gone".into(),
                name: "gone".into(),
                description: String::new(),
                body: String::new(),
                ..Default::default()
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
                ..Default::default()
            },
            LoadedEndpoint {
                method: "POST".into(),
                path: "/new".into(),
                name: "new".into(),
                description: String::new(),
                body: String::new(),
                ..Default::default()
            },
        ];

        let (added, removed) = diff(&before, &after);
        assert_eq!(added, vec!["POST /new"]);
        assert_eq!(removed, vec!["GET /gone"]);
    }
}
