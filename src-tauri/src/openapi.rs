//! Importing an OpenAPI or Swagger document.
//!
//! Loaders keep a section in step with a live API, which is the right default
//! when the API publishes its own manifest — but it needs the API to be up and
//! reachable, every time. An imported spec is the other half: a file you were
//! handed, read once, turned into ordinary saved requests that work on a plane.
//!
//! So this deliberately produces *requests*, not loader output. There's no
//! cache to go stale and nothing to refresh; after importing, the endpoints are
//! yours to edit like any you'd typed.
//!
//! Handles OpenAPI 3.x and Swagger 2.0, JSON or YAML.

use serde::{Deserialize, Serialize};

/// The HTTP methods an OpenAPI path item may define. Anything else in there is
/// metadata (`parameters`, `summary`, `$ref`, …), not an operation.
const OPERATIONS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedEndpoint {
    pub method: String,
    pub path: String,
    pub name: String,
    pub description: String,
    /// A JSON body to start from, or empty when the operation takes none.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    /// What the document calls itself, for naming the section.
    pub title: String,
    pub version: String,
    /// The first server URL, so the section's base URL can be filled in.
    pub base_url: String,
    pub endpoints: Vec<ImportedEndpoint>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("that file isn't JSON or YAML: {0}")]
    Unparseable(String),
    #[error("that doesn't look like an OpenAPI document — no `paths` section")]
    NotOpenApi,
    #[error("the document has `paths`, but no operations in it")]
    NoOperations,
}

impl Serialize for ImportError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Parses a spec. JSON is tried first since it's also valid YAML but the JSON
/// parser gives better errors for it.
pub fn parse(text: &str) -> Result<Import, ImportError> {
    let document: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(json_error) => serde_norway::from_str(text)
            .map_err(|yaml_error| ImportError::Unparseable(format!("{json_error}; {yaml_error}")))?,
    };

    let paths = document
        .get("paths")
        .and_then(|paths| paths.as_object())
        .ok_or(ImportError::NotOpenApi)?;

    let mut endpoints = Vec::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for (method, operation) in item {
            if !OPERATIONS.contains(&method.to_ascii_lowercase().as_str()) {
                continue;
            }
            let operation = operation.as_object();

            // `operationId` is the good name; a summary is the usual fallback;
            // failing both, the path itself is more use than nothing.
            let name = operation
                .and_then(|operation| {
                    operation
                        .get("operationId")
                        .or_else(|| operation.get("summary"))
                })
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| path.clone());

            let description = operation
                .and_then(|operation| operation.get("description"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();

            let body = operation
                .map(|operation| request_body(&document, operation))
                .unwrap_or_default();

            endpoints.push(ImportedEndpoint {
                method: method.to_ascii_uppercase(),
                path: path.clone(),
                name,
                description,
                body,
            });
        }
    }

    if endpoints.is_empty() {
        return Err(ImportError::NoOperations);
    }

    // Object key order isn't meaningful, so impose one the user can predict.
    endpoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.method.cmp(&b.method)));

    Ok(Import {
        title: string_at(&document, &["info", "title"]),
        version: string_at(&document, &["info", "version"]),
        base_url: base_url(&document),
        endpoints,
    })
}

/// How deep a schema is expanded before giving up. Specs self-reference — a
/// comment carrying replies of its own type — so this and the `$ref` trail are
/// what keep a skeleton finite.
const MAX_DEPTH: usize = 6;

/// A JSON body to start from, built from the operation's request schema.
///
/// An `example` written into the document always wins: it is what the author
/// chose to show. Failing that the schema is walked into a skeleton with one
/// placeholder per field, which is quicker to correct than an empty editor is
/// to fill.
///
/// JSON only. A form-encoded or binary body has no honest JSON skeleton, so
/// those are left empty rather than guessed at.
pub(crate) fn request_body(
    document: &serde_json::Value,
    operation: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let Some(media) = json_media(document, operation) else {
        return String::new();
    };

    let given = media.get("example").cloned().or_else(|| {
        media
            .get("examples")
            .and_then(|examples| examples.as_object())
            .and_then(|examples| examples.values().next())
            .and_then(|first| resolve(document, first).get("value").cloned())
    });

    // An example is real data and goes out as written. A skeleton is not: its
    // leaves are the *names of types*, which the editor turns into fields you
    // tab through. Only the second kind gets unwrapped below.
    let (value, derived) = match given {
        Some(value) => (value, false),
        None => match media.get("schema") {
            Some(schema) => (skeleton(document, schema, 0, &mut Vec::new()), true),
            None => return String::new(),
        },
    };

    if value.is_null() {
        return String::new();
    }

    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    if derived { unwrap_types(&text) } else { text }
}

