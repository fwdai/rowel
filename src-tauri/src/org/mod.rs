//! Org lockboxes: the parts of the app that speak to a company's Rowel Server
//! as a *member*, rather than as a sync client.
//!
//! A member signs in to the server once per install ([`client`]: an OAuth2
//! authorization-code sign-in the server fronts, federating to the company's
//! identity provider on its own side, so the app never meets the provider;
//! [`connect`] is the desktop flow that runs it), enrolls this device's X25519
//! key with it ([`enrollment`]), and keeps the session and the device's
//! standing sealed under the app key ([`connections`]). Every org lockbox the
//! server then hands the member is one more non-primary workspace
//! ([`lockboxes`]): its vault id is the server's lockbox id, its pack syncs
//! through `sync::providers::rowel` under that session, and its master is
//! unsealed by the enrollment key rather than derived from a password
//! (`KdfParams::External`). While the vault is open, the server's approvals
//! for this member reach the app's own grant dialog ([`approvals`]).
//!
//! The pieces take `(root, app_key)` and talk HTTP; the places an `AppHandle`
//! appears are wrappers that fetch those two from the running app. The
//! commands over all of it are `commands::org`.

use std::path::Path;

#[cfg(desktop)]
pub(crate) mod approvals;
pub(crate) mod client;
#[cfg(desktop)]
pub(crate) mod connect;
pub(crate) mod connections;
pub(crate) mod enrollment;
#[cfg(test)]
pub(crate) mod fake_server;
pub(crate) mod lockboxes;
#[cfg(test)]
mod tests;

/// Every root-level file sealed under the app key — the server connections
/// and this device's enrollment key — re-sealed under `new` after a change
/// of the primary's key replaced `old` (`appkey::replace_app_key`). Best
/// effort, each on its own: a file that will not move is logged, and is set
/// aside at the next primary unlock ([`set_aside_stale`]), after which the
/// member signs in again.
pub(crate) fn reseal_under(root: &Path, old: &[u8], new: &[u8]) {
    if let Err(e) = connections::reseal(root, old, new) {
        log::warn!("could not re-seal the server connections under the new app key: {e}");
    }
    if let Err(e) = enrollment::reseal(root, old, new) {
        log::warn!("could not re-seal the enrollment key under the new app key: {e}");
    }
}

/// The same two files, where `app_key` — just proved to be the app key —
/// does not open them: moved aside (`storage::set_aside`) so the next
/// sign-in starts clean, and never deleted.
///
/// Moved only on the verdict that the file is whole but fails its tag under
/// this key (`Error::SealedUnderAnotherKey`), which is what deleting the
/// primary leaves behind. A read that fails, or a file too short or too
/// garbled to be a sealed blob, says nothing about the key, and stays where it
/// is. Even the verdict is not proof: to an AEAD, a blob corrupted in place
/// fails its tag exactly as one sealed under another key does, and nothing in
/// the file tells the two apart. So a stale file is renamed rather than
/// removed — a wrong verdict then costs a sign-in and loses no bytes, which
/// matters most for the enrollment key, the private half every org lockbox on
/// the device is wrapped to.
pub(crate) fn set_aside_stale(root: &Path, app_key: &[u8]) {
    set_aside_if_stale(
        &root.join(crate::storage::SERVERS_FILE),
        "the server connections",
        || connections::load(root, app_key).map(drop),
    );
    set_aside_if_stale(
        &root.join(crate::storage::ENROLLMENT_KEY_FILE),
        "this device's enrollment key",
        || enrollment::load_or_create(root, app_key).map(drop),
    );
}

