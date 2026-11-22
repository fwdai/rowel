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
use crate::nearby::session::{Host, Keys, Lockbox};
use crate::org::lockboxes::is_org_lockbox_dir;
use crate::session::store_err;
use crate::state::AppState;
use crate::store::identity;
use crate::sync::engine::{LocalVault, SessionVault, WorkspaceVault};
use crate::sync::Provider;
use crate::workspace::{dir_of, Registry, Workspace};
use crate::{retired, storage};

type Material = Zeroizing<Vec<u8>>;

pub(super) struct AppHost {
    app: AppHandle,
}

impl AppHost {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }

    fn open_lockbox(&self) -> Option<(Material, Option<String>)> {
        let state = self.app.state::<AppState>();
        let session = state.session.lock().unwrap();
        let (key, store) = (session.key().ok()?, session.store().ok()?);
        let vault_id = identity::vault_id(store).ok().flatten();
        Some((Zeroizing::new(key.biometric_material().to_vec()), vault_id))
    }

    fn keyed_workspaces(&self, root: &Path) -> Vec<(Option<Material>, Workspace)> {
        let workspaces = Registry::load(root).workspaces;
        let state = self.app.state::<AppState>();
        let ring = state.keyring.lock().unwrap();
        workspaces
            .into_iter()
            .map(|w| (ring.get(&w.id), w))
            .collect()
    }

    // Under `workspace_lock`: the workspace holding `vault_id`, and whether it is the open one.
    fn locate(&self, state: &AppState, root: &Path, vault_id: &str) -> Result<(String, bool)> {
        let active = state.active_workspace.lock().unwrap().clone();
        let open_id =
            identity::vault_id(state.session.lock().unwrap().store()?).map_err(store_err)?;
        if open_id.as_deref() == Some(vault_id) {
            return Ok((active, true));
        }
        let workspace = Registry::load(root)
            .workspaces
            .into_iter()
            .find(|w| w.id != active && w.vault_id.as_deref() == Some(vault_id))
            .ok_or(Error::NotFound)?;
        Ok((workspace.id, false))
    }

    fn located(&self, vault_id: &str) -> Option<(String, bool)> {
        let state = self.app.state::<AppState>();
        let root = storage::root_dir(&self.app).ok()?;
        let _paths = state.workspace_lock.lock().unwrap();
        self.locate(&state, &root, vault_id).ok()
    }

    // Under `workspace_lock`, with the claim held.
    fn target(&self, state: &AppState, root: &Path, vault_id: &str) -> Result<Target> {
        let (workspace_id, is_open) = self.locate(state, root, vault_id)?;
        let dir = dir_of(root, &workspace_id);
        if is_org_lockbox_dir(&dir) {
            return Err(Error::NotFound);
        }
        if is_open {
            return Ok(Target::Open);
        }
        let material = state
            .keyring
            .lock()
            .unwrap()
            .get(&workspace_id)
            .ok_or(Error::Locked)?;
        Ok(Target::Closed {
            dir,
            workspace_id,
            material,
        })
    }

    fn open(&self, target: Target) -> Result<Vault> {
        let writer = Some(storage::install_id(&self.app));
        // Named only by a foreign-pack error, which a proved device's pack never is.
        let provider = Provider::default();
        Ok(match target {
            Target::Open => Vault::Open(SessionVault::capture(&self.app, provider, writer)?),
            Target::Closed {
                dir,
                workspace_id,
                material,
            } => {
                let key = VaultKey::from_material(material, true);
                let vault = WorkspaceVault::open(&dir, &workspace_id, key, provider, writer)?;
                Vault::Closed(vault)
            }
        })
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

// No descriptor, no pack; and an org lockbox is its server's to sync.
fn syncable_dir(root: &Path, workspace_id: &str) -> Option<PathBuf> {
    let dir = dir_of(root, workspace_id);
    (!is_org_lockbox_dir(&dir) && dir.join(storage::KDF_SIDECAR_FILE).exists()).then_some(dir)
}

fn lockbox(dir: &Path, workspace_id: &str, vault_id: String, material: Material) -> Lockbox {
    let key = VaultKey::from_material(material, true);
    Lockbox {
        retired: retired::load(dir, &key, workspace_id)
            .iter()
            .map(|epoch| Keys::of(&epoch.key()))
            .collect(),
        current: Keys::of(&key),
        vault_id,
    }
}

enum Target {
    Open,
    Closed {
        dir: PathBuf,
        workspace_id: String,
        material: Material,
    },
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
        let active = crate::workspace::active_id(&self.app);
        let mut open = self.open_lockbox();
        self.keyed_workspaces(&root)
            .into_iter()
            .filter_map(|(material, workspace)| {
                let dir = syncable_dir(&root, &workspace.id)?;
                let (vault_id, material) = if workspace.id == active {
                    let (material, vault_id) = open.take()?;
                    (vault_id.or(workspace.vault_id)?, material)
                } else {
                    (workspace.vault_id?, material?)
                };
                Some(lockbox(&dir, &workspace.id, vault_id, material))
            })
            .collect()
    }

    fn step<T>(
        &self,
        vault_id: &str,
        step: impl FnOnce(&dyn LocalVault) -> Result<T>,
    ) -> Result<T> {
        let state = self.app.state::<AppState>();
        let root = storage::root_dir(&self.app)?;
        let (_claim, target) = {
            let _paths = state.workspace_lock.lock().unwrap();
            (self.claim()?, self.target(&state, &root, vault_id))
        };
        // Dropped before the claim, so the lockbox closes before another run opens it.
        let vault = self.open(target?)?;
        if vault.vault_id()?.as_deref() != Some(vault_id) {
            return Err(Error::NotFound);
        }
        step(&*vault)
    }

    fn merged(&self, vault_id: &str, local: &dyn LocalVault, absorbed: &Absorbed) {
        let Some((workspace_id, is_open)) = self.located(vault_id) else {
            return;
        };
        if absorbed.renamed {
            crate::sync::mirror_name(&self.app, &workspace_id, local);
        }
        if is_open && absorbed.merged > 0 {
            crate::commands::sync::announce_merged(&self.app);
        }
    }

    fn pass_on(&self, vault_id: &str) {
        if self.located(vault_id).is_some_and(|(_, is_open)| is_open) {
            let state = self.app.state::<AppState>();
            crate::commands::sync::request_run_if_ready(&self.app, &state);
        }
        super::request_run(&self.app);
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
