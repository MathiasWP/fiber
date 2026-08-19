//! Dynamic endpoint loaders.
//!
//! A section can define a script that returns its endpoint list, for APIs that
//! publish their own route manifest. The script runs in an embedded QuickJS
//! interpreter with exactly two capabilities: `fetch`, routed through the same
//! authenticated request path as everything else in the section, and `console`.
//!
//! No filesystem, no process, no environment, no network except through that
//! `fetch`. This is a small *capability* surface, not a hardened sandbox — see
//! §6 of the design doc for the honest scope of that claim.
//!
//! QuickJS rather than embedding V8 because loaders must run headlessly for the
//! MCP server, and a script that makes one HTTP call and maps an array doesn't
//! justify V8's binary size or build time.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{AsyncContext, AsyncRuntime, Function, Value};
use serde::{Deserialize, Serialize};

/// Wall-clock ceiling for one run, and the memory the interpreter may claim.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoaderConfig {
    #[serde(default)]
    pub enabled: bool,
    /// JavaScript defining `async function load()`.
    #[serde(default)]
    pub source: String,
    /// 0 means "only when asked".
    #[serde(default)]
    pub ttl_seconds: u64,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: DEFAULT_SOURCE.to_string(),
            ttl_seconds: 0,
        }
    }
}

pub const DEFAULT_SOURCE: &str = r#"// Return this section's endpoints.
// `fetch` uses the section's base URL and auth, so a path is enough.
async function load() {
  const response = await fetch("/internal/endpoints");
  const { routes } = await response.json();

  return routes.map((route) => ({
    method: route.verb,
    path: route.url,
    name: route.handler
  }));
}
"#;

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
    pub logs: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub loaded_at: i64,
}

/// A request a loader made, before the section's base URL and auth are applied.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderRequest {
    pub path: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// How the host performs a loader's `fetch`. Injected so this module stays free
/// of both Tauri and the HTTP stack, and so tests can answer without a socket.
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
    #[error("loader is empty")]
    Empty,
    #[error("loader took longer than {}s", RUN_TIMEOUT.as_secs())]
    Timeout,
    #[error("loader did not define `async function load()`")]
    NoEntryPoint,
    #[error("{0}")]
    Script(String),
    #[error("loader returned something other than a list of endpoints: {0}")]
    BadShape(String),
    #[error("could not start the loader: {0}")]
    Engine(String),
}

impl Serialize for LoaderError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Everything the loader is allowed to touch. Values cross the boundary as JSON
/// strings, which keeps the conversion surface to one type and avoids fighting
/// the interpreter's marshalling.
const PRELUDE: &str = r#"
globalThis.console = {
  log: (...parts) => __hostLog(parts.map((part) =>
    typeof part === "string" ? part : JSON.stringify(part)).join(" ")),
};
globalThis.console.info = globalThis.console.log;
globalThis.console.warn = globalThis.console.log;
globalThis.console.error = globalThis.console.log;

globalThis.fetch = async (path, init) => {
  const options = init || {};
  const headers = options.headers
    ? Object.entries(options.headers).map(([name, value]) => [name, String(value)])
    : [];

  const raw = await __hostFetch(JSON.stringify({
    path: String(path),
    method: options.method || "GET",
    headers,
    body: options.body === undefined || options.body === null ? null : String(options.body),
  }));

  const outcome = JSON.parse(raw);
  if (!outcome.ok) throw new Error(outcome.error);

  const response = outcome.response;
  return {
    status: response.status,
    ok: response.status >= 200 && response.status < 300,
    headers: response.headers,
    text: async () => response.body,
    json: async () => JSON.parse(response.body),
  };
};

globalThis.__run = async () => {
  if (typeof load !== "function") return "__NO_ENTRY__";
  return JSON.stringify(await load());
};
"#;

/// Runs a loader and returns the endpoints it reported, plus anything it logged.
pub async fn run(
    source: &str,
    fetcher: Fetcher,
) -> Result<(Vec<LoadedEndpoint>, Vec<String>), LoaderError> {
    run_within(source, fetcher, RUN_TIMEOUT).await
}

