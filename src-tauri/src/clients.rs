//! Putting Fiber into an AI client's MCP config.
//!
//! `fiber mcp` is the entire server — same binary, different first argument —
//! so "installing" is one entry in one file per client. Every client here
//! stores that entry the same way Claude Desktop first did (`mcpServers`, a
//! command and its arguments); the two exceptions are VS Code, which names the
//! transport and calls the map `servers`, and Codex, whose config is TOML.
//!
//! The rule for touching someone else's config file is that Fiber only ever
//! adds or removes its own key. JSON files are parsed, edited and written back
//! whole — that is what the clients' own CLIs do, and those files are machine
//! written to begin with. Codex's is not: a `config.toml` is hand-edited and
//! full of comments, so there the edit is textual and touches only the
//! `[mcp_servers.fiber]` block. Nothing else in the file moves.
//!
//! A file that doesn't parse is left completely alone. The tab offers the
//! snippet to paste instead, which is the honest outcome — a rewrite would be
//! guessing at what the file meant.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The argument that turns the app binary into the MCP server.
const ARGS: [&str; 1] = ["mcp"];
/// The name Fiber gives itself in every client's server map.
const KEY: &str = "fiber";

/// How a client stores its server list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `{"mcpServers": {"fiber": {"command": …, "args": […]}}}` — Claude
    /// Desktop's original layout, which nearly everyone copied.
    McpServers,
    /// VS Code: the map is `servers` and each entry names its transport.
    VsCode,
    /// Codex: TOML, `[mcp_servers.fiber]`.
    Codex,
}

impl Shape {
    /// The top-level key holding the server map, for the JSON shapes.
    fn map_key(self) -> &'static str {
        match self {
            Shape::VsCode => "servers",
            _ => "mcpServers",
        }
    }
}

struct Def {
    id: &'static str,
    name: &'static str,
    shape: Shape,
}

/// Ordered as the tab shows them: the clients most likely to be installed
/// alongside an API client first.
const CLIENTS: &[Def] = &[
    Def {
        id: "claude-code",
        name: "Claude Code",
        shape: Shape::McpServers,
    },
    Def {
        id: "claude-desktop",
        name: "Claude Desktop",
        shape: Shape::McpServers,
    },
    Def {
        id: "cursor",
        name: "Cursor",
        shape: Shape::McpServers,
    },
    Def {
        id: "vscode",
        name: "VS Code",
        shape: Shape::VsCode,
    },
    Def {
        id: "windsurf",
        name: "Windsurf",
        shape: Shape::McpServers,
    },
    Def {
        id: "codex",
        name: "Codex CLI",
        shape: Shape::Codex,
    },
    Def {
        id: "gemini",
        name: "Gemini CLI",
        shape: Shape::McpServers,
    },
];

fn def(id: &str) -> Result<&'static Def, ClientError> {
    CLIENTS
        .iter()
        .find(|client| client.id == id)
        .ok_or_else(|| ClientError::Unknown(id.to_string()))
}

/// Where the client keeps its config.
///
/// The dot-directory clients use the same path everywhere. The two that follow
/// platform convention take the OS data directory on macOS and Windows
/// (`~/Library/Application Support`, `%APPDATA%`) and the config directory on
/// Linux (`~/.config`), which is where each of them actually looks.
fn config_path(id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let app = if cfg!(target_os = "linux") {
        dirs::config_dir()
    } else {
        dirs::data_dir()
    };

    match id {
        "claude-code" => Some(home?.join(".claude.json")),
        "claude-desktop" => Some(app?.join("Claude").join("claude_desktop_config.json")),
        "cursor" => Some(home?.join(".cursor").join("mcp.json")),
        "vscode" => Some(app?.join("Code").join("User").join("mcp.json")),
        "windsurf" => Some(
            home?
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
        ),
        "codex" => Some(home?.join(".codex").join("config.toml")),
        "gemini" => Some(home?.join(".gemini").join("settings.json")),
        _ => None,
    }
}

