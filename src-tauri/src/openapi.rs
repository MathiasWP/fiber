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

            endpoints.push(ImportedEndpoint {
                method: method.to_ascii_uppercase(),
                path: path.clone(),
                name,
                description,
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
            }
            .key()
        );
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
