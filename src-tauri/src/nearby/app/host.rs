//! The app as the nearby manager's [`Host`].

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::commands::sync::{claim_or_busy, RunClaim};
use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::nearby::exchange::Absorbed;
use crate::nearby::session::{Host, Kept, Keys, Lockbox};
use crate::org::lockboxes::is_org_lockbox_dir;
use crate::state::AppState;
use crate::sync::engine::{LocalVault, SessionVault, WorkspaceVault};
use crate::sync::Provider;
use crate::workspace::{active_id, dir_of, Registry};
use crate::{retired, storage};

type Material = Zeroizing<Vec<u8>>;

pub(super) struct AppHost {
    app: AppHandle,
}

impl AppHost {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }

    // The open one's key is the session's, the others' the keyring's.
    fn held(&self, root: &Path) -> Vec<Held> {
        let workspaces = Registry::load(root).workspaces;
        let active = active_id(&self.app);
        let state = self.app.state::<AppState>();
        let open = state.session.lock().unwrap().key().ok().map(material);
        let ring = state.keyring.lock().unwrap();
        workspaces
            .into_iter()
            .filter_map(|w| {
                let dir = syncable_dir(root, &w.id)?;
                let is_open = w.id == active;
                let material = if is_open {
                    open.clone()?
                } else {
                    ring.get(&w.id)?
                };
                Some(Held {
                    workspace_id: w.id,
                    dir,
                    material,
                    is_open,
                })
            })
            .collect()
    }

    fn located(&self, id: &str) -> Option<Held> {
        let root = storage::root_dir(&self.app).ok()?;
        let state = self.app.state::<AppState>();
        let _paths = state.workspace_lock.lock().unwrap();
        find(self.held(&root), id)
    }

    fn open(&self, held: Held) -> Result<Vault> {
        let writer = Some(storage::install_id(&self.app));
        // Named only by a foreign-pack error, which a proved device's pack never is.
        let provider = Provider::default();
        if held.is_open {
            let vault = SessionVault::capture(&self.app, provider, writer)?;
            return Ok(Vault::Open(vault));
        }
        let key = held.key();
        let vault = WorkspaceVault::open(&held.dir, &held.workspace_id, key, provider, writer)?;
        Ok(Vault::Closed(vault))
    }

    // The caller holds `workspace_lock`.
    fn claim(&self) -> Result<Claim> {
        let state = self.app.state::<AppState>();
        Ok(Claim {
            _held: claim_or_busy(&state)?,
            _then: Rerun(self.app.clone()),
        })
    }
}

fn material(key: &VaultKey) -> Material {
    Zeroizing::new(key.biometric_material().to_vec())
}

// No descriptor, no pack; and an org lockbox is its server's to sync.
fn syncable_dir(root: &Path, workspace_id: &str) -> Option<PathBuf> {
    let dir = dir_of(root, workspace_id);
    (!is_org_lockbox_dir(&dir) && dir.join(storage::KDF_SIDECAR_FILE).exists()).then_some(dir)
}

struct Held {
    workspace_id: String,
    dir: PathBuf,
    material: Material,
    is_open: bool,
}

impl Held {
    fn key(&self) -> VaultKey {
        VaultKey::from_material(self.material.clone(), true)
    }

    fn lockbox(&self) -> Lockbox {
        let key = self.key();
        Lockbox {
            retired: retired::load(&self.dir, &key, &self.workspace_id)
                .iter()
                .map(|epoch| Keys::of(&epoch.key()))
                .collect(),
            current: Keys::of(&key),
        }
    }
}

fn find(held: Vec<Held>, id: &str) -> Option<Held> {
    held.into_iter().find(|h| Keys::of(&h.key()).id == id)
}

// Its release answers a cloud run asked for meanwhile (`AppState::sync_rerun`),
// which takes `workspace_lock`: never dropped under it.
struct Claim {
    _held: RunClaim,
    _then: Rerun,
}

struct Rerun(AppHandle);

impl Drop for Rerun {
    fn drop(&mut self) {
        let state = self.0.state::<AppState>();
        if state.sync_rerun.load(Ordering::SeqCst) {
            crate::commands::sync::request_run_if_ready(&self.0, &state);
        }
    }
}

impl Host for AppHost {
    fn lockboxes(&self) -> Vec<Lockbox> {
        let Ok(root) = storage::root_dir(&self.app) else {
            return Vec::new();
        };
        self.held(&root).iter().map(Held::lockbox).collect()
    }

    fn step<T>(&self, id: &str, step: impl FnOnce(&dyn LocalVault) -> Result<T>) -> Result<T> {
        let state = self.app.state::<AppState>();
        let root = storage::root_dir(&self.app)?;
        let (_claim, held) = {
            let _paths = state.workspace_lock.lock().unwrap();
            (self.claim()?, find(self.held(&root), id))
        };
        // Dropped before the claim, so the lockbox closes before another run opens it.
        let vault = self.open(held.ok_or(Error::NotFound)?)?;
        step(&*vault)
    }

    fn merged(&self, id: &str, local: &dyn LocalVault, absorbed: &Absorbed) {
        let Some(held) = self.located(id) else {
            return;
        };
        if absorbed.renamed {
            crate::sync::mirror_name(&self.app, &held.workspace_id, local);
        }
        if held.is_open && absorbed.merged > 0 {
            crate::commands::sync::announce_merged(&self.app);
        }
    }

    fn pass_on(&self, id: &str) {
        if self.located(id).is_some_and(|held| held.is_open) {
            let state = self.app.state::<AppState>();
            crate::commands::sync::request_run_if_ready(&self.app, &state);
        }
        super::request_run(&self.app);
    }

    fn kept(&self) -> &Kept {
        &super::state(&self.app).kept
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

enum Vault {
    Open(SessionVault),
    Closed(WorkspaceVault),
}

impl Deref for Vault {
    type Target = dyn LocalVault;

    fn deref(&self) -> &Self::Target {
        match self {
            Vault::Open(vault) => vault,
            Vault::Closed(vault) => vault,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(workspace_id: &str, byte: u8) -> Held {
        Held {
            workspace_id: workspace_id.into(),
            dir: PathBuf::new(),
            material: Zeroizing::new(vec![byte; 32]),
            is_open: false,
        }
    }

    #[test]
    fn a_held_lockbox_is_found_by_its_current_keys_id() {
        let id = Keys::of(&held("b", 2).key()).id;
        let found = find(vec![held("a", 1), held("b", 2)], &id);
        assert_eq!(found.map(|h| h.workspace_id).as_deref(), Some("b"));
        assert!(find(vec![held("a", 1)], &id).is_none());
    }
}
