//! The app as the nearby session manager's [`Host`]: its lockboxes, its run
//! claim, its announcements.
//!
//! Which lockboxes: the open one, through the session, and every other whose
//! key the app keyring holds; org lockboxes are left out, since their server
//! is their one source (design §1). A run opens the open lockbox as a cloud
//! run does ([`SessionVault`], every step under the session lock, so a lock
//! ends it at its next step), and any other on a connection of its own
//! ([`WorkspaceVault`]), never the open one's database a second time.
//!
//! The run claim is the process's one (`AppState::syncing`, the flag a cloud
//! run's claim raises), taken under `workspace_lock` as a cloud run takes it:
//! a switch refuses to move the paths while it is held, so the lockbox a run
//! opened stays the open one, or stays closed, until the run lets go; and a
//! password change waits for it before re-keying anything under it.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::commands::sync::{claim_or_busy, RunClaim};
use crate::crypto::VaultKey;
use crate::error::{Error, Result};
use crate::nearby::exchange::PeerOutcome;
use crate::nearby::session::{Host, Keys, Lockbox};
use crate::org::lockboxes::is_org_lockbox_dir;
use crate::session::store_err;
use crate::state::AppState;
use crate::store::{identity, SqliteStore};
use crate::sync::engine::{self, LocalVault, SessionVault, WorkspaceVault};
use crate::sync::Provider;
use crate::workspace::{dir_of, Registry, Workspace};
use crate::{events, retired, storage};

/// How long forgetting a removed device waits for the run claim.
const FORGET_WAIT: Duration = Duration::from_secs(120);

pub(super) struct AppHost {
    app: AppHandle,
}

impl AppHost {
    pub fn new(app: &AppHandle) -> Self {
        Self { app: app.clone() }
    }

    /// Every lockbox this device can sync now, with what opens it: the open
    /// one's key from the session, the others' from the keyring.
    fn held(&self) -> Vec<Held> {
        let Ok(root) = storage::root_dir(&self.app) else {
            return Vec::new();
        };
        let state = self.app.state::<AppState>();
        let active = crate::workspace::active_id(&self.app);
        let mut open = {
            let session = state.session.lock().unwrap();
            match (session.key(), session.store()) {
                (Ok(key), Ok(store)) => Some((
                    Zeroizing::new(key.biometric_material().to_vec()),
                    identity::vault_id(store).ok().flatten(),
                )),
                _ => None,
            }
        };
        let registry = Registry::load(&root);
        let ring: Vec<_> = {
            let ring = state.keyring.lock().unwrap();
            let with_key = |workspace: Workspace| (ring.get(&workspace.id), workspace);
            registry.workspaces.into_iter().map(with_key).collect()
        };
        ring.into_iter()
            .filter_map(|(material, workspace)| {
                let dir = dir_of(&root, &workspace.id);
                // A lockbox with no descriptor cannot be packed, and an org
                // lockbox is its server's to sync.
                if is_org_lockbox_dir(&dir) || !dir.join(storage::KDF_SIDECAR_FILE).exists() {
                    return None;
                }
                let is_open = workspace.id == active;
                let (material, vault_id) = if is_open {
                    let (material, vault_id) = open.take()?;
                    (material, vault_id.or(workspace.vault_id)?)
                } else {
                    (material?, workspace.vault_id?)
                };
                Some(Held {
                    workspace_id: workspace.id,
                    dir,
                    vault_id,
                    material,
                    is_open,
                })
            })
            .collect()
    }

