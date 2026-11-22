//! What the session manager needs from the app; the app's is `nearby::app::host`.

use zeroize::Zeroizing;

use crate::crypto::VaultKey;
use crate::error::Result;
use crate::nearby::exchange::Absorbed;
use crate::nearby::trust::lockbox_id;
use crate::sync::engine::LocalVault;

#[derive(Clone, PartialEq)]
pub struct Keys {
    pub id: String,
    pub proof: Zeroizing<[u8; 32]>,
    pub discovery: Zeroizing<[u8; 32]>,
}

impl Keys {
    pub fn of(key: &VaultKey) -> Self {
        Self::new(key.nearby_proof_key(), key.nearby_discovery_key())
    }

    pub(super) fn new(proof: Zeroizing<[u8; 32]>, discovery: Zeroizing<[u8; 32]>) -> Self {
        Self {
            id: lockbox_id(&discovery),
            proof,
            discovery,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Lockbox {
    pub current: Keys,
    // Newest first.
    pub retired: Vec<Keys>,
}

impl Lockbox {
    pub fn id(&self) -> &str {
        &self.current.id
    }
}

pub trait Host: Send + Sync + 'static {
    fn lockboxes(&self) -> Vec<Lockbox>;

    /// `step` on lockbox `id` (its current key's), opened under the run claim;
    /// `Error::SyncBusy`, without running it, while another holds the claim.
    fn step<T>(&self, id: &str, step: impl FnOnce(&dyn LocalVault) -> Result<T>) -> Result<T>;

    // `merged` inside the step of a merge that changed the lockbox, the claim held;
    // `pass_on` once that lockbox's exchange is over, the claim released.
    fn merged(&self, id: &str, local: &dyn LocalVault, absorbed: &Absorbed);

    fn pass_on(&self, id: &str);

    fn now_ms(&self) -> i64;
}
