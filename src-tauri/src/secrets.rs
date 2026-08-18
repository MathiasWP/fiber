//! Secrets, in the OS keychain.
//!
//! Section files hold a *reference* — never a value — so a collection is safe to
//! share, safe to sync into a backup, and safe for the MCP server to read.
//!
//! There is deliberately no command to read a secret back out. The UI writes
//! them and asks whether one exists; only Rust ever sees the value, on its way
//! into an outgoing request.

const SERVICE: &str = "dev.fetch.app";

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

/// `None` when absent, which is not an error — an unconfigured section is a
/// normal state.
pub fn get(reference: &str) -> Option<String> {
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
