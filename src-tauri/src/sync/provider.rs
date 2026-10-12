//! Which cloud account a workspace syncs through.
//!
//! The one name every provider-shaped decision hangs off: the token file a
//! workspace keeps, the OAuth client it signs in with, the adapter its runs
//! address and the words its errors use. Adding a provider is adding a variant
//! here; the compiler then points at every `match` that has to learn it.

use serde::{Deserialize, Serialize};

use super::account::OAuthConfig;
use super::providers::{dropbox, gdrive};
use crate::storage;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// The default because it was the only one: a token file sealed before
    /// providers existed carries no name, and it is Google's.
    #[default]
    GoogleDrive,
    Dropbox,
}

impl Provider {
    /// Every provider, in the order a workspace's token files are looked for.
    pub const ALL: [Provider; 2] = [Provider::GoogleDrive, Provider::Dropbox];

    /// The product, as the user knows it: "Google Drive was disconnected…".
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google Drive",
            Provider::Dropbox => "Dropbox",
        }
    }

    /// The account, as a sentence names it: "this Google account…". Not
    /// [`Provider::display_name`]: a Google sign-in is to a Google account, of
    /// which Drive is one service, and the existing copy says so.
    pub fn account_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => "Google",
            Provider::Dropbox => "Dropbox",
        }
    }

    /// The sealed token file, relative to a workspace directory. At most one of
    /// these exists per workspace, so the file's name alone says which provider
    /// a *locked* workspace syncs through.
    pub fn token_file_name(self) -> &'static str {
        match self {
            Provider::GoogleDrive => storage::GDRIVE_FILE,
            Provider::Dropbox => storage::DROPBOX_FILE,
        }
    }
}

/// The OAuth client a provider's sign-in and token refresh talk to.
pub fn oauth_config(provider: Provider) -> &'static OAuthConfig {
    match provider {
        Provider::GoogleDrive => &gdrive::OAUTH,
        Provider::Dropbox => &dropbox::OAUTH,
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
    }

    // Google's file keeps the name it had before there was a choice, so an
    // existing install is still connected after the update.
    #[test]
    fn googles_token_file_keeps_its_name() {
        assert_eq!(Provider::GoogleDrive.token_file_name(), "auth/gdrive.swftx");
        assert_eq!(Provider::Dropbox.token_file_name(), "auth/dropbox.swftx");
    }

    #[test]
    fn every_provider_has_its_own_oauth_client() {
        for provider in Provider::ALL {
            assert_eq!(oauth_config(provider).provider, provider);
        }
    }
}
