use serde::{Serialize, Serializer};

use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid master password")]
    InvalidPassword,

    // The vault was written by a newer build (schema ahead of this binary).
    // Distinct from InvalidPassword so the UI never blames the user's memory
    // for a version mismatch.
    #[error("vault requires a newer version of the app")]
    VaultTooNew,

    // Failed-unlock backoff (T-AUTH-3): too many wrong attempts, wait it out.
    // `retry_after_secs` rides along in the serialized payload (see `Serialize`
    // below) so the frontend can render a countdown without parsing the message.
    #[error("too many failed attempts, try again in {retry_after_secs}s")]
    TooManyAttempts { retry_after_secs: u64 },

    #[error("vault is locked")]
    Locked,

    /// The vault changed hands — a lock, an unlock, a password change — after
    /// this work was prepared against it, so the write was refused rather than
    /// landed in a vault it was not sealed for. Trying again prepares it afresh.
    #[error("the vault changed while this was running")]
    StaleSession,

    #[error("entry not found")]
    NotFound,

    #[error("operation was cancelled")]
    Cancelled,

    #[error("sync is not configured")]
    SyncNotConfigured,

    // This platform cannot do the thing at all (no OCR recognizer, no save
    // dialog that can set file permissions). Not a failure to retry.
    #[error("{0}")]
    Unsupported(String),

    // A scan read text but it was neither a card nor an identity document.
    #[error("nothing recognized")]
    Unrecognized,

    // --- conditions the user can act on --------------------------------------
    //
    // Each of these used to be an `Error::Other` carrying English prose, which
    // the webview had no choice but to print as it stood. As variants they get a
    // stable `kind`, and the copy lives in the catalogues with the rest of the
    // UI — see `src/api/errors.ts`, which is pinned against `kind()` by
    // `src/api/contract.test.ts`. The `#[error(...)]` text below is the English
    // source the kind is translated from and a line in the log; nobody is shown
    // it.
    #[error("this share link is not valid")]
    ShareLinkInvalid,

    #[error("this share has expired or was revoked")]
    ShareExpired,

    #[error("this share was made by a newer version of the app")]
    ShareTooNew,

    #[error("this share is larger than the app allows")]
    ShareTooLarge,

    #[error("this entry is too large to share")]
    EntryTooLargeToShare,

    /// A share published by a different vault than the one asking to delete it.
    /// One Drive account holds every vault's shares in one folder, and the file
    /// id in a revoke is whatever the caller sent, so ownership is read back off
    /// Drive rather than assumed — see the app's `share::revoke`.
    #[error("this share was published by another vault")]
    ShareNotOwned,

    /// Sharing asked for by a vault that has no id yet. The id is written by the
    /// first successful sync, and every share is stamped with it, so a link
    /// published before that run would be one no vault could later name as its
    /// own — see the app's `commands::share`.
    #[error("sync this vault once before sharing from it")]
    ShareNeedsSync,

    #[error("another setup step is still running")]
    SetupBusy,

    #[error("wait for the sync in progress to finish")]
    SyncBusy,

    // Provider-neutral: the pending account may be Google's or Dropbox's, and
    // this error has no way to know which.
    #[error("connect a cloud account first")]
    AccountNotConnected,

    #[error("this Google account has no data to restore")]
    NoRemoteVault,

    #[error("this device is already set up")]
    AlreadySetUp,

    #[error("available in the primary workspace only")]
    PrimaryWorkspaceOnly,

    #[error("a workspace needs a name")]
    WorkspaceNameRequired,

    #[error("a workspace needs a master password")]
    WorkspacePasswordRequired,

    /// A delete aimed at the only workspace this device has. The app has to
    /// have a vault to open, and there would be no way back to one from an
    /// install with none — so the way out of a workspace you no longer want is
    /// to make (or restore) another one beside it first.
    #[error("this is the only workspace on this device")]
    LastWorkspace,

    /// A Drive restore pointed at a vault a workspace on this device already
    /// holds — the open one, or a locked one the registry remembers the vault
    /// id of. Restoring it beside itself would leave one device with two
    /// workspaces syncing the same pack, each overwriting the other's idea of it.
    #[error("this vault is already a workspace on this device")]
    VaultAlreadyOpen,

    /// A sync connect on a vault the account does not hold, while it holds
    /// others. The account is the source of truth for which vaults exist, so
    /// this vault is not added beside them: the user restores one of the
    /// account's vaults instead — see the app's `commands::sync::sync_adopt_pending`.
    #[error("this Google account holds other vaults")]
    VaultNotInAccount,

    /// The pack this vault syncs is gone from the account and a marker sits
    /// where it was — another device ran a "delete everywhere" on it. The run
    /// stops here rather than reading the empty folder as a first sync and
    /// uploading the vault again, which would undo the delete on every device.
    #[error("this vault was deleted from Google Drive on another device")]
    VaultDeletedRemotely,

    /// The account's pack is sealed under a key descriptor of the same or a
    /// later generation than this device's: another device changed the master
    /// password. The run stops until the user enters the new one
    /// (`sync_adopt_password` in the app).
    #[error("the master password of this lockbox was changed on another device")]
    PasswordChangedRemotely,

    /// The account's pack is sealed under an earlier generation this device
    /// has re-keyed away from and no longer holds the key of. It can neither
    /// be opened nor pushed over without losing what is in it; the run stops
    /// until that device catches up or the user replaces the account's copy
    /// (`sync_replace_remote` in the app).
    #[error("the account holds this lockbox under a password this device has already replaced")]
    VaultUnderRetiredPassword,

    /// A lockbox other than the primary was opened (or restored) with a
    /// password that opens it but is not the vault password — the one the
    /// primary opens with. Every lockbox opens with the vault password, so this
    /// one is not sealed under the app key; it stays closed until the user
    /// enters the vault password to move it over (`workspace_adopt_vault_password`
    /// in the app). `workspace_id` names the lockbox, which a restore has only
    /// just created, so the frontend knows what to move.
    #[error("this lockbox has a different password; enter the vault password to move it over")]
    VaultPasswordRequired { workspace_id: String },

    #[error("the file is too large")]
    FileTooLarge,

    #[error("the file is not text")]
    FileNotText,

    /// An attachment that would take the vault's files past their budget
    /// (see the app's `commands::attachments`). The file itself may be small.
    #[error("this vault's attachments are at their limit")]
    VaultFull,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error("crypto error: {0}")]
    Crypto(String),

    /// A sealed blob that is whole and well-formed but was sealed under a
    /// key other than the one offered (`crypto::open_workspace_key`): the
    /// one failure of an open that is conclusive about the key rather than
    /// about the file. What lets a caller remove a sidecar as stale without
    /// ever removing one it merely could not read.
    #[error("sealed under another key")]
    SealedUnderAnotherKey,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// The store keeps its own error type (see `store`); this is where it becomes
