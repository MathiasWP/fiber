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
use crate::loader::LoaderConfig;
use crate::http::{BodyKind, FormField, Header};

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
    /// Where this sits in the sidebar. Sections used to be sorted by name, which
    /// left no way to express an order you'd chosen by dragging.
    #[serde(default)]
    pub order: i32,
    /// Applied to every send from this collection. 0 means the HTTP default (60s).
    #[serde(default = "default_timeout_ms", skip_serializing_if = "is_default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub follow_redirects: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accept_invalid_certs: bool,
    /// Empty means no proxy — the system default, not "don't connect".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy: String,
    // Tables and array-of-tables must come after the scalars, or the TOML is
    // invalid. `auth` holds only a keychain reference, never a credential.
    #[serde(default)]
    pub auth: AuthConfig,
    /// A script that reports this section's endpoints. Absent for hand-written
    /// sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<LoaderConfig>,
    /// What the MCP server may do with this section. Off by default: a tool
    /// that can send authenticated requests is a confused deputy, so exposure
    /// is a decision the user makes per collection.
    #[serde(default)]
    pub mcp: McpAccess,
    /// Hand-written requests.
    #[serde(default)]
    pub requests: Vec<SavedRequest>,
    /// User data attached to *loaded* endpoints, keyed by `id` = `"GET /path"`.
    ///
    /// Kept apart from `requests` because loader output is regenerated on every
    /// refresh and must never be the source of truth for a body someone spent
    /// ten minutes writing. See §6 of the design doc.
    #[serde(default)]
    pub overlay: Vec<SavedRequest>,
}

fn default_timeout_ms() -> u64 {
    60_000
}

fn is_default_timeout(value: &u64) -> bool {
    *value == 60_000
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for Section {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            collapsed: false,
            order: 0,
            timeout_ms: 60_000,
            follow_redirects: true,
            accept_invalid_certs: false,
            proxy: String::new(),
            auth: AuthConfig::None,
            loader: None,
            mcp: McpAccess::default(),
            requests: Vec::new(),
            overlay: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpAccess {
    /// Whether agents can see this section at all.
    #[serde(default)]
    pub enabled: bool,
    /// Whether they may use anything but GET, HEAD and OPTIONS.
    #[serde(default)]
    pub allow_writes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRequest {
    pub id: String,
    pub name: String,
    pub method: String,
    /// Relative to the section's base URL. An absolute URL here wins outright.
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// OpenAPI tag, used as a folder in the sidebar. Empty is ungrouped.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "is_json_body")]
    pub body_kind: BodyKind,
    /// Form or multipart fields. Ignored when `body_kind` is json/text/file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<FormField>,
    /// Absolute path of a file to send as the raw body. `body_kind` is `file`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Values for `{name}` placeholders in `path`. Identity stays the template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_params: Vec<Header>,
    #[serde(default)]
    pub headers: Vec<Header>,
}

fn is_json_body(kind: &BodyKind) -> bool {
    *kind == BodyKind::Json
}

impl Default for SavedRequest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            method: "GET".into(),
            path: String::new(),
            description: String::new(),
            tag: String::new(),
            body: String::new(),
            body_kind: BodyKind::Json,
            form: Vec::new(),
            file: String::new(),
            path_params: Vec::new(),
            headers: Vec::new(),
        }
    }
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
    #[error("section file {file} is corrupt: {message}")]
    Corrupt { file: String, message: String },
}

impl Serialize for StoreError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Ids become file names — section files, loader caches, spilled history
/// bodies — so anything that could escape those directories is rejected.
pub(crate) fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The sections directory's application of that rule. Matters most once the
/// MCP server can create sections.
fn safe_file_name(id: &str) -> Result<String, StoreError> {
    if is_safe_id(id) {
        Ok(format!("{id}.toml"))
    } else {
        Err(StoreError::UnsafeId(id.to_string()))
    }
}

/// What a load found: the sections that parsed, and the files that didn't.
/// Both go to the UI — a corrupt file used to be skipped with only a log line,
/// which from the sidebar reads exactly like data loss.
#[derive(Debug, Serialize)]
pub struct SectionLoad {
    pub sections: Vec<Section>,
    pub errors: Vec<SectionLoadError>,
}

#[derive(Debug, Serialize)]
pub struct SectionLoadError {
    /// The file name alone; the UI already knows the directory.
    pub file: String,
    pub message: String,
}

/// Reads every section in `dir`. Files that fail to read or parse are reported
/// alongside the rest rather than failing the whole load — one bad file
/// shouldn't hide the rest of your work, but it shouldn't hide itself either.
pub fn load_all_reporting(dir: &Path) -> Result<SectionLoad, StoreError> {
    if !dir.exists() {
        return Ok(SectionLoad {
            sections: Vec::new(),
            errors: Vec::new(),
        });
    }

    let entries = fs::read_dir(dir).map_err(|source| StoreError::Read {
        path: dir.display().to_string(),
        source,
    })?;

    let mut sections = Vec::new();
    let mut errors = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        match fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Section>(&text) {
                Ok(section) => sections.push(section),
                Err(err) => errors.push(SectionLoadError {
                    file,
                    message: err.message().to_string(),
                }),
            },
            Err(err) => errors.push(SectionLoadError {
                file,
                message: err.to_string(),
            }),
        }
    }

    // Explicit order first, name as the tie-break so sections written before
    // ordering existed still come out somewhere sensible.
    sections.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(SectionLoad { sections, errors })
}