/// The limit is a parameter so tests can prove a runaway loader is stopped
/// without waiting out the real one.
pub async fn run_within(
    source: &str,
    fetcher: Fetcher,
    limit: Duration,
) -> Result<(Vec<LoadedEndpoint>, Vec<String>), LoaderError> {
    if source.trim().is_empty() {
        return Err(LoaderError::Empty);
    }

    // QuickJS runs JavaScript, so types come off first. Plain JS passes through
    // unchanged, being a subset.
    let source = &strip_types(source)?;

    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let collected = logs.clone();

    let outcome = tokio::time::timeout(limit, async move {
        let runtime = AsyncRuntime::new().map_err(|err| LoaderError::Engine(err.to_string()))?;
        runtime.set_memory_limit(MEMORY_LIMIT).await;

        // Without this a `while (true)` in a loader would wedge the interpreter
        // for good; the outer timeout can't interrupt a running script.
        let deadline = Instant::now() + limit;
        runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)))
            .await;

        let context = AsyncContext::full(&runtime)
            .await
            .map_err(|err| LoaderError::Engine(err.to_string()))?;

        let source = source.to_string();
        context
            .async_with(async |ctx| {
            let globals = ctx.globals();

            let sink = logs.clone();
            globals
                .set(
                    "__hostLog",
                    Function::new(ctx.clone(), move |line: String| {
                        let mut sink = sink.lock().unwrap();
                        // A runaway logger shouldn't grow without bound.
                        if sink.len() < 500 {
                            sink.push(line);
                        }
                    })
                    .map_err(|err| LoaderError::Engine(err.to_string()))?,
                )
                .map_err(|err| LoaderError::Engine(err.to_string()))?;

            let call = fetcher.clone();
            globals
                .set(
                    "__hostFetch",
                    // The outcome is encoded in the payload and thrown by the
                    // prelude, rather than returned as a Rust `Err`: rejecting a
                    // promise from a host future would mean marshalling an error
                    // into a JS exception, and there's nothing to gain from it.
                    rquickjs::function::Func::from(rquickjs::function::Async(move |raw: String| {
                        let call = call.clone();
                        async move {
                            let outcome = match serde_json::from_str::<LoaderRequest>(&raw) {
                                Ok(request) => call(request).await,
                                Err(err) => Err(err.to_string()),
                            };
                            match outcome {
                                Ok(response) => serde_json::json!({
                                    "ok": true,
                                    "response": response,
                                }),
                                Err(error) => serde_json::json!({
                                    "ok": false,
                                    "error": error,
                                }),
                            }
                            .to_string()
                        }
                    })),
                )
                .map_err(|err| LoaderError::Engine(err.to_string()))?;

            ctx.eval::<(), _>(PRELUDE)
                .map_err(|err| LoaderError::Engine(describe(&ctx, err)))?;
            ctx.eval::<(), _>(source.as_bytes())
                .map_err(|err| LoaderError::Script(describe(&ctx, err)))?;

            let run: Function = ctx
                .globals()
                .get("__run")
                .map_err(|err| LoaderError::Engine(err.to_string()))?;
            let promise: rquickjs::Promise = run
                .call(())
                .map_err(|err| LoaderError::Script(describe(&ctx, err)))?;
            let value: Value = promise
                .into_future::<Value>()
                .await
                .map_err(|err| LoaderError::Script(describe(&ctx, err)))?;

            let json = value
                .as_string()
                .ok_or(LoaderError::NoEntryPoint)?
                .to_string()
                .map_err(|err| LoaderError::Script(err.to_string()))?;

            if json == "__NO_ENTRY__" {
                return Err(LoaderError::NoEntryPoint);
            }
            Ok(json)
            })
            .await
    })
    .await
    .map_err(|_| LoaderError::Timeout)??;

    let endpoints = parse_endpoints(&outcome)?;
    let logs = collected.lock().unwrap().clone();
    Ok((endpoints, logs))
}

/// QuickJS exceptions carry their message on the context, not in the error.
fn describe(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    if matches!(err, rquickjs::Error::Exception) {
        let exception = ctx.catch();
        if let Some(exception) = exception.as_exception() {
            let message = exception.message().unwrap_or_default();
            return match exception.stack() {
                Some(stack) if !stack.trim().is_empty() => format!("{message}\n{stack}"),
                _ => message,
            };
        }
    }
    err.to_string()
}

