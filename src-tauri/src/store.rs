//! Collections on disk.
//!
//! One TOML file per section, plain text and stably ordered so diffs stay
//! readable — whether or not the directory is ever committed to git. Secrets
//! never live here; section files hold a reference and the value comes from the
//! OS keychain at send time (step 4).
//!
//! Like `http`, this module is free of Tauri types so the MCP server can read
//! the same files without a window.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::auth::AuthConfig;
use crate::http::Header;

/// A group of requests that share a base URL. Auth and loaders attach here too,
/// in later steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub collapsed: bool,
    // Tables and array-of-tables must come after the scalars, or the TOML is
    // invalid. `auth` holds only a keychain reference, never a credential.
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub requests: Vec<SavedRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRequest {
    pub id: String,
    pub name: String,
    pub method: String,
    /// Relative to the section's base URL. An absolute URL here wins outright.
    pub path: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub headers: Vec<Header>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unsafe section id `{0}`")]
    UnsafeId(String),
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not encode section: {0}")]
    Encode(#[from] toml::ser::Error),
}

impl Serialize for StoreError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Ids become file names, so anything that could escape the sections directory
/// is rejected. Matters most once the MCP server can create sections.
fn safe_file_name(id: &str) -> Result<String, StoreError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(format!("{id}.toml"))
    } else {
        Err(StoreError::UnsafeId(id.to_string()))
    }
}

/// Reads every section in `dir`, skipping files that fail to parse rather than
/// failing the whole load — one bad file shouldn't hide the rest of your work.
pub fn load_all(dir: &Path) -> Result<Vec<Section>, StoreError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(dir).map_err(|source| StoreError::Read {
        path: dir.display().to_string(),
        source,
    })?;

    let mut sections = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        match fs::read_to_string(&path).map_err(|source| StoreError::Read {
            path: path.display().to_string(),
            source,
        }) {
            Ok(text) => match toml::from_str::<Section>(&text) {
                Ok(section) => sections.push(section),
                Err(err) => log::warn!("skipping {}: {err}", path.display()),
            },
            Err(err) => log::warn!("{err}"),
        }
    }

    sections.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(sections)
}

/// Reads a single section by id. Used on every send to find the section's auth
/// config, so the file on disk stays the single source of truth.
pub fn load_one(dir: &Path, id: &str) -> Result<Option<Section>, StoreError> {
    let path = dir.join(safe_file_name(id)?);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Read {
                path: path.display().to_string(),
                source,
            })
        }
    };
    Ok(toml::from_str(&text).ok())
}

/// Writes via a temp file and a rename so an interrupted save can't leave a
/// half-written section behind.
pub fn save(dir: &Path, section: &Section) -> Result<(), StoreError> {
    let file_name = safe_file_name(&section.id)?;
    fs::create_dir_all(dir).map_err(|source| StoreError::Write {
        path: dir.display().to_string(),
        source,
    })?;

    let encoded = toml::to_string_pretty(section)?;
    let target = dir.join(file_name);
    let temp = target.with_extension("toml.tmp");

    fs::write(&temp, encoded).map_err(|source| StoreError::Write {
        path: temp.display().to_string(),
        source,
    })?;
    fs::rename(&temp, &target).map_err(|source| StoreError::Write {
        path: target.display().to_string(),
        source,
    })
}

pub fn delete(dir: &Path, id: &str) -> Result<(), StoreError> {
    let target = dir.join(safe_file_name(id)?);
    match fs::remove_file(&target) {
        Ok(()) => Ok(()),
        // Already gone is the outcome the caller wanted.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Write {
            path: target.display().to_string(),
            source,
        }),
    }
}

/// Resolves a request's path against its section's base URL.
///
/// This is the whole of the "no variables" idea: a section owns the base, a
/// request owns a path. Kept in Rust so the app and the MCP server can never
/// disagree about where a request is actually going.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim();
    let path = path.trim();

    // An absolute path ignores the section entirely — the escape hatch for the
    // one endpoint that lives somewhere else.
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if base.is_empty() {
        return path.to_string();
    }
    if path.is_empty() {
        return base.trim_end_matches('/').to_string();
    }

    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// `<app data>/sections`
pub fn sections_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sections")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_base_and_path() {
        assert_eq!(join_url("https://a.com", "/user"), "https://a.com/user");
        assert_eq!(join_url("https://a.com/", "/user"), "https://a.com/user");
        assert_eq!(join_url("https://a.com/v1", "user"), "https://a.com/v1/user");
        assert_eq!(join_url("https://a.com/v1/", "/user"), "https://a.com/v1/user");
        assert_eq!(
            join_url("https://a.com", "/user?expand=orders"),
            "https://a.com/user?expand=orders"
        );
    }

    #[test]
    fn absolute_paths_bypass_the_section() {
        assert_eq!(
            join_url("https://a.com", "https://b.com/z"),
            "https://b.com/z"
        );
        assert_eq!(join_url("", "https://b.com/z"), "https://b.com/z");
    }

    #[test]
    fn tolerates_empty_sides() {
        assert_eq!(join_url("https://a.com/", ""), "https://a.com");
        assert_eq!(join_url("", "/user"), "/user");
        assert_eq!(join_url("", ""), "");
    }

    #[test]
    fn rejects_ids_that_could_escape_the_directory() {
        assert!(safe_file_name("../../etc/passwd").is_err());
        assert!(safe_file_name("a/b").is_err());
        assert!(safe_file_name("").is_err());
        assert!(safe_file_name("acme-api_1").is_ok());
    }

    #[test]
    fn round_trips_a_section_through_disk() {
        let dir = std::env::temp_dir().join(format!("fetch-store-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let section = Section {
            id: "acme".into(),
            name: "Acme API".into(),
            base_url: "https://api.acme.com".into(),
            collapsed: false,
            auth: crate::auth::AuthConfig::None,
            requests: vec![SavedRequest {
                id: "req1".into(),
                name: "Get user".into(),
                method: "GET".into(),
                path: "/user/42".into(),
                body: String::new(),
                headers: vec![Header {
                    name: "Accept".into(),
                    value: "application/json".into(),
                }],
            }],
        };

        save(&dir, &section).unwrap();

        // The file a human would open and diff.
        let raw = fs::read_to_string(dir.join("acme.toml")).unwrap();
        assert!(raw.contains("baseUrl = \"https://api.acme.com\""), "{raw}");

        let loaded = load_all(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Acme API");
        assert_eq!(loaded[0].requests.len(), 1);
        assert_eq!(loaded[0].requests[0].path, "/user/42");
        assert_eq!(loaded[0].requests[0].headers[0].name, "Accept");

        delete(&dir, "acme").unwrap();
        assert!(load_all(&dir).unwrap().is_empty());
        // Deleting something already gone is not an error.
        delete(&dir, "acme").unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_corrupt_file_does_not_hide_the_others() {
        let dir = std::env::temp_dir().join(format!("fetch-store-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("broken.toml"), "this is not = valid toml [[[").unwrap();
        save(
            &dir,
            &Section {
                id: "good".into(),
                name: "Good".into(),
                base_url: String::new(),
                collapsed: false,
                auth: crate::auth::AuthConfig::None,
                requests: vec![],
            },
        )
        .unwrap();

        let loaded = load_all(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "good");

        let _ = fs::remove_dir_all(&dir);
    }
}