/// The binary an entry should point at: this one.
///
/// On macOS that is the executable inside the bundle
/// (`/Applications/Fiber.app/Contents/MacOS/fiber`), which is exactly what a
/// client needs to spawn. Asking the running process beats hard-coding a path,
/// because it stays right when the app is somewhere else — a second copy, a
/// dev build, `~/Applications`.
pub fn binary() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        // If the OS won't say, the bare name is the one thing that might still
        // work: a client whose PATH has Fiber on it.
        .unwrap_or_else(|_| "fiber".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Listed, and pointing at this binary.
    Installed,
    /// Listed, but pointing somewhere else — usually the app moved.
    Outdated,
    /// Not listed.
    Absent,
    /// The config exists but could not be parsed, so Fiber won't rewrite it.
    Unreadable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: String,
    pub name: String,
    /// The config file, with the home directory shortened to `~`.
    pub path: String,
    pub state: State,
    /// Whether the client looks installed at all — its config file or the
    /// directory that holds it exists. Only a hint: a client can be installed
    /// and not have written its config yet.
    pub detected: bool,
    /// What is configured now, when that is not this binary. Shown so a stale
    /// entry explains itself rather than just claiming to be wrong.
    pub command: Option<String>,
    /// Why the file could not be read, when `state` is `Unreadable`.
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("unknown client `{0}`")]
    Unknown(String),
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
    #[error("{path} could not be parsed, so it was left alone — paste the snippet in by hand ({message})")]
    Corrupt { path: String, message: String },
    #[error(
        "{path} lists Fiber in a form that can't be edited safely — change it by hand instead"
    )]
    HandWritten { path: String },
    #[error("Fiber doesn't know where {0} keeps its config on this system")]
    NoPath(String),
}

impl Serialize for ClientError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// What a client currently has under Fiber's key.
struct Entry {
    command: String,
    args: Vec<String>,
}

impl Entry {
    /// Whether this entry already runs the binary we would install.
    fn matches(&self, binary: &str) -> bool {
        self.command == binary && self.args == ARGS
    }
}

/// Every client, with what its config says right now.
pub fn statuses() -> Vec<Status> {
    let binary = binary();
    CLIENTS
        .iter()
        .map(|client| status(client, &binary))
        .collect()
}

fn status(client: &Def, binary: &str) -> Status {
    let Some(path) = config_path(client.id) else {
        return Status {
            id: client.id.to_string(),
            name: client.name.to_string(),
            path: String::new(),
            state: State::Unreadable,
            detected: false,
            command: None,
            message: Some(ClientError::NoPath(client.name.to_string()).to_string()),
        };
    };

    let detected = path.exists() || path.parent().is_some_and(Path::exists);
    let (state, command, message) = match read_entry(client.shape, &path) {
        Ok(None) => (State::Absent, None, None),
        Ok(Some(entry)) if entry.matches(binary) => (State::Installed, None, None),
        Ok(Some(entry)) => (State::Outdated, Some(entry.command), None),
        Err(err) => (State::Unreadable, None, Some(err.to_string())),
    };

    Status {
        id: client.id.to_string(),
        name: client.name.to_string(),
        path: pretty(&path),
        state,
        detected,
        command,
        message,
    }
}

/// Adds — or corrects — Fiber's entry, and reports what the file says
/// afterwards.
pub fn install(id: &str) -> Result<Status, ClientError> {
    let client = def(id)?;
    let path = config_path(id).ok_or_else(|| ClientError::NoPath(client.name.to_string()))?;
    let binary = binary();

    match client.shape {
        Shape::Codex => install_toml(&path, &binary)?,
        shape => install_json(shape, &path, &binary)?,
    }
    Ok(status(client, &binary))
}

/// Removes Fiber's entry, leaving everything else in the file as it was.
pub fn uninstall(id: &str) -> Result<Status, ClientError> {
    let client = def(id)?;
    let path = config_path(id).ok_or_else(|| ClientError::NoPath(client.name.to_string()))?;

    match client.shape {
        Shape::Codex => uninstall_toml(&path)?,
        shape => uninstall_json(shape, &path)?,
    }
    Ok(status(client, &binary()))
}

// ---------------------------------------------------------------- JSON shapes

/// The whole config, or an empty object when there is no file yet. A file that
/// exists but doesn't parse is an error rather than an empty object: treating
/// it as empty would replace someone's config with nothing but Fiber.
fn load_json(path: &Path) -> Result<serde_json::Value, ClientError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::Value::Object(Default::default()))
        }
        Err(source) => {
            return Err(ClientError::Read {
                path: pretty(path),
                source,
            })
        }
    };
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    // Comments are the common cause here — VS Code's own `mcp.json` allows
    // them and serde_json does not. The message says the file was left alone,
    // which is the part that matters.
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|err| ClientError::Corrupt {
        path: pretty(path),
        message: err.to_string(),
    })?;
    if !value.is_object() {
        return Err(ClientError::Corrupt {
            path: pretty(path),
            message: "the file isn't a JSON object".to_string(),
        });
    }
    Ok(value)
}

