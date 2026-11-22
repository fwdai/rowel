//! What the session manager needs from the app; the app's is `nearby::app::host`.

use zeroize::Zeroizing;

use crate::crypto::VaultKey;
use crate::error::Result;
use crate::nearby::exchange::Absorbed;
use crate::sync::engine::LocalVault;

/// The two nearby subkeys of one key of a lockbox.
#[derive(Clone, PartialEq)]
pub struct Keys {
    /// Proves the key on a connection (`trust::own_proof`).
    pub proof: Zeroizing<[u8; 32]>,
    /// Tags the lockbox in an advertisement (`trust::discovery_tag`).
    pub discovery: Zeroizing<[u8; 32]>,
}

impl Keys {
    pub fn of(key: &VaultKey) -> Self {
        Self {
            proof: key.nearby_proof_key(),
            discovery: key.nearby_discovery_key(),
        }
    }
}

/// A lockbox this device can sync with another now.
#[derive(Clone, PartialEq)]
pub struct Lockbox {
    /// What the two devices address it by, once each has proved its key.
    pub vault_id: String,
    pub current: Keys,
    /// The keys of the epochs it was re-keyed away from, newest first: a
    /// device still on one of them is told the password changed.
    pub retired: Vec<Keys>,
}

pub trait Host: Send + Sync + 'static {
    fn lockboxes(&self) -> Vec<Lockbox>;

    /// `step` on lockbox `vault_id`, opened under the run claim;
    /// `Error::SyncBusy`, without running it, while another holds the claim.
    fn step<T>(&self, vault_id: &str, step: impl FnOnce(&dyn LocalVault) -> Result<T>)
        -> Result<T>;

    /// Called with the claim released.
    fn announce(&self, vault_id: &str, absorbed: &Absorbed);

    fn now_ms(&self) -> i64;
}