    /// Which lockbox `vault_id` is, read under `workspace_lock` with the claim
    /// held: the open one, or a closed one and its key.
    fn target(&self, state: &AppState, root: &Path, vault_id: &str) -> Result<Target> {
        let active = state.active_workspace.lock().unwrap().clone();
        let open_id = {
            let session = state.session.lock().unwrap();
            identity::vault_id(session.store()?).map_err(store_err)?
        };
        if open_id.as_deref() == Some(vault_id) {
            if is_org_lockbox_dir(&dir_of(root, &active)) {
                return Err(Error::NotFound);
            }
            return Ok(Target::Open {
                workspace_id: active,
            });
        }
        let workspace_id = Registry::load(root)
            .workspaces
            .into_iter()
            .find(|w| w.id != active && w.vault_id.as_deref() == Some(vault_id))
            .ok_or(Error::NotFound)?
            .id;
        let dir = dir_of(root, &workspace_id);
        if is_org_lockbox_dir(&dir) {
            return Err(Error::NotFound);
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

    /// The run claim, or `SyncBusy` while another holds it. The caller holds
    /// `workspace_lock`.
    fn claim(&self) -> Result<Claim> {
        let state = self.app.state::<AppState>();
        Ok(Claim {
            _held: claim_or_busy(&state)?,
            _then: Rerun(self.app.clone()),
        })
    }

    /// The run claim for a step that is not a run, waited for while another
    /// holds it.
    fn wait_for_claim(&self) -> Option<Claim> {
        let deadline = Instant::now() + FORGET_WAIT;
        loop {
            let claim = {
                let state = self.app.state::<AppState>();
                let _paths = state.workspace_lock.lock().unwrap();
                self.claim().ok()
            };
            if claim.is_some() || Instant::now() >= deadline {
                return claim;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

struct Held {
    workspace_id: String,
    dir: PathBuf,
    vault_id: String,
    material: Zeroizing<Vec<u8>>,
    is_open: bool,
}

enum Target {
    Open {
        workspace_id: String,
    },
    Closed {
        dir: PathBuf,
        workspace_id: String,
        material: Zeroizing<Vec<u8>>,
    },
}

/// The process's one run claim, held by a nearby step. A cloud run asked for
/// while it was held finds it taken and leaves its request raised
/// (`AppState::sync_rerun`); [`Rerun`], dropped after the claim, answers that
/// request, as a cloud run's own release does. Dropped outside
/// `workspace_lock`, which the cloud run takes.
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
    type Run = AppRun;

    fn lockboxes(&self) -> Vec<Lockbox> {
        self.held()
            .into_iter()
            .map(|held| {
                let key = VaultKey::from_material(held.material, true);
                Lockbox {
                    retired: retired::load(&held.dir, &key, &held.workspace_id)
                        .iter()
                        .map(|epoch| Keys::of(&epoch.key()))
                        .collect(),
                    current: Keys::of(&key),
                    vault_id: held.vault_id,
                }
            })
            .collect()
    }

    fn begin(&self, vault_id: &str) -> Result<AppRun> {
        let state = self.app.state::<AppState>();
        let root = storage::root_dir(&self.app)?;
        // The claim and which lockbox it covers, as one step under the paths'
        // lock. The block ends before anything below can drop the claim:
        // releasing it may start a cloud run, which takes this lock.
        let (claim, target) = {
            let _paths = state.workspace_lock.lock().unwrap();
            let claim = self.claim()?;
            (claim, self.target(&state, &root, vault_id))
        };
        let target = target?;
        let writer = Some(storage::install_id(&self.app));
        // The provider names an account in a foreign-pack error; a pack from
        // a device that proved this lockbox's key is never one.
        let provider = Provider::default();
        let (vault, workspace_id, is_open) = match target {
            Target::Open { workspace_id } => (
                Vault::Open(SessionVault::capture(&self.app, provider, writer)?),
                workspace_id,
                true,
            ),
            Target::Closed {
                dir,
                workspace_id,
                material,
            } => {
                let key = VaultKey::from_material(material, true);
                let vault = WorkspaceVault::open(&dir, &workspace_id, key, provider, writer)?;
                (Vault::Closed(vault), workspace_id, false)
            }
        };
        let run = AppRun {
            vault,
            workspace_id,
            is_open,
            _claim: claim,
        };
        // The proofs were for this vault id: what opened has to be it.
        if run.vault_id()?.as_deref() != Some(vault_id) {
            return Err(Error::NotFound);
        }
        Ok(run)
    }

    fn finish(&self, run: AppRun, outcome: PeerOutcome) {
        let name = outcome
            .renamed
            .then(|| run.name().ok().and_then(|(name, _)| name))
            .flatten();
        let (is_open, workspace_id) = (run.is_open, run.workspace_id.clone());
        // The lockbox closes and the claim goes before anything below asks
        // for a run of its own.
        drop(run);
        if let Some(name) = name {
            let recorded = if is_open {
                crate::workspace::record_vault_name(&self.app, &name)
            } else {
                crate::workspace::record_vault_name_of(&self.app, &workspace_id, &name)
            };
            match recorded {
                Ok(()) => events::workspace_renamed(&self.app),
                Err(e) => log::warn!("nearby: could not record lockbox {workspace_id}'s name: {e}"),
            }
        }
        if outcome.merged == 0 && !outcome.renamed {
            return;
        }
        if is_open {
            if outcome.merged > 0 {
                // What a cloud run that merged says: the refreshed list, and
                // iOS's logins.
                events::vault_merged(&self.app, crate::commands::sync::entry_metas(&self.app));
                crate::credential_identities::publish(&self.app);
            }
            // The account has not seen what came from the other device.
            let state = self.app.state::<AppState>();
            crate::commands::sync::request_run_if_ready(&self.app, &state);
        }
        // Nor have the other devices here; the one it came from has nothing
        // left to say and moves no pack.
        super::request_run(&self.app);
    }

    fn forget(&self, device_id: &str) {
        let Some(claim) = self.wait_for_claim() else {
            log::warn!("nearby: a removed device's rows are kept for now");
            return;
        };
        for held in self.held() {
            let forgotten = if held.is_open {
                let state = self.app.state::<AppState>();
                let session = state.session.lock().unwrap();
                session
                    .store()
                    .and_then(|store| engine::forget_peer(store, device_id))
            } else {
                let key = VaultKey::from_material(held.material, true);
                SqliteStore::open(&held.dir.join(storage::DB_FILE), &*key.sqlcipher_key())
                    .map_err(store_err)
                    .and_then(|store| engine::forget_peer(&store, device_id))
            };
            if let Err(e) = forgotten {
                log::warn!(
                    "nearby: lockbox {} keeps a removed device's row: {e}",
                    held.workspace_id
                );
            }
        }
        drop(claim);
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// A run on one lockbox: it open, and the claim. Dropped in that order: the
/// lockbox's connection closes before the claim lets another run in.
pub(crate) struct AppRun {
    vault: Vault,
    workspace_id: String,
    is_open: bool,
    _claim: Claim,
}

enum Vault {
    Open(SessionVault),
    Closed(WorkspaceVault),
}

impl Deref for AppRun {
    type Target = dyn LocalVault;

    fn deref(&self) -> &Self::Target {
        match &self.vault {
            Vault::Open(vault) => vault,
            Vault::Closed(vault) => vault,
        }
    }
}
