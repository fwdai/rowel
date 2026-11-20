//! What the session manager needs from the app, and nothing more: which
//! lockboxes this device holds and their keys, a run on one of them under the
//! process's run claim, the end of that run, forgetting a device, and the
//! clock. The app's is `nearby::app::host`, over `AppState`; the tests
//! run two of their own in one process.

use std::ops::Deref;

use zeroize::Zeroizing;

use crate::crypto::VaultKey;
use crate::error::Result;
use crate::nearby::exchange::PeerOutcome;
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
    /// One run on one lockbox: the lockbox open for it, which it derefs to,
    /// and the run claim, both held until it is dropped or handed to
    /// [`Host::finish`].
    type Run: Deref<Target = dyn LocalVault>;

    /// Every lockbox this device can sync now.
    fn lockboxes(&self) -> Vec<Lockbox>;

    /// Take the run claim and open the lockbox `vault_id` under it.
    /// `Error::SyncBusy` while another run holds the claim, and
    /// `Error::Locked` while there is no session to open it in: both are
    /// asked again later.
    fn begin(&self, vault_id: &str) -> Result<Self::Run>;

    /// A run ended, having done `outcome` here: the claim goes, and what the
    /// merge changed is announced as a cloud run's would be.
    fn finish(&self, run: Self::Run, outcome: PeerOutcome);

    /// Stop holding tombstones back for the device `device_id` in every
    /// lockbox (`engine::forget_peer`): it was removed. Blocking; called on a
    /// thread of its own.
    fn forget(&self, device_id: &str);

    /// The clock every merge stamp and every converge is read from, in ms
    /// since the Unix epoch.
    fn now_ms(&self) -> i64;
}
