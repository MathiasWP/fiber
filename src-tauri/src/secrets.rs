//! Secrets, in the OS keychain.
//!
//! Section files hold a *reference* — never a value — so a collection is safe to
//! share, safe to sync into a backup, and safe for the MCP server to read.
//!
//! The UI writes them and asks whether one exists; only Rust ever sees the
//! value, on its way into an outgoing request. There is deliberately no command
//! that hands one back to the frontend.
//!
//! `fiber mcp export-secrets` is the single exception, and lives in mcp.rs
//! rather than here so that it stays a deployment tool rather than an API: a
//! container cannot reach the keychain, and the alternative was every user
//! copying the same values out by hand. See mcp::export_secrets.

use std::collections::HashMap;
use std::sync::OnceLock;

const SERVICE: &str = "dev.fiber.app";

/// The app-managed credential file, in the data directory a containerised
/// server already mounts. See `vault` for why it is sealed rather than plain.
pub const FILE_NAME: &str = "mcp-secrets.enc";

/// Keychain reference for the key that seals that file. Not a section's
/// credential, so it is namespaced away from the `<sectionId>:auth` references.
pub const KEY_REF: &str = "mcp:file-key";

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

pub fn set(reference: &str, value: &str) -> Result<(), SecretError> {
    entry(reference)?.set_password(value)?;
    Ok(())
}

/// Authenticated encryption for the credential file.
///
/// The file lives in the mounted collections directory, which is the only
/// channel the desktop app and a containerised server both reach — the app
/// cannot write to ToolHive's secret store, and the container cannot read the
/// keychain. Putting credentials there in the clear would undo the point of
/// keeping them in a keychain at all, so the file carries ciphertext and the
/// key stays out of the mount: in the keychain on the app's side, in ToolHive's
/// encrypted store on the container's. A copy of the file on its own is inert.
///
/// The key is long-lived and the values rotate underneath it, which is the
/// whole trick: signing in again rewrites the file, and nothing has to re-issue
/// a secret or restart a workload.
mod vault {
    use base64::Engine as _;
    use chacha20poly1305::aead::{Aead, Generate, KeyInit};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

    /// Version-tagged so a future format change is a clear error rather than a
    /// decryption failure, and so a plaintext file is recognisably not this.
    const MAGIC: &str = "FIBER-SECRETS-1 ";
    const NONCE_LEN: usize = 24;