fn read_json_entry(shape: Shape, path: &Path) -> Result<Option<Entry>, ClientError> {
    let value = load_json(path)?;
    let Some(entry) = value.get(shape.map_key()).and_then(|map| map.get(KEY)) else {
        return Ok(None);
    };
    Ok(Some(Entry {
        command: entry
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        args: entry
            .get("args")
            .and_then(|value| value.as_array())
            .map(|args| {
                args.iter()
                    .map(|arg| arg.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
    }))
}

fn json_entry(shape: Shape, binary: &str) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    // VS Code wants the transport named; the others infer stdio from the
    // presence of a command.
    if shape == Shape::VsCode {
        entry.insert("type".to_string(), "stdio".into());
    }
    entry.insert("command".to_string(), binary.into());
    entry.insert(
        "args".to_string(),
        serde_json::Value::Array(ARGS.iter().map(|arg| (*arg).into()).collect()),
    );
    serde_json::Value::Object(entry)
}

fn install_json(shape: Shape, path: &Path, binary: &str) -> Result<(), ClientError> {
    let mut config = load_json(path)?;
    let root = config
        .as_object_mut()
        .expect("load_json returns an object or an error");

    // A map that exists but holds something other than an object is the one
    // case left: replacing it would throw away servers, so refuse.
    let map = root
        .entry(shape.map_key())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let Some(map) = map.as_object_mut() else {
        return Err(ClientError::Corrupt {
            path: pretty(path),
            message: format!("`{}` isn't an object", shape.map_key()),
        });
    };
    map.insert(KEY.to_string(), json_entry(shape, binary));

    let mut text = serde_json::to_string_pretty(&config).map_err(|err| ClientError::Corrupt {
        path: pretty(path),
        message: err.to_string(),
    })?;
    text.push('\n');
    write_atomically(path, &text)
}

fn uninstall_json(shape: Shape, path: &Path) -> Result<(), ClientError> {
    let mut config = load_json(path)?;
    let root = config
        .as_object_mut()
        .expect("load_json returns an object or an error");

    let Some(map) = root.get_mut(shape.map_key()).and_then(|map| map.as_object_mut()) else {
        // Nothing to take out. Not writing at all keeps a "remove" that had no
        // work to do from reformatting the file.
        return Ok(());
    };
    if map.remove(KEY).is_none() {
        return Ok(());
    }

    let mut text = serde_json::to_string_pretty(&config).map_err(|err| ClientError::Corrupt {
        path: pretty(path),
        message: err.to_string(),
    })?;
    text.push('\n');
    write_atomically(path, &text)
}

// ----------------------------------------------------------------------- TOML

/// Codex's config is hand-written, so it is read with a parser and edited as
/// text. Round-tripping it through a TOML serialiser would sort the keys and
/// drop every comment in the file.
fn read_toml_entry(path: &Path) -> Result<Option<Entry>, ClientError> {
    match read_optional(path)? {
        Some(text) => parse_toml_entry(&text, path),
        None => Ok(None),
    }
}

fn parse_toml_entry(text: &str, path: &Path) -> Result<Option<Entry>, ClientError> {
    let table: toml::Table = toml::from_str(text).map_err(|err| ClientError::Corrupt {
        path: pretty(path),
        message: err.message().to_string(),
    })?;
    let Some(entry) = table.get("mcp_servers").and_then(|map| map.get(KEY)) else {
        return Ok(None);
    };
    Ok(Some(Entry {
        command: entry
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        args: entry
            .get("args")
            .and_then(|value| value.as_array())
            .map(|args| {
                args.iter()
                    .map(|arg| arg.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
    }))
}

fn toml_block(binary: &str) -> String {
    let args = ARGS
        .iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // The command goes through TOML's own string escaping: a Windows path is
    // full of backslashes, and `\U` in a bare quoted string is a parse error.
    let command = toml::Value::String(binary.to_string()).to_string();
    format!("[mcp_servers.{KEY}]\ncommand = {command}\nargs = [{args}]\n")
}

/// Where the `[mcp_servers.fiber]` block starts and ends, as byte offsets.
///
/// A TOML table runs from its header to the next header at any level, so the
/// end is the next line starting with `[`. Returns `None` when the block was
/// written some other way — an inline table under `[mcp_servers]`, say — which
/// the caller reports rather than guessing at.
fn toml_span(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(begin) = start {
            if trimmed.starts_with('[') {
                return Some((begin, offset));
            }
        } else {
            let header = trimmed.trim_start_matches('[').trim_end_matches(']');
            let header: String = header.chars().filter(|c| !c.is_whitespace()).collect();
            if trimmed.starts_with('[')
                && (header == format!("mcp_servers.{KEY}")
                    || header == format!("mcp_servers.\"{KEY}\""))
            {
                start = Some(offset);
            }
        }
        offset += line.len();
    }
    start.map(|begin| (begin, text.len()))
}

fn install_toml(path: &Path, binary: &str) -> Result<(), ClientError> {
    let existing = read_optional(path)?.unwrap_or_default();
    // Parsed first: an unparseable file is left alone, same as the JSON side.
    let present = parse_toml_entry(&existing, path)?.is_some();
    let block = toml_block(binary);

    let text = if present {
        let Some((start, end)) = toml_span(&existing) else {
            return Err(ClientError::HandWritten { path: pretty(path) });
        };
        format!("{}{}{}", &existing[..start], block, &existing[end..])
    } else if existing.trim().is_empty() {
        block
    } else {
        // Appended, so every comment and every other table stays where it is.
        let separator = if existing.ends_with('\n') { "\n" } else { "\n\n" };
        format!("{existing}{separator}{block}")
    };
    write_atomically(path, &text)
}

fn uninstall_toml(path: &Path) -> Result<(), ClientError> {
    let Some(existing) = read_optional(path)? else {
        return Ok(());
    };
    if parse_toml_entry(&existing, path)?.is_none() {
        return Ok(());
    }
    let Some((start, end)) = toml_span(&existing) else {
        return Err(ClientError::HandWritten { path: pretty(path) });
    };
    let mut text = format!("{}{}", &existing[..start], &existing[end..]);
    // The block took its trailing blank line with it; don't leave the file
    // ending in a growing stack of them.
    while text.ends_with("\n\n") {
        text.pop();
    }
    write_atomically(path, &text)
}

// ---------------------------------------------------------------------- Shared

fn read_entry(shape: Shape, path: &Path) -> Result<Option<Entry>, ClientError> {
    match shape {
        Shape::Codex => read_toml_entry(path),
        shape => read_json_entry(shape, path),
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, ClientError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ClientError::Read {
            path: pretty(path),
            source,
        }),
    }
}

/// Temp file and a rename, as collections are saved: a client reading its
/// config while Fiber writes it sees the old file or the new one, never half
/// of either.
fn write_atomically(path: &Path, text: &str) -> Result<(), ClientError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ClientError::Write {
            path: pretty(parent),
            source,
        })?;
    }
    let temp = path.with_extension(format!("fiber-tmp-{}", std::process::id()));
    (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()
    })()
    .map_err(|source| ClientError::Write {
        path: pretty(&temp),
        source,
    })?;
    fs::rename(&temp, path).map_err(|source| ClientError::Write {
        path: pretty(path),
        source,
    })
}