// the app's.
pub fn store_err(e: StoreError) -> Error {
    Error::Other(e.to_string())
}

impl Error {
    // The discriminant the frontend switches on. Public because sync reports
    // a failed run as status rather than as a rejection, and carries the kind
    // alongside the message so a screen can recognise one failure without
    // matching on English prose (see `state::SyncFailure` in the app).
    pub fn kind(&self) -> &'static str {
        match self {
            Error::InvalidPassword => "invalidPassword",
            Error::VaultTooNew => "vaultTooNew",
            Error::TooManyAttempts { .. } => "tooManyAttempts",
            Error::Locked => "locked",
            Error::StaleSession => "staleSession",
            Error::NotFound => "notFound",
            Error::Cancelled => "cancelled",
            Error::SyncNotConfigured => "syncNotConfigured",
            Error::Unsupported(_) => "unsupported",
            Error::Unrecognized => "unrecognized",
            Error::ShareLinkInvalid => "shareLinkInvalid",
            Error::ShareExpired => "shareExpired",
            Error::ShareTooNew => "shareTooNew",
            Error::ShareTooLarge => "shareTooLarge",
            Error::EntryTooLargeToShare => "entryTooLargeToShare",
            Error::ShareNotOwned => "shareNotOwned",
            Error::ShareNeedsSync => "shareNeedsSync",
            Error::SetupBusy => "setupBusy",
            Error::SyncBusy => "syncBusy",
            Error::AccountNotConnected => "accountNotConnected",
            Error::NoRemoteVault => "noRemoteVault",
            Error::AlreadySetUp => "alreadySetUp",
            Error::PrimaryWorkspaceOnly => "primaryWorkspaceOnly",
            Error::WorkspaceNameRequired => "workspaceNameRequired",
            Error::WorkspacePasswordRequired => "workspacePasswordRequired",
            Error::LastWorkspace => "lastWorkspace",
            Error::VaultAlreadyOpen => "vaultAlreadyOpen",
            Error::VaultNotInAccount => "vaultNotInAccount",
            Error::VaultDeletedRemotely => "vaultDeletedRemotely",
            Error::PasswordChangedRemotely => "passwordChangedRemotely",
            Error::VaultUnderRetiredPassword => "vaultUnderRetiredPassword",
            Error::VaultPasswordRequired { .. } => "vaultPasswordRequired",
            Error::FileTooLarge => "fileTooLarge",
            Error::FileNotText => "fileNotText",
            Error::VaultFull => "vaultFull",
            Error::Io(_) => "io",
            Error::Serde(_) => "serde",
            Error::Crypto(_) => "crypto",
            Error::SealedUnderAnotherKey => "sealedUnderAnotherKey",
            Error::Other(_) => "other",
        }
    }
}