/// As above, for callers with nobody to show a broken file to — the MCP server
/// and the migration. There the unparseable files are logged and skipped.
pub fn load_all(dir: &Path) -> Result<Vec<Section>, StoreError> {
    let load = load_all_reporting(dir)?;
    for error in &load.errors {
        log::warn!("skipping {}: {}", error.file, error.message);
    }
    Ok(load.sections)
}

/// Reads a single section by id. Used when the in-memory cache has not seen
/// this collection yet (MCP, or a send before `list_sections` has returned).
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
    match toml::from_str(&text) {
        Ok(section) => Ok(Some(section)),
        // A file that exists but no longer parses is not "no section". Treating
        // it as such once meant the request went out anyway — with the
        // section's auth silently missing.
        Err(err) => Err(StoreError::Corrupt {
            file: format!("{id}.toml"),
            message: err.message().to_string(),
        }),
    }
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
    // The temp name carries the process id: the app and a headless `fiber mcp`
    // can save the same section at once, and two writers sharing one temp file
    // could rename each other's half-written bytes into place. Same directory
    // as the target, so the rename stays atomic.
    let temp = target.with_extension(format!("toml.tmp-{}", std::process::id()));

    (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, encoded.as_bytes())?;
        // Flushed to disk *before* the rename. The rename orders the metadata,
        // not the data — without this, a crash at the wrong moment could leave
        // the renamed file in place but empty.
        file.sync_all()
    })()
    .map_err(|source| StoreError::Write {
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

/// Replaces `{name}` placeholders in a path with the given values.
///
/// Empty values are left as `{name}` so a half-filled template is obvious
/// rather than silently sending `/pet/` for `/pet/{petId}`. Values are percent-
/// encoded as a single path segment — slashes in a value do not become extra
/// path components.
pub fn apply_path_params(path: &str, params: &[Header]) -> String {
    let mut result = path.to_string();
    for param in params {
        let name = param.name.trim();
        if name.is_empty() || param.value.is_empty() {
            continue;
        }
        let needle = format!("{{{name}}}");
        result = result.replace(&needle, &encode_path_segment(&param.value));
    }
    result
}

fn encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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
    fn substitutes_path_params_without_inventing_slashes() {
        assert_eq!(
            apply_path_params("/pet/{petId}", &[Header {
                name: "petId".into(),
                value: "123".into(),
            }]),
            "/pet/123"
        );
        assert_eq!(
            apply_path_params("/pet/{petId}/uploadImage", &[Header {
                name: "petId".into(),
                value: "a/b".into(),
            }]),
            "/pet/a%2Fb/uploadImage"
        );
        // Unfilled placeholders stay visible rather than collapsing the path.
        assert_eq!(apply_path_params("/pet/{petId}", &[]), "/pet/{petId}");
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
            order: 0,
            auth: crate::auth::AuthConfig::None,
            loader: None,
            mcp: Default::default(),
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
                ..Default::default()
            }],
            overlay: vec![],
        ..Default::default()
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
        assert_eq!(loaded[0].timeout_ms, 60_000);
        assert!(loaded[0].follow_redirects);
        assert!(!loaded[0].accept_invalid_certs);
        assert!(loaded[0].proxy.is_empty());

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
                order: 0,
                auth: crate::auth::AuthConfig::None,
                loader: None,
                mcp: Default::default(),
                requests: vec![],
                overlay: vec![],
            ..Default::default()
            },
        )
        .unwrap();

        let loaded = load_all(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "good");

        let _ = fs::remove_dir_all(&dir);
    }

    /// A corrupt file must be *reported*, not merely survived: from the sidebar
    /// a silently skipped section is indistinguishable from a deleted one.
    #[test]
    fn a_corrupt_file_is_named_alongside_the_survivors() {
        let dir = std::env::temp_dir().join(format!("fetch-store-report-{}", std::process::id()));
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
                order: 0,
                auth: crate::auth::AuthConfig::None,
                loader: None,
                mcp: Default::default(),
                requests: vec![],
                overlay: vec![],
            ..Default::default()
            },
        )
        .unwrap();

        let load = load_all_reporting(&dir).unwrap();
        assert_eq!(load.sections.len(), 1);
        assert_eq!(load.errors.len(), 1);
        assert_eq!(load.errors[0].file, "broken.toml");
        assert!(!load.errors[0].message.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    /// `load_one` backs every authenticated send, so "missing" and "corrupt"
    /// must stay distinct: missing means no auth to apply, corrupt means the
    /// send has to stop rather than go out unauthenticated.
    #[test]
    fn a_corrupt_section_is_an_error_not_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("fetch-store-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert!(load_one(&dir, "absent").unwrap().is_none(), "missing is fine");

        fs::write(dir.join("bad.toml"), "not = valid [[[").unwrap();
        let err = load_one(&dir, "bad").unwrap_err();
        assert!(
            err.to_string().contains("bad.toml is corrupt"),
            "the message should name the file: {err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