/// `~/.cursor/mcp.json` rather than the full path — shorter, and it doesn't
/// put the user's name on screen.
fn pretty(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fiber-clients-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_json_config_is_created_with_only_fiber_in_it() {
        let path = temp_dir("new-json").join("mcp.json");
        install_json(Shape::McpServers, &path, "/Applications/Fiber.app/f").unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcpServers"]["fiber"]["command"], "/Applications/Fiber.app/f");
        assert_eq!(value["mcpServers"]["fiber"]["args"][0], "mcp");
        assert_eq!(value["mcpServers"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn installing_keeps_every_other_server_and_every_other_key() {
        let path = temp_dir("merge").join("claude.json");
        fs::write(
            &path,
            r#"{"numStartups": 7, "mcpServers": {"other": {"command": "elsewhere"}}}"#,
        )
        .unwrap();
        install_json(Shape::McpServers, &path, "/bin/fiber").unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["numStartups"], 7);
        assert_eq!(value["mcpServers"]["other"]["command"], "elsewhere");
        assert_eq!(value["mcpServers"]["fiber"]["command"], "/bin/fiber");
    }

    #[test]
    fn installing_twice_replaces_the_entry_rather_than_adding_one() {
        let path = temp_dir("twice").join("mcp.json");
        install_json(Shape::McpServers, &path, "/old/fiber").unwrap();
        assert!(!read_json_entry(Shape::McpServers, &path)
            .unwrap()
            .unwrap()
            .matches("/new/fiber"));

        install_json(Shape::McpServers, &path, "/new/fiber").unwrap();
        let entry = read_json_entry(Shape::McpServers, &path).unwrap().unwrap();
        assert!(entry.matches("/new/fiber"));
    }

    #[test]
    fn vs_code_gets_its_own_key_and_a_named_transport() {
        let path = temp_dir("vscode").join("mcp.json");
        install_json(Shape::VsCode, &path, "/bin/fiber").unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["servers"]["fiber"]["type"], "stdio");
        assert!(value.get("mcpServers").is_none());
    }

    #[test]
    fn uninstalling_leaves_the_other_servers_behind() {
        let path = temp_dir("uninstall").join("mcp.json");
        fs::write(&path, r#"{"mcpServers": {"other": {"command": "elsewhere"}}}"#).unwrap();
        install_json(Shape::McpServers, &path, "/bin/fiber").unwrap();
        uninstall_json(Shape::McpServers, &path).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["mcpServers"].get("fiber").is_none());
        assert_eq!(value["mcpServers"]["other"]["command"], "elsewhere");
    }

    #[test]
    fn a_config_that_does_not_parse_is_never_rewritten() {
        let path = temp_dir("corrupt").join("mcp.json");
        // A comment: legal in VS Code's own file, not in JSON.
        let original = "{\n  // servers\n  \"servers\": {}\n}";
        fs::write(&path, original).unwrap();

        let err = install_json(Shape::VsCode, &path, "/bin/fiber").unwrap_err();
        assert!(matches!(err, ClientError::Corrupt { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn a_server_map_that_is_not_a_map_is_refused_rather_than_replaced() {
        let path = temp_dir("wrong-shape").join("mcp.json");
        fs::write(&path, r#"{"mcpServers": []}"#).unwrap();

        assert!(matches!(
            install_json(Shape::McpServers, &path, "/bin/fiber"),
            Err(ClientError::Corrupt { .. })
        ));
    }

    #[test]
    fn codex_keeps_its_comments_and_its_key_order() {
        let path = temp_dir("codex").join("config.toml");
        let original = "# my settings\nmodel = \"gpt-5\"\napproval = \"never\"\n";
        fs::write(&path, original).unwrap();
        install_toml(&path, "/bin/fiber").unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(original), "{text}");
        assert!(text.contains("[mcp_servers.fiber]"));
        let entry = read_toml_entry(&path).unwrap().unwrap();
        assert!(entry.matches("/bin/fiber"));
    }

    #[test]
    fn codex_replaces_only_its_own_block() {
        let path = temp_dir("codex-replace").join("config.toml");
        fs::write(
            &path,
            "[mcp_servers.fiber]\ncommand = \"/old/fiber\"\nargs = [\"mcp\"]\n\n[mcp_servers.other]\ncommand = \"keep\"\n",
        )
        .unwrap();
        install_toml(&path, "/new/fiber").unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[mcp_servers.other]"), "{text}");
        assert!(!text.contains("/old/fiber"), "{text}");
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(table["mcp_servers"]["other"]["command"].as_str(), Some("keep"));
        assert_eq!(
            table["mcp_servers"]["fiber"]["command"].as_str(),
            Some("/new/fiber")
        );
    }

    #[test]
    fn removing_from_codex_takes_the_block_and_nothing_else() {
        let path = temp_dir("codex-remove").join("config.toml");
        fs::write(&path, "model = \"gpt-5\"\n").unwrap();
        install_toml(&path, "/bin/fiber").unwrap();
        uninstall_toml(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "model = \"gpt-5\"\n");
        assert!(read_toml_entry(&path).unwrap().is_none());
    }

    #[test]
    fn a_windows_path_survives_toml_escaping() {
        let path = temp_dir("codex-windows").join("config.toml");
        install_toml(&path, r"C:\Users\me\Fiber\fiber.exe").unwrap();

        let entry = read_toml_entry(&path).unwrap().unwrap();
        assert!(entry.matches(r"C:\Users\me\Fiber\fiber.exe"));
    }

    #[test]
    fn an_inline_codex_entry_is_reported_rather_than_mangled() {
        let path = temp_dir("codex-inline").join("config.toml");
        let original = "[mcp_servers]\nfiber = { command = \"/old/fiber\", args = [\"mcp\"] }\n";
        fs::write(&path, original).unwrap();

        assert!(matches!(
            install_toml(&path, "/new/fiber"),
            Err(ClientError::HandWritten { .. })
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn removing_something_that_was_never_there_is_not_an_error() {
        let dir = temp_dir("absent");
        let json = dir.join("mcp.json");
        let config = dir.join("config.toml");
        uninstall_json(Shape::McpServers, &json).unwrap();
        uninstall_toml(&config).unwrap();
        assert!(!json.exists());
        assert!(!config.exists());
    }

    #[test]
    fn every_client_knows_where_its_config_lives() {
        for client in CLIENTS {
            assert!(
                config_path(client.id).is_some(),
                "{} has no config path",
                client.id
            );
        }
        assert!(def("nonesuch").is_err());
    }
}
