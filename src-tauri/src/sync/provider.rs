//! Which cloud account a workspace syncs through.
//!
//! The one name every provider-shaped decision hangs off: the token file a
//! workspace keeps, the OAuth client it signs in with, the adapter its runs
//! address and the words its errors use. Adding a provider is adding a variant
//! here; the compiler then points at every `match` that has to learn it.

use serde::{Deserialize, Serialize};

use super::account::OAuthConfig;
use super::providers::{dropbox, gdrive, icloud};
use crate::storage;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// The default because it was the only one: a token file sealed before
    /// providers existed carries no name, and it is Google's.
    #[default]
    GoogleDrive,
    Dropbox,
    /// The device's own iCloud account, through CloudKit. No sign-in of its
    /// own: Apple devices only, and only a build signed with the iCloud
    /// entitlements ([`Provider::supported`]).
    ICloud,
    /// A company's Rowel Server, holding an org lockbox's pack and the key
    /// that opens it. No sign-in of the workspace's own either: the member
    /// signs in to the server once per install (`org::connections`), and the
    /// token file only names which server the lockbox came from.
    RowelServer,
}

impl Provider {
    /// Every provider, in the order a workspace's token files are looked for
    /// and the order the pickers offer them.
    pub const ALL: [Provider; 4] = [
        Provider::GoogleDrive,
        Provider::Dropbox,
        Provider::ICloud,
        Provider::RowelServer,
    ];

    /// The providers this build can connect from the account pickers: every
    /// one but those the platform rules out ([`Provider::supported`]), and
    /// never the Rowel Server — an org lockbox is connected from Settings ›
    /// Lockboxes › Rowel Server, where the member signs in and picks the
    /// lockboxes the server offers; it is not an account a vault of the
    /// user's own is pushed into. What `app_status` tells the frontend to
    /// offer.
    pub fn available() -> Vec<Provider> {
        Self::ALL
            .into_iter()
            .filter(|p| p.supported() && *p != Provider::RowelServer)
            .collect()
    }

    /// Whether this build can sync through the provider at all. Google,
    /// Dropbox and the Rowel Server are HTTP and work everywhere; iCloud needs
    /// Apple's CloudKit and a binary signed for it.
    pub fn supported(self) -> bool {
        match self {
            Provider::GoogleDrive | Provider::Dropbox | Provider::RowelServer => true,
            Provider::ICloud => icloud::supported(),
        }
    }

    /// Whether connecting means an OAuth consent in the browser. iCloud is the
    /// one that does not: its account is the device's, and a connect only
    /// checks that there is one.
    pub fn signs_in_with_oauth(self) -> bool {
        self.oauth().is_some()
    }

