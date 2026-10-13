//! The shape of Rowel's footprint in a Google Drive account.
//!
//! ```text
//! Rowel/
//!   Lockboxes/             (`Vaults/` on accounts created before the rename)
//!     <vault-id>.rowel     one pack per vault
//!   Shares/
//!     <random>.rowelshare              one outstanding share
//! ```
//!
//! Every folder name, file name and extension Drive ever sees is minted here,
//! so the sync engine, first-run onboarding and sharing cannot drift apart —
//! and so the local `.rowel` backup and the pack on Drive, which are the same
//! bytes, carry the same extension.
//!
//! A vault is addressed by its *vault id*: the identity of the data, stored in
//! the encrypted `meta` table (see [`crate::store::identity`]) and carried
//! inside every pack. It is deliberately not the local workspace id, which is
//! `default` on every install and cannot tell two devices' primaries apart.
//! The folder shows the opaque id and never the user's label for the vault.
//!
//! Pure: names in, names out. Nothing here talks to Drive.

/// The one folder Rowel owns in the account.
pub const ROOT_FOLDER: &str = "Rowel";
/// Subfolder of [`ROOT_FOLDER`] holding one pack per lockbox, and the only
/// one a fresh account is given.
pub const LOCKBOXES_FOLDER: &str = "Lockboxes";
/// What [`LOCKBOXES_FOLDER`] was called before the lockbox rename. Accounts
/// created then keep it and go on syncing in it.
///
/// Deliberately not renamed in place (a `files.update` of the folder's name):
/// an older client that finds `Rowel/` but no `Vaults/` reads the account as
/// never pushed, recreates `Vaults/` and uploads its own pack there, and the
/// two devices fork without a word. The rename becomes safe once clients that
/// only know this name are out of support; until then it is read, never moved.
pub const LEGACY_VAULTS_FOLDER: &str = "Vaults";
/// Where to look for the packs folder, most preferred first. When an account
/// has both (two devices raced across the rename, or a user made one by hand)
/// [`LOCKBOXES_FOLDER`] wins, on every device alike.
pub const PACKS_FOLDERS: [&str; 2] = [LOCKBOXES_FOLDER, LEGACY_VAULTS_FOLDER];
/// Subfolder of [`ROOT_FOLDER`] holding outstanding shares.
pub const SHARES_FOLDER: &str = "Shares";

/// Extension of a vault pack — on Drive, and of the backup `export_vault`
/// writes. Also the desktop file association in `tauri.conf.json`; keep the
/// two in step.
pub const VAULT_EXTENSION: &str = "rowel";
/// Extension of a sealed share envelope.
pub const SHARE_EXTENSION: &str = "rowelshare";
/// Extension of the marker left where a deleted vault's pack used to be.
pub const DELETED_EXTENSION: &str = "deleted";
/// MIME type a vault pack is uploaded under; the same one the desktop file
/// association declares.
pub const VAULT_MIME: &str = "application/vnd.rowel";

/// The live pack of the vault with this id: `<vault-id>.rowel`.
pub fn vault_file_name(vault_id: &str) -> String {
    format!("{vault_id}.{VAULT_EXTENSION}")
}

/// The vault id a file in the packs folder ([`PACKS_FOLDERS`]) is the pack of,
/// if it is one.
///
/// Exact: `<id>.rowel` where `<id>` is non-empty lowercase hex. This is the
/// user's own Drive folder, so anything else they have put in it — a copy, a
/// share, a note — reads as `None` rather than as a vault.
pub fn vault_id_of(file_name: &str) -> Option<&str> {
    let id = file_name.strip_suffix(&format!(".{VAULT_EXTENSION}"))?;
    let is_hex = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    is_hex.then_some(id)
}

/// The marker left in place of a vault deleted from the account:
/// `<vault-id>.deleted`.
///
/// Deliberately not a `.rowel` name, so [`vault_id_of`] passes over it and
/// every listing built on that — onboarding's probe, auto-join, the "other
/// vaults in this account" offer — sees the vault as gone. The one thing that
/// reads it is a run whose own pack has disappeared: the marker is what tells
/// "another device deleted this vault" apart from "this vault has never
/// pushed", which would otherwise put the pack straight back.
pub fn deleted_marker_name(vault_id: &str) -> String {
    format!("{vault_id}.{DELETED_EXTENSION}")
}

/// A share file's name: `token` is random and the name says nothing else —
/// Drive shows the owner a file list, so it carries no title.
pub fn share_file_name(token: &str) -> String {
    format!("{token}.{SHARE_EXTENSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_pack_is_named_for_its_vault() {
        assert_eq!(vault_file_name("a1b2c3"), "a1b2c3.rowel");
        assert_eq!(vault_id_of("a1b2c3.rowel"), Some("a1b2c3"));
    }

    #[test]
    fn only_an_exact_hex_stem_reads_as_a_live_vault() {
        assert_eq!(vault_id_of("a1b2c3-copy.rowel"), None);
        assert_eq!(vault_id_of("notes.txt"), None);
        assert_eq!(vault_id_of(".rowel"), None);
        assert_eq!(vault_id_of("A1B2.rowel"), None);
        assert_eq!(vault_id_of("a1b2.rowelshare"), None);
        assert_eq!(vault_id_of("a1b2"), None);
    }

    // The marker a "delete everywhere" leaves has to be invisible to every
    // listing that asks which vaults the account holds.
    #[test]
    fn a_deleted_vaults_marker_is_not_a_pack() {
        assert_eq!(deleted_marker_name("a1b2c3"), "a1b2c3.deleted");
        assert_eq!(vault_id_of(&deleted_marker_name("a1b2c3")), None);
    }

    #[test]
    fn a_share_is_named_by_its_token_alone() {
        assert_eq!(share_file_name("deadbeef"), "deadbeef.rowelshare");
    }
}
