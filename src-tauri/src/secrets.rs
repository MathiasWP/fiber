//! Secrets, in the OS keychain.
//!
//! Section files hold a *reference* — never a value — so a collection is safe to
//! share, safe to sync into a backup, and safe for the MCP server to read.
//!
//! There is deliberately no command to read a secret back out. The UI writes
//! them and asks whether one exists; only Rust ever sees the value, on its way
//! into an outgoing request.

use std::collections::HashMap;
use std::sync::OnceLock;

const SERVICE: &str = "dev.fiber.app";

#[derive(Debug, thiserror::Error)]
#[error("keychain: {0}")]
pub struct SecretError(#[from] keyring::Error);

impl serde::Serialize for SecretError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

fn entry(reference: &str) -> Result<keyring::Entry, SecretError> {
    Ok(keyring::Entry::new(SERVICE, reference)?)
}

/// Reads from a named service. Only the rename migration needs this — the app
/// itself never looks anywhere but [`SERVICE`].
pub fn get_from(service: &str, reference: &str) -> Option<String> {
    keyring::Entry::new(service, reference)
        .ok()?
        .get_password()
        .ok()
}

pub fn set(reference: &str, value: &str) -> Result<(), SecretError> {
    entry(reference)?.set_password(value)?;
    Ok(())
}

/// Secrets supplied through the environment, for headless runs where the OS
/// keychain isn't reachable — most importantly the MCP server inside a
/// container, where a manager like ToolHive injects them. `FIBER_SECRETS` holds
/// a JSON object of `reference -> value`; `FIBER_SECRETS_FILE` points at a file
/// with the same. Both are unset in the desktop app, which uses only the
/// keychain, so its behaviour is unchanged.
fn injected() -> &'static HashMap<String, String> {
    static INJECTED: OnceLock<HashMap<String, String>> = OnceLock::new();
    INJECTED.get_or_init(|| {
        let raw = std::env::var("FIBER_SECRETS").ok().or_else(|| {
            let path = std::env::var_os("FIBER_SECRETS_FILE")?;
            std::fs::read_to_string(path).ok()
        });
        raw.as_deref().map(parse_injected).unwrap_or_default()
    })
}

/// The injected-secrets document is a flat JSON object of string values;
/// anything else resolves to no injected secrets rather than an error, so a
/// malformed file can't wedge the server.
fn parse_injected(raw: &str) -> HashMap<String, String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// `None` when absent, which is not an error — an unconfigured section is a
/// normal state.
pub fn get(reference: &str) -> Option<String> {
    // Injected secrets win, so a containerised MCP server never has to reach a
    // keychain it can't see.
    if let Some(value) = injected().get(reference) {
        return Some(value.clone());
    }
    entry(reference).ok()?.get_password().ok()
}

pub fn has(reference: &str) -> bool {
    get(reference).is_some()
}

pub fn delete(reference: &str) -> Result<(), SecretError> {
    match entry(reference)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_injected_secrets_as_a_reference_map() {
        // A bearer token and a login body — both stored as string values, keyed
        // by the same reference the section file names.
        let map = parse_injected(r#"{"sec-1:auth":"tok-123","sec-2:login":"{\"user\":\"me\"}"}"#);
        assert_eq!(map.get("sec-1:auth").map(String::as_str), Some("tok-123"));
        assert_eq!(map.get("sec-2:login").map(String::as_str), Some(r#"{"user":"me"}"#));
    }

    #[test]
    fn malformed_injected_secrets_are_empty_not_fatal() {
        assert!(parse_injected("not json").is_empty());
        // Right JSON, wrong shape: a value must be a string.
        assert!(parse_injected(r#"{"sec-1:auth":123}"#).is_empty());
        assert!(parse_injected("[]").is_empty());
    }
}