/// Validates the shape a loader returned, with messages aimed at whoever wrote
/// the script rather than at whoever wrote this file.
fn parse_endpoints(json: &str) -> Result<Vec<LoadedEndpoint>, LoaderError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|err| LoaderError::BadShape(err.to_string()))?;

    let items = value.as_array().ok_or_else(|| {
        LoaderError::BadShape(format!("expected an array, got {}", kind_of(&value)))
    })?;

    let mut endpoints = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            LoaderError::BadShape(format!("item {index} is {}, not an object", kind_of(item)))
        })?;
        for required in ["method", "path"] {
            if !object.get(required).is_some_and(|v| v.is_string()) {
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

/// Strips TypeScript type annotations so loaders can be written in TS.
///
/// Done in Rust rather than the editor because the MCP server runs loaders
/// headlessly — a transpile that only happens in the UI would mean a loader
/// that works in the app and fails everywhere else.
pub fn strip_types(source: &str) -> Result<String, LoaderError> {
    use oxc::allocator::Allocator;
    use oxc::codegen::Codegen;
    use oxc::parser::Parser;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};

    let allocator = Allocator::default();
    let source_type = SourceType::ts();

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(error) = parsed.errors.first() {
        return Err(LoaderError::Script(error.to_string()));
    }

    let mut program = parsed.program;
    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    Transformer::new(
        &allocator,
        std::path::Path::new("loader.ts"),
        &TransformOptions::default(),
    )
    .build_with_scoping(scoping, &mut program);

    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answering(body: &'static str) -> Fetcher {
        Arc::new(move |request: LoaderRequest| {
            Box::pin(async move {
                Ok(LoaderResponse {
                    status: 200,
                    headers: vec![("content-type".into(), "application/json".into())],
                    // Echo the path back so tests can assert what was requested.
                    body: body.replace("{{path}}", &request.path),
                })
            })
        })
    }

    fn failing() -> Fetcher {
        Arc::new(|_| Box::pin(async { Err("could not connect".to_string()) }))
    }

    #[tokio::test]
    async fn runs_the_documented_loader_shape() {
        let source = r#"
            async function load() {
              const response = await fetch("/internal/endpoints");
              const { routes } = await response.json();
              return routes.map((route) => ({
                method: route.verb,
                path: route.url,
                name: route.handler
              }));
            }
        "#;

        let (endpoints, _) = run(
            source,
            answering(r#"{"routes":[{"verb":"get","url":"/user/42","handler":"getUser"}]}"#),
        )
        .await
        .unwrap();

        assert_eq!(endpoints.len(), 1);
        // Methods are normalised, so a loader returning "get" still keys the
        // same as a hand-written GET.
        assert_eq!(endpoints[0].method, "GET");
        assert_eq!(endpoints[0].path, "/user/42");
        assert_eq!(endpoints[0].name, "getUser");
        assert_eq!(endpoints[0].key(), "GET /user/42");
    }

    #[tokio::test]
    async fn the_loader_sees_the_path_it_asked_for() {
        let source = r#"
            async function load() {
              const response = await fetch("/where/am/i");
              const body = await response.json();
              return [{ method: "GET", path: body.seen }];
            }
        "#;

        let (endpoints, _) = run(source, answering(r#"{"seen":"{{path}}"}"#))
            .await
            .unwrap();
        assert_eq!(endpoints[0].path, "/where/am/i");
    }

    #[tokio::test]
    async fn captures_console_output() {
        let source = r#"
            async function load() {
              console.log("looking things up");
              console.log({ nested: true });
              return [];
            }
        "#;

        let (endpoints, logs) = run(source, answering("{}")).await.unwrap();
        assert!(endpoints.is_empty());
        assert_eq!(logs, vec!["looking things up", r#"{"nested":true}"#]);
    }

    #[tokio::test]
    async fn surfaces_a_failed_fetch_to_the_script() {
        // The loader can catch it, which is the point of reporting it as a
        // rejected promise rather than killing the run.
        let source = r#"
            async function load() {
              try {
                await fetch("/nope");
                return [{ method: "GET", path: "/unreachable" }];
              } catch (error) {
                return [{ method: "GET", path: "/failed", name: String(error) }];
              }
            }
        "#;

        let (endpoints, _) = run(source, failing()).await.unwrap();
        assert_eq!(endpoints[0].path, "/failed");
        assert!(endpoints[0].name.contains("could not connect"), "{:?}", endpoints[0].name);
    }

    #[tokio::test]
    async fn reports_a_missing_entry_point() {
        let outcome = run("const x = 1;", answering("{}")).await;
        assert!(matches!(outcome, Err(LoaderError::NoEntryPoint)), "{outcome:?}");
    }

    #[tokio::test]
    async fn reports_a_syntax_error_with_its_message() {
        let outcome = run("async function load( {", answering("{}")).await;
        match outcome {
            Err(LoaderError::Script(message)) => {
                assert!(!message.is_empty(), "an empty message helps nobody");
            }
            other => panic!("expected a script error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reports_a_thrown_error() {
        let source = r#"
            async function load() { throw new Error("the API moved"); }
        "#;
        match run(source, answering("{}")).await {
            Err(LoaderError::Script(message)) => assert!(message.contains("the API moved"), "{message}"),
            other => panic!("expected a script error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_output_that_is_not_endpoints() {
        let cases = [
            ("async function load() { return 42; }", "expected an array"),
            ("async function load() { return [1]; }", "not an object"),
            ("async function load() { return [{ path: '/x' }]; }", "`method`"),
            ("async function load() { return [{ method: 'GET' }]; }", "`path`"),
        ];

        for (source, expected) in cases {
            match run(source, answering("{}")).await {
                Err(LoaderError::BadShape(message)) => {
                    assert!(message.contains(expected), "{message} should mention {expected}");
                }
                other => panic!("expected a shape error for {source}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn an_endless_loop_is_interrupted_rather_than_hanging() {
        // Belt and braces on the interrupt handler: if this regresses, the test
        // suite hangs rather than failing, so keep the loop cheap to detect.
        let outcome = run_within(
            "async function load() { while (true) {} }",
            answering("{}"),
            Duration::from_millis(500),
        )
        .await;
        assert!(outcome.is_err(), "an endless loader must not run forever");
    }

    #[tokio::test]
    async fn has_no_filesystem_or_network_beyond_fetch() {
        let source = r#"
            async function load() {
              const reachable = [];
              for (const name of ["require", "process", "XMLHttpRequest", "WebSocket", "importScripts"]) {
                if (typeof globalThis[name] !== "undefined") reachable.push(name);
              }
              return reachable.map((name) => ({ method: "GET", path: "/" + name }));
            }
        "#;

        let (endpoints, _) = run(source, answering("{}")).await.unwrap();
        assert!(endpoints.is_empty(), "loader reached {endpoints:?}");
    }

    #[tokio::test]
    async fn loaders_can_be_typescript() {
        let source = r#"
            interface Route {
              verb: string;
              url: string;
            }

            async function load(): Promise<Array<{ method: string; path: string }>> {
              const response = await fetch("/internal/endpoints");
              const { routes } = (await response.json()) as { routes: Route[] };
              return routes.map((route: Route) => ({
                method: route.verb,
                path: route.url satisfies string,
              }));
            }
        "#;

        let (endpoints, _) = run(
            source,
            answering(r#"{"routes":[{"verb":"POST","url":"/typed"}]}"#),
        )
        .await
        .unwrap();

        assert_eq!(endpoints[0].key(), "POST /typed");
    }

    #[test]
    fn stripping_types_leaves_the_logic_alone() {
        let js = strip_types("const x: string = 'hi'; function f<T>(v: T): T { return v; }").unwrap();
        assert!(!js.contains(": string"), "{js}");
        assert!(!js.contains("<T>"), "{js}");
        assert!(js.contains("'hi'") || js.contains("\"hi\""), "{js}");
        assert!(js.contains("return v"), "{js}");
    }

    #[test]
    fn a_type_error_is_not_a_syntax_error() {
        // Types are erased, not checked — a loader that lies about a type still
        // runs, which is the same bargain every TS-to-JS transpile makes.
        assert!(strip_types("const n: number = 'actually a string';").is_ok());
    }

    #[test]
    fn reports_where_the_syntax_broke() {
        match strip_types("async function load( {") {
            Err(LoaderError::Script(message)) => assert!(!message.is_empty()),
            other => panic!("expected a script error, got {other:?}"),
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
            },
            LoadedEndpoint {
                method: "GET".into(),
                path: "/gone".into(),
                name: "gone".into(),
                description: String::new(),
            },
        ];
        let after = vec![
            LoadedEndpoint {
                // Renaming an endpoint is not a change of identity.
                method: "GET".into(),
                path: "/a".into(),
                name: "renamed".into(),
                description: String::new(),
            },
            LoadedEndpoint {
                method: "POST".into(),
                path: "/new".into(),
                name: "new".into(),
                description: String::new(),
            },
        ];

        let (added, removed) = diff(&before, &after);
        assert_eq!(added, vec!["POST /new"]);
        assert_eq!(removed, vec!["GET /gone"]);
    }
}