/// Marks a leaf as the name of a type rather than a value.
///
/// A bare `number` is not valid JSON, so it cannot be carried through
/// `serde_json`. It travels as a string wearing this sentinel and is unquoted
/// on the way out, which keeps the whole skeleton one ordinary serialisation
/// rather than a second pretty-printer written by hand.
const TYPE_MARK: &str = "\u{1}type:";

fn type_name(name: &str) -> serde_json::Value {
    serde_json::Value::String(format!("{TYPE_MARK}{name}"))
}

/// Turns `"\u{1}type:number"` back into a bare `number`.
///
/// The mark is a control character, which `serde_json` escapes as `\u0001` on
/// the way out — so what appears in the text is the escape, and that is what
/// this matches. Nothing a person could type collides with it.
fn unwrap_types(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let opening = "\"\\u0001type:";

    while let Some(start) = rest.find(opening) {
        out.push_str(&rest[..start]);
        let after = &rest[start + opening.len()..];
        match after.find('"') {
            Some(end) => {
                out.push_str(&after[..end]);
                rest = &after[end + 1..];
            }
            // Unterminated is impossible from our own serialisation, but
            // truncating the body would be a poor way to find that out.
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The JSON media object for a request body, from either spec version. Both
/// versions end up exposing the schema under `schema`, which is all the caller
/// needs to know.
fn json_media<'a>(
    document: &'a serde_json::Value,
    operation: &'a serde_json::Map<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    // OpenAPI 3: `requestBody.content`, keyed by media type.
    if let Some(body) = operation.get("requestBody") {
        let content = resolve(document, body).get("content")?.as_object()?;
        return content.get("application/json").or_else(|| {
            content
                .iter()
                .find(|(kind, _)| kind.contains("json"))
                .map(|(_, value)| value)
        });
    }

    // Swagger 2: a parameter with `in: body`, holding the schema directly.
    operation
        .get("parameters")
        .and_then(|parameters| parameters.as_array())
        .and_then(|parameters| {
            parameters
                .iter()
                .map(|parameter| resolve(document, parameter))
                .find(|parameter| {
                    parameter.get("in").and_then(|value| value.as_str()) == Some("body")
                })
        })
}

/// Follows a local `$ref` to what it points at. Anything else — including a
/// `$ref` to another file, which we have no way to fetch — is returned as-is.
fn resolve<'a>(
    document: &'a serde_json::Value,
    value: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let Some(pointer) = value.get("$ref").and_then(|it| it.as_str()) else {
        return value;
    };
    let Some(rest) = pointer.strip_prefix("#/") else {
        return value;
    };

    let mut current = document;
    for segment in rest.split('/') {
        // JSON Pointer escapes, in the order the spec requires: `~1` first.
        let key = segment.replace("~1", "/").replace("~0", "~");
        match current.get(&key) {
            Some(next) => current = next,
            None => return value,
        }
    }
    current
}

/// One placeholder value per field, following `$ref`s but never in a circle.
fn skeleton(
    document: &serde_json::Value,
    schema: &serde_json::Value,
    depth: usize,
    trail: &mut Vec<String>,
) -> serde_json::Value {
    if depth > MAX_DEPTH {
        return serde_json::Value::Null;
    }

    // A `$ref` already on the trail is a cycle — stop rather than follow it.
    if let Some(pointer) = schema.get("$ref").and_then(|it| it.as_str()) {
        if trail.iter().any(|seen| seen == pointer) {
            return serde_json::Value::Null;
        }
        trail.push(pointer.to_string());
        let value = skeleton(document, resolve(document, schema), depth, trail);
        trail.pop();
        return value;
    }

    let Some(object) = schema.as_object() else {
        return serde_json::Value::Null;
    };

    // Anything the document states outright beats anything we invent.
    for key in ["example", "default"] {
        if let Some(value) = object.get(key) {
            return value.clone();
        }
    }
    if let Some(first) = object
        .get("enum")
        .and_then(|it| it.as_array())
        .and_then(|it| it.first())
    {
        return first.clone();
    }

    // `allOf` is a merge; `oneOf` and `anyOf` are a choice, and the first
    // branch is as good a guess as any.
    if let Some(parts) = object.get("allOf").and_then(|it| it.as_array()) {
        let mut merged = serde_json::Map::new();
        for part in parts {
            if let serde_json::Value::Object(fields) = skeleton(document, part, depth, trail) {
                merged.extend(fields);
            }
        }
        return serde_json::Value::Object(merged);
    }
    // A branch that only says `null` is the nullable half of the choice, not
    // the interesting one — `anyOf: [{"type": "null"}, {"type": "string"}]` is
    // a nullable string, and answering `null` hides the string. Same reasoning
    // as `type_of` below, for the other spelling of the same idea.
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = object.get(key).and_then(|it| it.as_array()) {
            let pick = branches
                .iter()
                .find(|branch| !only_null(branch))
                .or_else(|| branches.first());
            if let Some(branch) = pick {
                return skeleton(document, branch, depth, trail);
            }
        }
    }

    let kind = type_of(object);
    let kind = kind.as_deref();

    if kind == Some("object") || (kind.is_none() && object.contains_key("properties")) {
        let mut fields = serde_json::Map::new();
        if let Some(properties) = object.get("properties").and_then(|it| it.as_object()) {
            for (name, property) in properties {
                fields.insert(
                    name.clone(),
                    skeleton(document, property, depth + 1, trail),
                );
            }
        }
        return serde_json::Value::Object(fields);
    }

    if kind == Some("array") {
        let item = object
            .get("items")
            .map(|items| skeleton(document, items, depth + 1, trail))
            .unwrap_or(serde_json::Value::Null);
        return serde_json::Value::Array(if item.is_null() { vec![] } else { vec![item] });
    }

    // The name of the type rather than an empty value of it: "" tells you
    // nothing, `string` tells you what belongs there — and leaves the body
    // invalid until you have replaced it, which is the point.
    match kind {
        Some("string") => type_name("string"),
        Some("integer") => type_name("integer"),
        Some("number") => type_name("number"),
        Some("boolean") => type_name("boolean"),
        // `null` is a value, not a gap: a schema that says null means null.
        Some("null") => serde_json::Value::Null,
        // Anything else is a schema this cannot read — no type at all, a
        // vocabulary we do not implement, a `$ref` that went in a circle.
        // `null` was the old answer and it was a lie: it reads as "the API
        // wants null here" and looks filled in, so it was neither replaced nor
        // noticed. A gap that says it is a gap is worse to look at and better
        // to have.
        _ => type_name("unknown"),
    }
}