// Every rejection reaches the frontend as `{ kind, message }` — the kind is what
// it branches on, the message is what it may show. `camelCase` matches every
// other DTO sent to the frontend (see `models.rs`).
impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ErrorDto<'a> {
            kind: &'static str,
            message: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            retry_after_secs: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            workspace_id: Option<&'a str>,
        }

        ErrorDto {
            kind: self.kind(),
            message: self.to_string(),
            retry_after_secs: match self {
                Error::TooManyAttempts { retry_after_secs } => Some(*retry_after_secs),
                _ => None,
            },
            workspace_id: match self {
                Error::VaultPasswordRequired { workspace_id } => Some(workspace_id),
                _ => None,
            },
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_variant_serializes_as_kind_and_message() {
        let json = serde_json::to_value(Error::Locked).unwrap();
        assert_eq!(json["kind"], "locked");
        assert_eq!(json["message"], "vault is locked");
        assert!(json.get("retryAfterSecs").is_none());
    }

    #[test]
    fn the_password_epoch_errors_carry_their_kinds() {
        let json = serde_json::to_value(Error::PasswordChangedRemotely).unwrap();
        assert_eq!(json["kind"], "passwordChangedRemotely");
        assert_eq!(
            json["message"],
            "the master password of this lockbox was changed on another device"
        );
        let json = serde_json::to_value(Error::VaultUnderRetiredPassword).unwrap();
        assert_eq!(json["kind"], "vaultUnderRetiredPassword");
        assert_eq!(
            json["message"],
            "the account holds this lockbox under a password this device has already replaced"
        );
    }

    #[test]
    fn a_lockbox_off_the_vault_password_names_itself() {
        let json = serde_json::to_value(Error::VaultPasswordRequired {
            workspace_id: "a1b2".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "vaultPasswordRequired");
        assert_eq!(
            json["message"],
            "this lockbox has a different password; enter the vault password to move it over"
        );
        assert_eq!(json["workspaceId"], "a1b2");
        assert!(serde_json::to_value(Error::Locked)
            .unwrap()
            .get("workspaceId")
            .is_none());
    }

    #[test]
    fn too_many_attempts_carries_the_countdown() {
        let json = serde_json::to_value(Error::TooManyAttempts {
            retry_after_secs: 8,
        })
        .unwrap();
        assert_eq!(json["kind"], "tooManyAttempts");
        assert_eq!(json["message"], "too many failed attempts, try again in 8s");
        assert_eq!(json["retryAfterSecs"], 8);
    }
}