// `open` is only asked of a file that exists, so the enrollment key's
// load-or-create never mints one here.
fn set_aside_if_stale(path: &Path, what: &str, open: impl FnOnce() -> crate::error::Result<()>) {
    if !path.exists() {
        return;
    }
    match open() {
        Ok(()) => {}
        Err(crate::error::Error::SealedUnderAnotherKey) => {
            match crate::storage::set_aside(path) {
                Ok(aside) => log::warn!(
                    "{what} are sealed under an app key this device no longer has; set aside as {}",
                    aside.display()
                ),
                Err(e) => log::warn!(
                    "{what} are sealed under an app key this device no longer has, and could not be set aside: {e}"
                ),
            }
        }
        Err(e) => log::warn!("{what} could not be read and are left as they are: {e}"),
    }
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    use crate::storage;

    const APP_KEY: &[u8] = &[3u8; 32];
    const OTHER_KEY: &[u8] = &[5u8; 32];

    fn sealed_files(root: &Path, key: &[u8]) {
        connections::upsert(
            root,
            key,
            connections::tests::connection("https://a.example", "a1"),
        )
        .unwrap();
        enrollment::load_or_create(root, key).unwrap();
    }

    fn stale_copies(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.contains(".stale-"))
            .collect();
        names.sort();
        names
    }

    // Files sealed under a key the device no longer has are moved aside with
    // their bytes intact, and a file that opens is left where it is.
    #[test]
    fn a_file_sealed_under_another_key_is_set_aside_not_deleted() {
        let root = tempfile::tempdir().unwrap();
        sealed_files(root.path(), OTHER_KEY);
        let enrollment_path = root.path().join(storage::ENROLLMENT_KEY_FILE);
        let before = std::fs::read(&enrollment_path).unwrap();

        set_aside_stale(root.path(), OTHER_KEY);
        assert!(
            stale_copies(root.path()).is_empty(),
            "they open, so they stay"
        );

        set_aside_stale(root.path(), APP_KEY);
        assert!(!enrollment_path.exists());
        assert!(!root.path().join(storage::SERVERS_FILE).exists());
        let aside = stale_copies(root.path());
        assert_eq!(aside.len(), 2, "{aside:?}");
        let kept = aside
            .iter()
            .find(|name| name.starts_with(storage::ENROLLMENT_KEY_FILE))
            .unwrap();
        assert_eq!(std::fs::read(root.path().join(kept)).unwrap(), before);
        // And the next sign-in has a clean slate to enroll a fresh key in.
        assert!(enrollment::load_or_create(root.path(), APP_KEY).is_ok());
    }

    // A file the key verdict cannot be reached for — unreadable, not base64,
    // too short to hold a nonce and a tag — says nothing about the key.
    #[test]
    fn a_file_not_known_to_be_stale_stays_where_it_is() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(storage::SERVERS_FILE);
        for content in ["not base64!", "AAAA"] {
            std::fs::write(&path, content).unwrap();
            set_aside_stale(root.path(), APP_KEY);
            assert!(path.exists(), "{content:?} stays");
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        set_aside_stale(root.path(), APP_KEY);
        assert!(path.is_dir(), "unreadable, left alone");
        assert!(stale_copies(root.path()).is_empty());
    }

    // A file corrupted in place fails its tag as one under another key does,
    // and is set aside the same way: the rename is what makes a wrong verdict
    // cost nothing but a sign-in.
    #[test]
    fn a_file_corrupted_in_place_keeps_its_bytes() {
        let root = tempfile::tempdir().unwrap();
        sealed_files(root.path(), APP_KEY);
        let path = root.path().join(storage::ENROLLMENT_KEY_FILE);
        let mut sealed = std::fs::read_to_string(&path).unwrap().into_bytes();
        // A character in the middle: the one before the padding carries
        // bits a strict base64 reader requires to be zero, and flipping it
        // would make the file malformed rather than corrupt.
        let middle = sealed.len() / 2;
        sealed[middle] = if sealed[middle] == b'A' { b'B' } else { b'A' };
        std::fs::write(&path, &sealed).unwrap();

        set_aside_stale(root.path(), APP_KEY);
        let aside = stale_copies(root.path());
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read(root.path().join(&aside[0])).unwrap(), sealed);
        assert!(root.path().join(storage::SERVERS_FILE).exists(), "it opens");
    }
}
