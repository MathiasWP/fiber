//! "There's a newer version" — a check, not an updater.
//!
//! Fiber ships unsigned beyond an ad-hoc signature, and a self-replacing app
//! bundle is exactly what Gatekeeper re-examines on the next launch. So this
//! deliberately stops at telling you: it asks GitHub what the latest release
//! is, compares it to the running version, and hands the frontend a URL. The
//! download stays a thing you do on purpose.
//!
//! The releases API is public and unauthenticated — the repository is public,
//! and no token ships in the app.

use std::time::Duration;

const LATEST_RELEASE: &str = "https://api.github.com/repos/MathiasWP/fiber/releases/latest";

/// A release newer than the one running.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    /// Without the `v`, to match what the app reports about itself.
    pub version: String,
    /// What's running now. Carried along so the toast can say "you're on X"
    /// without the frontend needing its own copy of the version.
    pub current: String,
    /// The release page, opened in the user's browser.
    pub url: String,
    pub notes: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not reach GitHub: {0}")]
    Network(#[from] reqwest::Error),
    #[error("unexpected response from GitHub: {0}")]
    NotJson(#[from] serde_json::Error),
    #[error("unreadable version {0:?}")]
    Version(String),
}

impl serde::Serialize for UpdateError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `0.2.10` sorts after `0.2.9`, which a string comparison gets wrong.
fn parts(version: &str) -> Result<(u64, u64, u64), UpdateError> {
    let bad = || UpdateError::Version(version.to_string());
    let mut fields = version.trim_start_matches('v').split('.');
    let mut next = || -> Result<u64, UpdateError> {
        fields
            .next()
            .ok_or_else(bad)?
            // Tolerate a `-rc.1` suffix on the patch field rather than refusing
            // to compare: the numeric part is what decides ordering here.
            .split(['-', '+'])
            .next()
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())
    };
    Ok((next()?, next()?, next()?))
}

fn is_newer(candidate: &str, current: &str) -> Result<bool, UpdateError> {
    Ok(parts(candidate)? > parts(current)?)
}

/// `None` when the running version is already the latest.
pub async fn check(current: &str) -> Result<Option<Update>, UpdateError> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("fiber/", env!("CARGO_PKG_VERSION")))
        // A background check must never hold up the app; failing quietly and
        // trying again tomorrow is the whole error strategy.
        .timeout(Duration::from_secs(10))
        .build()?;

    let response = client
        .get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    // A repository with no published release answers 404. That is an ordinary
    // state — every project is in it until its first release — not a failure.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    // Parsed by hand rather than with `.json()`: reqwest is built here without
    // its `json` feature, and the rest of the crate decodes the same way.
    let body = response.error_for_status()?.text().await?;
    let release: Release = serde_json::from_str(&body)?;

    // `/releases/latest` already excludes both, but the app is long-lived and
    // the endpoint is not ours.
    if release.draft || release.prerelease {
        return Ok(None);
    }

    if !is_newer(&release.tag_name, current)? {
        return Ok(None);
    }

    Ok(Some(Update {
        version: release.tag_name.trim_start_matches('v').to_string(),
        current: current.to_string(),
        url: release.html_url,
        notes: release.body.unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_patch_is_newer() {
        assert!(is_newer("v0.2.1", "0.2.0").unwrap());
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("v0.2.0", "0.2.0").unwrap());
    }

    #[test]
    fn an_older_release_is_not_newer() {
        assert!(!is_newer("v0.1.9", "0.2.0").unwrap());
    }

    #[test]
    fn ten_sorts_after_nine() {
        assert!(is_newer("v0.2.10", "0.2.9").unwrap());
        assert!(!is_newer("v0.2.9", "0.2.10").unwrap());
    }

    #[test]
    fn a_major_bump_beats_a_large_minor() {
        assert!(is_newer("v1.0.0", "0.99.99").unwrap());
    }

    #[test]
    fn a_prerelease_suffix_still_compares() {
        assert!(is_newer("v0.3.0-rc.1", "0.2.0").unwrap());
    }

    #[test]
    fn nonsense_is_an_error_rather_than_an_update() {
        assert!(is_newer("not-a-version", "0.2.0").is_err());
    }
}