    /// The product, as the user knows it: "Google Drive was disconnected…".
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google Drive",
            Provider::Dropbox => "Dropbox",
            Provider::ICloud => "iCloud",
            Provider::RowelServer => "Rowel Server",
        }
    }

    /// The account, as a sentence names it: "this Google account…". Not
    /// [`Provider::display_name`]: a Google sign-in is to a Google account, of
    /// which Drive is one service, and the existing copy says so. A Rowel
    /// Server is both the product and the account.
    pub fn account_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google",
            Provider::Dropbox => "Dropbox",
            Provider::ICloud => "iCloud",
            Provider::RowelServer => "Rowel Server",
        }
    }

    /// Rowel's folder as the user sees it in the account, for copy that names
    /// a path inside it: `Rowel` on Drive, `Apps/Rowel` in Dropbox, where an
    /// App folder app lives under `Apps/`. iCloud has no folder the user can
    /// open — the packs are CloudKit records — so its adapter never looks a
    /// packs folder up and no copy names one; the root's name stands in. A
    /// Rowel Server has no folders at all — a pack is a resource under its
    /// lockbox — and the same stand-in applies.
    pub fn packs_root_label(self) -> &'static str {
        match self {
            Provider::GoogleDrive | Provider::ICloud | Provider::RowelServer => {
                super::layout::ROOT_FOLDER
            }
            Provider::Dropbox => dropbox::APP_FOLDER,
        }
    }

    /// The sealed token file, relative to a workspace directory. At most one of
    /// these exists per workspace, so the file's name alone says which provider
    /// a *locked* workspace syncs through.
    ///
    /// iCloud's holds no token — the device's account is the credential — but
    /// the file is still what says "this workspace syncs through iCloud", and
    /// every rule about connecting, disconnecting and re-sealing reads it. The
    /// Rowel Server's holds the server's URL and no token, for the same reason.
    pub fn token_file_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => storage::GDRIVE_FILE,
            Provider::Dropbox => storage::DROPBOX_FILE,
            Provider::ICloud => storage::ICLOUD_FILE,
            Provider::RowelServer => storage::ROWEL_SERVER_FILE,
        }
    }

    /// The OAuth client the provider's sign-in and token refresh talk to, or
    /// `None` for a provider whose workspaces have no sign-in of their own:
    /// iCloud, and the Rowel Server, whose sign-in is the install's
    /// (`org::client`), not a workspace's.
    pub fn oauth(self) -> Option<&'static OAuthConfig> {
        match self {
            Provider::GoogleDrive => Some(&gdrive::OAUTH),
            Provider::Dropbox => Some(&dropbox::OAUTH),
            Provider::ICloud | Provider::RowelServer => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The serialized names are the frontend's contract, and the token file's.
    #[test]
    fn providers_serialize_in_camel_case() {
        assert_eq!(
            serde_json::to_value(Provider::GoogleDrive).unwrap(),
            "googleDrive"
        );
        assert_eq!(serde_json::to_value(Provider::Dropbox).unwrap(), "dropbox");
        assert_eq!(serde_json::to_value(Provider::ICloud).unwrap(), "iCloud");
        assert_eq!(
            serde_json::to_value(Provider::RowelServer).unwrap(),
            "rowelServer"
        );
    }

    // Google's file keeps the name it had before there was a choice, so an
    // existing install is still connected after the update.
    #[test]
    fn googles_token_file_keeps_its_name() {
        assert_eq!(Provider::GoogleDrive.token_file_name(), "auth/gdrive.swftx");
        assert_eq!(Provider::Dropbox.token_file_name(), "auth/dropbox.swftx");
        assert_eq!(Provider::ICloud.token_file_name(), "auth/icloud.swftx");
        assert_eq!(
            Provider::RowelServer.token_file_name(),
            "auth/rowel-server.swftx"
        );
    }

    #[test]
    fn every_oauth_provider_has_its_own_client() {
        for provider in Provider::ALL {
            match provider.oauth() {
                Some(config) => assert_eq!(config.provider, provider),
                None => assert!(matches!(provider, Provider::ICloud | Provider::RowelServer)),
            }
        }
    }

    // iCloud and the Rowel Server are the providers without a browser sign-in
    // of the workspace's own; the HTTP ones work on every platform.
    #[test]
    fn only_icloud_and_the_server_skip_oauth() {
        assert!(Provider::GoogleDrive.signs_in_with_oauth());
        assert!(Provider::Dropbox.signs_in_with_oauth());
        assert!(!Provider::ICloud.signs_in_with_oauth());
        assert!(!Provider::RowelServer.signs_in_with_oauth());
        assert!(Provider::GoogleDrive.supported());
        assert!(Provider::Dropbox.supported());
        assert!(Provider::RowelServer.supported());
        let available = Provider::available();
        assert!(available.starts_with(&[Provider::GoogleDrive, Provider::Dropbox]));
        assert_eq!(
            available.contains(&Provider::ICloud),
            Provider::ICloud.supported()
        );
    }

    // The server is a provider for every rule that reads a token file, and
    // never one the account pickers offer: an org lockbox is connected from
    // its own Settings section, not pushed into like a drive.
    #[test]
    fn the_server_is_looked_up_but_never_offered() {
        assert!(Provider::ALL.contains(&Provider::RowelServer));
        assert!(!Provider::available().contains(&Provider::RowelServer));
    }
}
