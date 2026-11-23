//! What the session manager needs from the app; the app's is `nearby::app::host`.

use std::collections::HashMap;
use std::sync::Mutex;

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

// The packs of lockboxes whose password changed on another device, for the adopt to
// open: by the id of the key a lockbox is on here, then by that of the key sealing the pack.
#[derive(Default)]
pub struct Kept(Mutex<HashMap<String, HashMap<String, Vec<u8>>>>);

impl Kept {
    pub fn sealed_under(&self, id: &str) -> Vec<String> {
        let kept = self.0.lock().unwrap();
        let packs = kept.get(id).into_iter().flatten();
        packs
            .map(|(sealed_under, _)| sealed_under.clone())
            .collect()
    }

    pub fn packs(&self, id: &str) -> Vec<Vec<u8>> {
        let kept = self.0.lock().unwrap();
        let packs = kept.get(id).into_iter().flatten();
        packs.map(|(_, pack)| pack.clone()).collect()
    }

    pub fn keep(&self, id: &str, sealed_under: &str, pack: Vec<u8>) {
        let mut kept = self.0.lock().unwrap();
        let packs = kept.entry(id.to_string()).or_default();
        packs.insert(sealed_under.to_string(), pack);
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
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

    fn kept(&self) -> &Kept;

    fn now_ms(&self) -> i64;
}