/// Whether a branch of a choice says nothing but "or null".
fn only_null(branch: &serde_json::Value) -> bool {
    branch
        .get("type")
        .and_then(|it| it.as_str())
        .is_some_and(|kind| kind == "null")
}

/// A schema's type, tolerating JSON Schema's two spellings of it.
///
/// OpenAPI 3.0 writes `"type": "string"` and marks nullability separately.
/// 3.1 dropped that and writes `"type": ["string", "null"]` instead — which is
/// what most generators now emit, and which read as no type at all here. Every
/// nullable field in a 3.1 document came out as `null`.
///
/// A union is reported as its first member that is not `null`, since the point
/// is to show what to type, and `null` is what you leave it as instead.
fn type_of(object: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    match object.get("type") {
        Some(serde_json::Value::String(name)) => Some(name.clone()),
        Some(serde_json::Value::Array(names)) => names
            .iter()
            .filter_map(|name| name.as_str())
            .find(|name| *name != "null")
            .map(str::to_string)
            // `"type": ["null"]` means exactly one thing.
            .or_else(|| names.first().and_then(|it| it.as_str()).map(str::to_string)),
        _ => None,
    }
}

fn string_at(document: &serde_json::Value, path: &[&str]) -> String {
    let mut current = document;
    for key in path {
        match current.get(key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_string()
}

/// OpenAPI 3 puts it in `servers[0].url`; Swagger 2 splits it across `schemes`,
/// `host` and `basePath`.
fn base_url(document: &serde_json::Value) -> String {
    if let Some(url) = document
        .get("servers")
        .and_then(|servers| servers.as_array())
        .and_then(|servers| servers.first())
        .and_then(|server| server.get("url"))
        .and_then(|url| url.as_str())
    {
        return url.trim_end_matches('/').to_string();
    }

    let host = string_at(document, &["host"]);
    if host.is_empty() {
        return String::new();
    }
    let scheme = document
        .get("schemes")
        .and_then(|schemes| schemes.as_array())
        .and_then(|schemes| schemes.first())
        .and_then(|scheme| scheme.as_str())
        .unwrap_or("https");
    let base = string_at(document, &["basePath"]);

    format!("{scheme}://{host}{base}")
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_openapi_3_document() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": { "title": "Acme", "version": "1.2.0" },
            "servers": [{ "url": "https://api.acme.com/v1/" }],
            "paths": {
                "/users": {
                    "get": { "operationId": "listUsers", "description": "All of them" },
                    "post": { "summary": "Create a user" },
                    "parameters": [{ "name": "page" }]
                },
                "/users/{id}": { "get": {} }
            }
        }"#;

        let import = parse(spec).unwrap();
        assert_eq!(import.title, "Acme");
        assert_eq!(import.version, "1.2.0");
        // Trailing slash removed, so joining a path can't double it.
        assert_eq!(import.base_url, "https://api.acme.com/v1");

        let keys: Vec<String> = import
            .endpoints
            .iter()
            .map(|endpoint| format!("{} {}", endpoint.method, endpoint.path))
            .collect();
        assert_eq!(
            keys,
            vec!["GET /users", "POST /users", "GET /users/{id}"],
            "`parameters` is not an operation"
        );

        assert_eq!(import.endpoints[0].name, "listUsers");
        assert_eq!(import.endpoints[0].description, "All of them");
        // No operationId, so the summary names it.
        assert_eq!(import.endpoints[1].name, "Create a user");
        // Neither, so the path does.
        assert_eq!(import.endpoints[2].name, "/users/{id}");
    }

    #[test]
    fn reads_yaml_too() {
        // The form most specs are actually handed over in.
        let spec = "
openapi: 3.0.0
info:
  title: Acme
  version: '1.0'
servers:
  - url: https://api.acme.com
paths:
  /health:
    get:
      operationId: health
";
        let import = parse(spec).unwrap();
        assert_eq!(import.base_url, "https://api.acme.com");
        assert_eq!(import.endpoints.len(), 1);
        assert_eq!(import.endpoints[0].name, "health");
    }

    #[test]
    fn reads_swagger_2_documents() {
        let spec = r#"{
            "swagger": "2.0",
            "info": { "title": "Legacy", "version": "1" },
            "schemes": ["https"],
            "host": "legacy.example.com",
            "basePath": "/api",
            "paths": { "/things": { "get": { "operationId": "listThings" } } }
        }"#;

        let import = parse(spec).unwrap();
        assert_eq!(import.base_url, "https://legacy.example.com/api");
        assert_eq!(import.endpoints[0].method, "GET");
    }

    #[test]
    fn a_spec_without_servers_imports_without_a_base_url() {
        let spec = r#"{"openapi":"3.0.0","paths":{"/x":{"get":{}}}}"#;
        let import = parse(spec).unwrap();
        assert!(import.base_url.is_empty(), "nothing to guess from");
        assert_eq!(import.endpoints.len(), 1);
    }

    #[test]
    fn explains_what_is_wrong_with_a_document_it_cannot_use() {
        assert!(matches!(parse("{{{"), Err(ImportError::Unparseable(_))));
        assert!(matches!(
            parse(r#"{"openapi":"3.0.0"}"#),
            Err(ImportError::NotOpenApi)
        ));
        assert!(matches!(
            parse(r#"{"openapi":"3.0.0","paths":{}}"#),
            Err(ImportError::NoOperations)
        ));
        // A path item with only metadata has no operations either.
        assert!(matches!(
            parse(r#"{"paths":{"/x":{"parameters":[]}}}"#),
            Err(ImportError::NoOperations)
        ));
    }

    #[test]
    fn keys_match_what_the_rest_of_the_app_uses() {
        // Imported endpoints become ordinary requests, so their identity must
        // read the same as a loaded one: `METHOD /path`.
        let import = parse(r#"{"paths":{"/users":{"get":{}}}}"#).unwrap();
        let endpoint = &import.endpoints[0];
        assert_eq!(
            format!("{} {}", endpoint.method, endpoint.path),
            crate::loader::LoadedEndpoint {
                method: "get".into(),
                path: "/users".into(),
                name: String::new(),
                description: String::new(),
                body: String::new(),
            }
            .key()
        );
    }

    #[test]
    fn fills_a_body_from_the_request_schema() {
        let spec = r##"{
            "openapi": "3.0.0",
            "paths": {
                "/users": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/NewUser" }
                                }
                            }
                        }
                    }
                }
            },
            "components": {
                "schemas": {
                    "NewUser": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "age": { "type": "integer" },
                            "admin": { "type": "boolean" },
                            "tags": { "type": "array", "items": { "type": "string" } },
                            "role": { "type": "string", "enum": ["owner", "guest"] },
                            "address": {
                                "type": "object",
                                "properties": { "city": { "type": "string" } }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        let body = &import.endpoints[0].body;

        // Type names, not empty values: `""` says nothing about what belongs
        // there, and a body that is still full of them is visibly unfinished.
        assert!(body.contains("\"name\": string"), "{body}");
        assert!(body.contains("\"age\": integer"), "{body}");
        assert!(body.contains("\"admin\": boolean"), "{body}");
        assert!(body.contains("\"city\": string"), "{body}");
        // An array shows what one element looks like.
        assert!(body.contains("\"tags\": [\n    string\n  ]"), "{body}");
        // An enum has real values, so it names one rather than its type.
        assert!(body.contains("\"role\": \"owner\""), "{body}");
    }

    /// OpenAPI 3.1 spells "nullable string" as a union of types, and that is
    /// what most generators now emit. Reading only the string form meant every
    /// such field came out as `null` — which looks like a filled-in value, so
    /// it was neither replaced nor noticed.
    #[test]
    fn a_nullable_field_still_names_its_type() {
        let spec = r##"{
            "openapi": "3.1.0",
            "info": { "title": "Acme", "version": "1" },
            "paths": {
                "/users": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": ["string", "null"] },
                                            "age": { "type": ["null", "integer"] },
                                            "nothing": { "type": ["null"] },
                                            "anything": {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        let body = &import.endpoints[0].body;

        assert!(body.contains("\"name\": string"), "{body}");
        // Order in the union says nothing: `null` is never the interesting half.
        assert!(body.contains("\"age\": integer"), "{body}");
        // Except when it is the only half.
        assert!(body.contains("\"nothing\": null"), "{body}");
        // A schema with nothing in it is a gap too, and says so rather than
        // claiming the API wants null.
        assert!(body.contains("\"anything\": unknown"), "{body}");
    }

    /// The other spelling of nullable, and the one real specs in the wild use:
    /// a choice with a `null` branch in it. Which side it sits on is arbitrary,
    /// so the branch that says something wins either way.
    #[test]
    fn a_nullable_choice_names_the_half_that_says_something() {
        let spec = r##"{
            "openapi": "3.1.0",
            "info": { "title": "Acme", "version": "1" },
            "paths": {
                "/users": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "after": {
                                                "anyOf": [{ "type": "string" }, { "type": "null" }]
                                            },
                                            "before": {
                                                "anyOf": [{ "type": "null" }, { "type": "string" }]
                                            },
                                            "either": {
                                                "oneOf": [{ "type": "null" }, { "type": "integer" }]
                                            },
                                            "open": {
                                                "anyOf": [{}, { "type": "null" }]
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        let body = &import.endpoints[0].body;

        assert!(body.contains("\"after\": string"), "{body}");
        assert!(body.contains("\"before\": string"), "{body}");
        assert!(body.contains("\"either\": integer"), "{body}");
        // Nothing to say on either side, so it is a gap rather than a `null`.
        assert!(body.contains("\"open\": unknown"), "{body}");
    }

    /// An example the author wrote beats one derived from the schema.
    #[test]
    fn an_example_in_the_document_wins() {
        let spec = r##"{
            "openapi": "3.0.0",
            "paths": {
                "/users": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "example": { "name": "Ada" },
                                    "schema": {
                                        "type": "object",
                                        "properties": { "name": { "type": "string" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&import.endpoints[0].body).unwrap(),
            json!({ "name": "Ada" })
        );
    }

    /// A schema that contains itself is ordinary — a comment with replies. The
    /// trail has to stop it rather than recurse until the stack gives out.
    #[test]
    fn a_self_referencing_schema_terminates() {
        let spec = r##"{
            "openapi": "3.0.0",
            "paths": {
                "/comments": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/Comment" }
                                }
                            }
                        }
                    }
                }
            },
            "components": {
                "schemas": {
                    "Comment": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "replies": {
                                "type": "array",
                                "items": { "$ref": "#/components/schemas/Comment" }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        let body = &import.endpoints[0].body;
        assert!(body.contains("\"text\": string"), "{body}");
        // The cycle is cut, so `replies` is present but empty rather than
        // nested forever.
        assert!(body.contains("\"replies\": []"), "{body}");
    }

    /// Swagger 2 has no `requestBody` — the schema rides on a parameter.
    #[test]
    fn reads_a_swagger_2_body_parameter() {
        let spec = r##"{
            "swagger": "2.0",
            "paths": {
                "/users": {
                    "post": {
                        "parameters": [
                            { "in": "query", "name": "dry" },
                            {
                                "in": "body",
                                "name": "user",
                                "schema": {
                                    "type": "object",
                                    "properties": { "name": { "type": "string" } }
                                }
                            }
                        ]
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        assert!(import.endpoints[0].body.contains("\"name\": string"));
    }

    /// A GET takes no body, and a form-encoded one has no honest JSON skeleton.
    #[test]
    fn leaves_the_body_empty_when_there_is_nothing_to_build() {
        let spec = r##"{
            "openapi": "3.0.0",
            "paths": {
                "/users": { "get": { "operationId": "listUsers" } },
                "/upload": {
                    "post": {
                        "requestBody": {
                            "content": {
                                "application/x-www-form-urlencoded": {
                                    "schema": {
                                        "type": "object",
                                        "properties": { "file": { "type": "string" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"##;

        let import = parse(spec).unwrap();
        for endpoint in &import.endpoints {
            assert_eq!(endpoint.body, "", "{} should carry no body", endpoint.path);
        }
    }
}

#[cfg(test)]
mod real_world {
    /// Parses the real Swagger Petstore spec, vendored so this always runs.
    /// Hand-written fixtures test the shapes I thought of; this one catches the
    /// shapes I didn't.
    #[test]
    fn handles_the_petstore_spec() {
        let text = include_str!("../tests/fixtures/petstore.json");
        let import = super::parse(text).expect("petstore should parse");

        assert!(import.endpoints.len() > 10, "{} endpoints", import.endpoints.len());
        assert!(!import.title.is_empty());
        assert!(
            import.endpoints.iter().all(|e| e.path.starts_with('/')),
            "every path should be rooted"
        );
        assert!(
            import
                .endpoints
                .iter()
                .all(|e| e.method.chars().all(|c| c.is_ascii_uppercase())),
            "methods are normalised"
        );
        // `parameters` sits alongside operations in a path item and is not one.
        assert!(!import.endpoints.iter().any(|e| e.method == "PARAMETERS"));
    }
}