    fn b64() -> &'static base64::engine::general_purpose::GeneralPurpose {
        &base64::engine::general_purpose::STANDARD
    }

    /// A fresh 32-byte key, base64 for a pipe and an environment variable.
    pub fn new_key() -> Result<String, String> {
        let key = Key::try_generate().map_err(|err| format!("no system randomness: {err}"))?;
        Ok(b64().encode(key.as_slice()))
    }

    fn cipher(key: &str) -> Result<XChaCha20Poly1305, String> {
        let raw = b64()
            .decode(key.trim())
            .map_err(|_| "the secrets key is not valid base64".to_string())?;
        XChaCha20Poly1305::new_from_slice(&raw)
            .map_err(|_| format!("the secrets key must be 32 bytes, got {}", raw.len()))
    }

    pub fn is_sealed(document: &str) -> bool {
        document.trim_start().starts_with(MAGIC)
    }

    pub fn seal(key: &str, plaintext: &str) -> Result<String, String> {
        let nonce = XNonce::try_generate().map_err(|err| format!("no system randomness: {err}"))?;
        let sealed = cipher(key)?
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| "could not encrypt the secrets file".to_string())?;
        let mut body = Vec::with_capacity(NONCE_LEN + sealed.len());
        body.extend_from_slice(nonce.as_slice());
        body.extend_from_slice(&sealed);
        Ok(format!("{MAGIC}{}\n", b64().encode(&body)))
    }

    pub fn open(key: &str, document: &str) -> Result<String, String> {
        let body = document
            .trim()
            .strip_prefix(MAGIC.trim_end())
            .ok_or("the secrets file is not in FIBER-SECRETS-1 format")?;
        let body = b64()
            .decode(body.trim())
            .map_err(|_| "the secrets file is not valid base64".to_string())?;
        if body.len() <= NONCE_LEN {
            return Err("the secrets file is truncated".into());
        }
        let (nonce, sealed) = body.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| "bad nonce".to_string())?;
        let plain = cipher(key)?.decrypt(&nonce, sealed).map_err(|_| {
            "could not decrypt the secrets file — FIBER_SECRETS_KEY does not match it".to_string()
        })?;
        String::from_utf8(plain).map_err(|_| "the secrets file did not decrypt to text".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_sealed_document_round_trips() {
            let key = new_key().unwrap();
            let sealed = seal(&key, r#"{"sec-1:auth":"tok"}"#).unwrap();
            assert!(is_sealed(&sealed), "it should announce its own format");
            assert!(
                !sealed.contains("tok"),
                "the token must not survive in the clear: {sealed}"
            );
            assert_eq!(open(&key, &sealed).unwrap(), r#"{"sec-1:auth":"tok"}"#);
        }

        /// The file sits in a directory that is mounted, synced and backed up.
        /// A copy of it without the key has to be worthless, and a copy that
        /// someone has edited has to fail loudly rather than decrypt to
        /// something else.
        #[test]
        fn another_key_or_a_tampered_body_is_rejected() {
            let key = new_key().unwrap();
            let sealed = seal(&key, r#"{"sec-1:auth":"tok"}"#).unwrap();

            assert!(open(&new_key().unwrap(), &sealed).is_err(), "wrong key");

            // Flip a byte in the middle of the ciphertext.
            let mut tampered: Vec<char> = sealed.chars().collect();
            let at = tampered.len() / 2;
            tampered[at] = if tampered[at] == 'A' { 'B' } else { 'A' };
            let tampered: String = tampered.into_iter().collect();
            assert!(
                open(&key, &tampered).is_err(),
                "authentication must catch it"
            );
        }

        /// Plaintext must not be mistaken for a sealed file — that is the
        /// downgrade this format exists to make visible.
        #[test]
        fn plaintext_is_not_mistaken_for_a_sealed_file() {
            assert!(!is_sealed(r#"{"sec-1:auth":"tok"}"#));
            assert!(open(&new_key().unwrap(), r#"{"sec-1:auth":"tok"}"#).is_err());
        }
    }
}

pub use vault::{is_sealed, new_key, open, seal};

/// Secrets supplied through the environment, for headless runs where the OS
/// keychain isn't reachable — most importantly the MCP server inside a
/// container, where a manager like ToolHive injects them. `FIBER_SECRETS` holds
/// a JSON object of `reference -> value`; `FIBER_SECRETS_FILE` points at a file
/// with the same, optionally sealed with `FIBER_SECRETS_KEY`.
///
/// Both are unset in the desktop app, which uses only the keychain, so its
/// behaviour is unchanged.
fn env_result() -> &'static Result<HashMap<String, String>, String> {
    // A process's own environment cannot change under it, so this is read once.
    // The *file* is the live half — see `file_secrets`.
    static INJECTED: OnceLock<Result<HashMap<String, String>, String>> = OnceLock::new();
    INJECTED.get_or_init(|| match std::env::var("FIBER_SECRETS") {
        Ok(raw) => parse_injected(&raw),
        Err(_) => Ok(HashMap::new()),
    })
}

/// The credential file, read afresh every time it is asked for.
///
/// This is what makes signing in again take effect in a running containerised
/// server. The old behaviour read `FIBER_SECRETS` once into a `OnceLock`, so a
/// container held whatever was true when it started: the user signed in, the
/// keychain got the new token, and the server went on presenting the expired
/// one until someone re-exported the secrets and restarted the workload. A
/// mounted file changes under a running process, so re-reading it is half the
/// fix; the other half is `send::send_authenticated_streaming` dropping the
/// cached token on a 401, which is what sends anyone back here to look.
///
/// Deliberately uncached, unlike the mtime+size stamp `mcp::all_sections` uses
/// for collections. Two reasons. It is not hot: `auth::header_for` only reaches
/// a lookup when `AuthState` has no live token, which is once per collection
/// per run plus each 401 — everything else is answered from memory. And a
/// stamp would be *wrong* here in a way it is not for collections: one token
/// replaced by another of the same length within a single mtime tick is the
/// ordinary case for a refreshed JWT or session cookie, and a bind mount can
/// coarsen mtime to the second. Missing that change is the bug this function
/// exists to fix, so it is not worth reintroducing to save a kilobyte read.
fn file_secrets() -> Result<HashMap<String, String>, String> {
    let Some(path) = std::env::var_os("FIBER_SECRETS_FILE") else {
        return Ok(HashMap::new());
    };
    // Absent is not an error. The app only writes the file once someone has set
    // a container up, and a server pointed at a path that isn't there yet
    // should say "no credentials", not refuse to start.
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(err) => return Err(format!("could not read FIBER_SECRETS_FILE: {err}")),
    };
    decode_file(&raw, std::env::var("FIBER_SECRETS_KEY").ok().as_deref())
}

/// The key is passed in rather than read here, so the format rules — including
/// the two ways a file and a key can disagree — are testable without setting
/// process-wide environment variables.
fn decode_file(raw: &str, key: Option<&str>) -> Result<HashMap<String, String>, String> {
    match (key, is_sealed(raw)) {
        (Some(key), true) => parse_injected(&open(key.trim(), raw)?),
        // With a key set, plaintext is refused rather than accepted. Otherwise
        // swapping the file for an unencrypted one of an attacker's choosing
        // would be a silent downgrade, and encrypting it would buy nothing.
        (Some(_), false) => Err("FIBER_SECRETS_KEY is set but the secrets file is not \
             encrypted. Rewrite it with `fiber mcp export-secrets --to <path>`."
            .into()),
        (None, true) => Err(
            "the secrets file is encrypted but FIBER_SECRETS_KEY is not set. \
             Pass it, e.g. `--secret fiber-key,target=FIBER_SECRETS_KEY`."
                .into(),
        ),
        (None, false) => parse_injected(raw),
    }
}

/// The injected-secrets document — from either source — is a flat JSON object
/// of string values; anything else is a startup error in headless mode.
/// Silently treating a typo as "no credentials" makes every authenticated call
/// fail for an unrelated, invisible reason.
fn parse_injected(raw: &str) -> Result<HashMap<String, String>, String> {
    serde_json::from_str(raw)
        .map_err(|err| format!("injected secrets must be a JSON object of string values: {err}"))
}

pub fn validate_injected() -> Result<(), String> {
    env_result().as_ref().map(|_| ()).map_err(Clone::clone)?;
    file_secrets().map(|_| ())
}

/// Injected values, environment first. Returns nothing on a malformed file
/// rather than propagating: `validate_injected` has already refused to start
/// the server for that, and a file that goes bad while running should fail the
/// request that needed it, not every lookup that didn't.
fn injected(reference: &str) -> Option<String> {
    if let Some(value) = env_result().as_ref().ok()?.get(reference) {
        return Some(value.clone());
    }
    file_secrets().ok()?.get(reference).cloned()
}

/// Whether credentials are coming from somewhere that can change underneath a
/// running process — which in practice means "is this the containerised server".
///
/// The desktop app has neither variable set, so callers can use this to take a
/// step that would be wrong there. See `send::send_authenticated_streaming`:
/// dropping a cached token on a 401 costs a container one file read and costs
/// the app a keychain prompt.
pub fn has_injected_source() -> bool {
    std::env::var_os("FIBER_SECRETS").is_some() || std::env::var_os("FIBER_SECRETS_FILE").is_some()
}

/// `None` when absent, which is not an error — an unconfigured section is a
/// normal state.
pub fn get(reference: &str) -> Option<String> {
    // Injected secrets win, so a containerised MCP server never has to reach a
    // keychain it can't see.
    if let Some(value) = injected(reference) {
        return Some(value);
    }
    entry(reference).ok()?.get_password().ok()
}

/// Whether a secret exists — deliberately without reading it.
///
/// `get(..).is_some()` would answer the same question and cost a great deal
/// more: fetching a keychain item's *data* needs authorization, and on a build
/// signed ad-hoc — where the ACL cannot survive an update — that authorization
/// is a password prompt. Fetching its *attributes* needs none. The UI only ever
/// asks whether something is there, so that is all this looks at.
#[cfg(target_os = "macos")]
pub fn has(reference: &str) -> bool {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

    // Injected secrets never touch the keychain, so answer for them first.
    if injected(reference).is_some() {
        return true;
    }

    ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(reference)
        // Attributes only. Adding `load_data` here would put the prompt straight
        // back, which is the entire point of this function.
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search()
        .map(|found| !found.is_empty())
        // A missing item searches as an error rather than an empty result.
        .unwrap_or(false)
}

/// Elsewhere there is no such distinction to exploit, and no prompt to avoid.
#[cfg(not(target_os = "macos"))]
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
        let map = parse_injected(r#"{"sec-1:auth":"tok-123","sec-2:login":"{\"user\":\"me\"}"}"#)
            .unwrap();
        assert_eq!(map.get("sec-1:auth").map(String::as_str), Some("tok-123"));
        assert_eq!(
            map.get("sec-2:login").map(String::as_str),
            Some(r#"{"user":"me"}"#)
        );
    }

    #[test]
    fn malformed_injected_secrets_are_rejected() {
        assert!(parse_injected("not json").is_err());
        // Right JSON, wrong shape: a value must be a string.
        assert!(parse_injected(r#"{"sec-1:auth":123}"#).is_err());
        assert!(parse_injected("[]").is_err());
    }

    /// The whole point of the file: a value replaced on disk is the value the
    /// next lookup gets. A cache keyed on mtime+size would pass a test that
    /// changed the length and fail this one, which is why there isn't one.
    #[test]
    fn a_rewritten_file_is_read_again() {
        let key = new_key().unwrap();
        let before = decode_file(
            &seal(&key, r#"{"s:auth":"old-token"}"#).unwrap(),
            Some(&key),
        );
        assert_eq!(before.unwrap().get("s:auth").unwrap(), "old-token");

        // Same length, as a refreshed JWT or session cookie usually is.
        let after = decode_file(
            &seal(&key, r#"{"s:auth":"new-token"}"#).unwrap(),
            Some(&key),
        );
        assert_eq!(after.unwrap().get("s:auth").unwrap(), "new-token");
    }

    /// A key without an encrypted file is a downgrade — someone swapping the
    /// sealed file for one of their own — and an encrypted file without a key
    /// is a container missing its `--secret`. Both have to be named, because
    /// "no credentials" would send every request out unauthenticated instead.
    #[test]
    fn a_file_and_a_key_that_disagree_are_both_refused() {
        let key = new_key().unwrap();
        let sealed = seal(&key, r#"{"s:auth":"tok"}"#).unwrap();
        let plain = r#"{"s:auth":"tok"}"#;

        assert!(
            decode_file(plain, Some(&key)).is_err(),
            "plaintext with a key"
        );
        assert!(decode_file(&sealed, None).is_err(), "sealed without a key");
        // And the two that agree still work, in both directions.
        assert!(decode_file(plain, None).is_ok());
        assert!(decode_file(&sealed, Some(&key)).is_ok());
    }

    /// A path that is not there yet is the normal state before anyone has set a
    /// container up, and must not stop the server from starting.
    #[test]
    fn a_missing_secrets_file_is_not_an_error() {
        let path = std::env::temp_dir().join("fiber-secrets-absent.json");
        let _ = std::fs::remove_file(&path);
        // SAFETY: single-threaded test setup; nothing else reads this variable
        // until `file_secrets` does, on the next line.
        unsafe { std::env::set_var("FIBER_SECRETS_FILE", &path) };
        let found = file_secrets();
        unsafe { std::env::remove_var("FIBER_SECRETS_FILE") };
        assert!(found.unwrap().is_empty());
    }
}
