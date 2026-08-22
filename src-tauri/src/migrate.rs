//! One-time migration from the app's former identity.
//!
//! The app was called Fetch, which put its collections under
//! `dev.fetch.app` and its credentials in a keychain service of the same name.
//! Renaming to Fiber moves both. Without this, a rename would silently orphan
//! every collection, every stored response and every captured credential —
//! they'd still be on disk, just nowhere the app looks.
//!
//! Runs at startup in both the app and the MCP server, and does nothing once
//! the new location exists.

use std::path::Path;

/// Where data lived before the rename.
const FORMER_IDENTIFIER: &str = "dev.fetch.app";

/// Written once the keychain copy has run, so it happens exactly once per
/// install rather than only on the launch that happened to move the data
/// directory. A failed directory move used to skip the secrets forever.
const SECRETS_MARKER: &str = ".migrated-secrets";

/// Where the app's data used to live, on this machine.
fn former_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|dir| dir.join(FORMER_IDENTIFIER))
}

/// Moves the data directory across, if there's one to move and nothing in the
/// way. Returns whether anything was migrated.
pub fn data_dir(new_dir: &Path) -> bool {
    match former_data_dir() {
        Some(former) => move_across(&former, new_dir),
        None => false,
    }
}

/// The move itself, with the source injected.
///
/// Separate from [`data_dir`] so tests can never name the real directory: an
/// earlier version of this took only the destination and looked the source up
/// itself, and a test that passed a fresh temp path duly moved a real
/// collection into it.
fn move_across(former: &Path, new_dir: &Path) -> bool {
    if new_dir.exists() || !former.exists() {
        return false;
    }

    match std::fs::rename(former, new_dir) {
        Ok(()) => {
            log::info!(
                "moved collections from {} to {}",
                former.display(),
                new_dir.display()
            );
            true
        }
        // A rename can fail where a copy works — across volumes, most
        // commonly. The old directory is left in place: once the new one
        // exists nothing looks at it again, and a downgrade still finds its
        // data where it expects.
        Err(err) => {
            log::warn!(
                "could not move {} across ({err}); copying instead",
                former.display()
            );
            match copy_dir(former, new_dir) {
                Ok(()) => {
                    log::info!(
                        "copied collections from {} to {}",
                        former.display(),
                        new_dir.display()
                    );
                    true
                }
                Err(err) => {
                    log::warn!("could not copy {} across: {err}", former.display());
                    false
                }
            }
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Runs the keychain copy at most once per install, keyed on a marker file in
/// the data directory rather than on the directory move having happened this
/// launch — a move that failed (and fell back, or will succeed next time) must
/// not cost the credentials their one chance to migrate. Safe to call every
/// startup; returns whether the copy actually ran.
pub fn secrets_once(app_data_dir: &Path) -> bool {
    let marker = app_data_dir.join(SECRETS_MARKER);
    if marker.exists() {
        return false;
    }

    let sections = crate::store::sections_dir(app_data_dir);
    secrets(references(&sections).into_iter());

    // The marker is written even when nothing was found: `secrets` itself
    // never clobbers, so a retry would be harmless — just a pointless keychain
    // walk on every launch from here on.
    if let Err(err) =
        std::fs::create_dir_all(app_data_dir).and_then(|()| std::fs::write(&marker, b""))
    {
        log::warn!("could not write {}: {err}", marker.display());
    }
    true
}

/// Copies credentials across for the references the given sections name.
///
/// The keychain can't be enumerated by service, but it doesn't need to be: the
/// section files say which references exist, and that's the complete set.
/// Copies rather than moves, so a downgrade still works.
pub fn secrets(references: impl Iterator<Item = String>) -> usize {
    let mut moved = 0;
    for reference in references {
        // Anything already in the new service wins; never clobber.
        if crate::secrets::has(&reference) {
            continue;
        }
        let Some(value) = crate::secrets::get_from(FORMER_IDENTIFIER, &reference) else {
            continue;
        };
        if crate::secrets::set(&reference, &value).is_ok() {
            moved += 1;
        }
    }
    if moved > 0 {
        log::info!("carried {moved} credential(s) across from the former keychain service");
    }
    moved
}

/// Every keychain reference the collections in `sections_dir` name.
pub fn references(sections_dir: &Path) -> Vec<String> {
    crate::store::load_all(sections_dir)
        .unwrap_or_default()
        .iter()
        .filter_map(|section| section.auth.secret_ref().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fiber-migrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn moves_collections_across() {
        let former = scratch("from");
        let current = scratch("to");
        std::fs::create_dir_all(former.join("sections")).unwrap();
        std::fs::write(former.join("sections/acme.toml"), "id = \"acme\"").unwrap();

        assert!(move_across(&former, &current));
        assert!(current.join("sections/acme.toml").exists(), "content came too");
        assert!(!former.exists(), "and the old location is gone");

        let _ = std::fs::remove_dir_all(current);
    }

    #[test]
    fn never_clobbers_an_existing_directory() {
        let former = scratch("occupied-from");
        let current = scratch("occupied-to");
        std::fs::create_dir_all(&former).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("keep.txt"), "current data").unwrap();

        // An existing destination means this already ran, or it's a fresh
        // install. Either way the current data wins.
        assert!(!move_across(&former, &current));
        assert_eq!(
            std::fs::read_to_string(current.join("keep.txt")).unwrap(),
            "current data"
        );

        let _ = std::fs::remove_dir_all(former);
        let _ = std::fs::remove_dir_all(current);
    }

    /// A rename can fail — across volumes, say — and the migration must not
    /// give up there. A destination whose parent doesn't exist makes the
    /// rename fail the same way, which lets the fallback be exercised without
    /// a second disk.
    #[test]
    fn falls_back_to_copying_when_the_rename_fails() {
        let former = scratch("copy-from");
        let current = scratch("copy-to").join("deep/target");
        std::fs::create_dir_all(former.join("sections")).unwrap();
        std::fs::write(former.join("sections/acme.toml"), "id = \"acme\"").unwrap();

        assert!(move_across(&former, &current));
        assert_eq!(
            std::fs::read_to_string(current.join("sections/acme.toml")).unwrap(),
            "id = \"acme\""
        );
        // A copy leaves the source: nothing reads it once the new dir exists,
        // and a downgrade still finds its data.
        assert!(former.exists());

        let _ = std::fs::remove_dir_all(former);
        let _ = std::fs::remove_dir_all(current);
    }

    /// The keychain copy runs once per install, marker-keyed — not "once, if
    /// the directory happened to move this launch".
    #[test]
    fn the_secrets_migration_runs_exactly_once() {
        // An empty data dir: no sections, so no references, so `secrets_once`
        // never actually reaches the real keychain here.
        let dir = scratch("secrets-once");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(secrets_once(&dir), "first launch runs it");
        assert!(dir.join(SECRETS_MARKER).exists());
        assert!(!secrets_once(&dir), "every later launch skips it");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn does_nothing_when_there_is_nothing_to_move() {
        let former = scratch("missing-from");
        let current = scratch("missing-to");
        assert!(!move_across(&former, &current));
        assert!(!current.exists(), "a fresh install creates nothing here");
    }

    #[test]
    fn collects_the_references_collections_name() {
        let dir = scratch("refs");

        let mut section = crate::store::Section {
            id: "sec-1".into(),
            name: "Acme".into(),
            base_url: String::new(),
            collapsed: false,
            order: 0,
            auth: crate::auth::AuthConfig::Bearer {
                secret_ref: "sec-1:auth".into(),
            },
            loader: None,
            mcp: Default::default(),
            requests: vec![],
            overlay: vec![],
        ..Default::default()
        };
        crate::store::save(&dir, &section).unwrap();

        section.id = "sec-2".into();
        section.auth = crate::auth::AuthConfig::None;
        crate::store::save(&dir, &section).unwrap();

        let found = references(&dir);
        assert_eq!(found, vec!["sec-1:auth"], "only sections with auth have one");

        let _ = std::fs::remove_dir_all(dir);
    }
}
