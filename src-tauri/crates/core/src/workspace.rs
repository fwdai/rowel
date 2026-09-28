//! The workspace registry: which vaults ("workspaces") this install has,
//! which one is active, and where each one's files are. Read by the app and by
//! the iOS AutoFill extension alike, which opens the active one.
//!
//! The registry holds nothing secret: ids and user-chosen labels. The primary
//! workspace *is* the data dir itself, so an install that never made a second
//! one has no `workspaces.json` on disk at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write_file;
use crate::error::{Error, Result};

/// The workspace every install starts with, and the only one that is not held
/// in a subdirectory. Its id is stable and reserved: generated ids are hex.
pub const PRIMARY_ID: &str = "default";

/// The registry, in the root data dir — beside the primary workspace's files
/// rather than inside any workspace, since it is what says where those are.
pub const REGISTRY_FILE: &str = "workspaces.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    /// `None` until the user names it, which is the primary's normal state —
    /// the frontend shows a translated default label rather than a name in
    /// whatever language the install was first run in.
    pub name: Option<String>,
    /// The vault id (see [`crate::store::identity`]) of the vault inside, as of
    /// its last sync or its restore — `None` for a vault that has never had one,
    /// which is a vault with no pack on Drive.
    ///
    /// Recorded here, outside the vault, for one reader: a Drive restore asking
    /// whether the pack it was pointed at is already a workspace on this
    /// device. Every workspace but the active one is locked, and the id inside
    /// a locked vault is unreadable — so without this the check could only see
    /// the open workspace, and a duplicate of a locked one went through. Not a
    /// secret: the same id is the pack's file name in the user's own Drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<String>,
    /// How many live entries the vault held when it was last open here — the
    /// lock screen's picker says it beside each workspace. `None` for a vault
    /// not opened on this device since the count was first kept.
    ///
    /// Recorded outside the vault knowingly: it is the one thing about a
    /// vault's contents this file says, readable without the password by
    /// anyone with the disk. It never leaves the device — a Drive pack keeps
    /// even the count sealed (see `sync::pack`). As of the last open or lock
    /// here, so entries another device added show up after this one next syncs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u32>,
    /// The colour the user picked for the workspace's tile: a palette key the
    /// frontend defines (`indigo`, `rose`, …), never a hex value, so each theme
    /// owns what the key looks like. `None` until one is chosen, and the UI
    /// derives a hue from the id instead.
    ///
    /// Device-local, like the count: kept in this file only, never in the vault,
    /// so it does not travel in the pack or reach the user's other devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub active: String,
    pub workspaces: Vec<Workspace>,
}

/// What an install with no registry file means: one workspace, and it is open.
impl Default for Registry {
    fn default() -> Self {
        Self {
            active: PRIMARY_ID.to_string(),
            workspaces: vec![Workspace {
                id: PRIMARY_ID.to_string(),
                name: None,
                vault_id: None,
                item_count: None,
                color: None,
            }],
        }
    }
}

impl Registry {
    /// Read the registry under `root`, or the single-workspace default.
    ///
    /// Never fails. A missing file is the ordinary case (nobody has made a
    /// second workspace), and a corrupt one must not lock the user out of the
    /// vault they already have: the default still names the primary, whose
    /// database is exactly where it has always been.
    pub fn load(root: &Path) -> Registry {
        match Registry::read(root) {
            Ok(Some(registry)) => registry,
            Ok(None) => Registry::default(),
            Err(e) => {
                log::warn!("{e}; assuming one workspace");
                Registry::default()
            }
        }
    }

    /// The registry as it is on disk, for a writer: `None` when there is no
    /// file, and an error — never the default — when there is one that cannot
    /// be read or parsed.
    ///
    /// [`Registry::load`]'s fallback is right for a reader and wrong for a
    /// writer: a change saved over the default it stands in with replaces the
    /// file with a one-workspace registry, and every other workspace drops off
    /// the list for good. A file this cannot read is left as it is.
    pub fn read(root: &Path) -> Result<Option<Registry>> {
        let path = root.join(REGISTRY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let mut registry = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|json| serde_json::from_str::<Registry>(&json).map_err(|e| e.to_string()))
            .map_err(|e| Error::Other(format!("cannot read {}: {e}", path.display())))?;

        // An `active` naming a workspace that is not in the list would resolve
        // to a directory nothing created. The primary always opens.
        if !registry.workspaces.iter().any(|w| w.id == registry.active) {
            registry.active = PRIMARY_ID.to_string();
        }
        Ok(Some(registry))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        atomic_write_file(&root.join(REGISTRY_FILE), &serde_json::to_string(self)?)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.workspaces.iter().any(|w| w.id == id)
    }

    /// The workspace other than `except` that holds the vault with this id, if
    /// the registry knows of one. What a Drive restore asks before adding a
    /// second workspace for a pack this device already syncs (see
    /// [`Workspace::vault_id`] for why the registry is what knows).
    pub fn holder_of(&self, vault_id: &str, except: &str) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.id != except && w.vault_id.as_deref() == Some(vault_id))
    }

    /// Give the workspace `id` a tile colour, or take it away with `None`.
    /// `NotFound` when the registry names no such workspace.
    pub fn recolor(&mut self, id: &str, color: Option<String>) -> Result<()> {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(Error::NotFound)?;
        workspace.color = color;
        Ok(())
    }

    /// The registry as it will read once `id` is gone from this device.
    ///
    /// Deleting the primary is the one case that moves another workspace: the
    /// root *is* the primary's directory ([`dir_of`]), and an install with a
    /// registry but nothing in the root is one whose next unlock opens nothing.
    /// So the first survivor is promoted — it takes `PRIMARY_ID` and keeps its
    /// name and vault id, which are what the user knows it by and what a Drive
    /// restore compares against. First in registry order rather than by name or
    /// by age, so the same install always promotes the same workspace.
    ///
    /// `active` is kept pointing at a workspace that still exists, and under
    /// whatever id it answers to afterwards.
    pub fn without(&self, id: &str) -> Result<Deletion> {
        if !self.contains(id) {
            return Err(Error::NotFound);
        }
        if self.workspaces.len() < 2 {
            return Err(Error::LastWorkspace);
        }

        let mut workspaces: Vec<Workspace> = self
            .workspaces
            .iter()
            .filter(|w| w.id != id)
            .cloned()
            .collect();
        // Before `active` is resolved: the promoted workspace answers to its
        // new id from here on, and `active` may be naming it.
        let promoted = (id == PRIMARY_ID)
            .then(|| std::mem::replace(&mut workspaces[0].id, PRIMARY_ID.to_string()));

        let active = if self.active == id {
            workspaces[0].id.clone()
        } else if promoted.as_deref() == Some(self.active.as_str()) {
            PRIMARY_ID.to_string()
        } else {
            self.active.clone()
        };
        Ok(Deletion {
            registry: Registry { active, workspaces },
            promoted,
        })
    }
}

/// What [`Registry::without`] worked out: the file to save, and the workspace
/// whose files have to be moved into the root before it is saved. The decision
/// only — the app's `workspace::apply_deletion` is what carries it out on disk.
#[derive(Debug, PartialEq)]
pub struct Deletion {
    pub registry: Registry,
    /// The id the promoted workspace's directory still has on disk, when the
    /// primary was the one deleted. `None` for every other delete.
    pub promoted: Option<String>,
}

/// Where a workspace's vault lives.
///
/// The primary is the root itself, not `root/workspaces/default`: that is what
/// makes this change cost existing installs nothing — no migration, no moved
/// files, and a build without workspaces reads the same paths it always did.
pub fn dir_of(root: &Path, id: &str) -> PathBuf {
    if id == PRIMARY_ID {
        root.to_path_buf()
    } else {
        root.join("workspaces").join(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    #[test]
    fn a_root_with_no_registry_file_is_one_active_primary() {
        let registry = Registry::load(&tmp_root());
        assert_eq!(registry, Registry::default());
        assert_eq!(registry.active, PRIMARY_ID);
        assert_eq!(registry.workspaces.len(), 1);
        assert_eq!(registry.workspaces[0].name, None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let root = tmp_root();
        let saved = Registry {
            active: "a1b2".into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "a1b2".into(),
                    name: Some("Work".into()),
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
            ],
        };
        saved.save(&root).unwrap();
        assert_eq!(Registry::load(&root), saved);
    }

    // A registry written before vault ids were recorded still reads, and a
    // workspace with no id is not written with a `null` for one.
    #[test]
    fn a_vault_id_is_optional_on_disk() {
        let root = tmp_root();
        fs::write(
            root.join(REGISTRY_FILE),
            r#"{"active":"default","workspaces":[{"id":"default","name":null}]}"#,
        )
        .unwrap();
        let loaded = Registry::load(&root);
        assert_eq!(loaded.workspaces[0].vault_id, None);

        loaded.save(&root).unwrap();
        let json = fs::read_to_string(root.join(REGISTRY_FILE)).unwrap();
        assert!(!json.contains("vaultId"), "{json}");
        assert!(!json.contains("itemCount"), "{json}");
        assert_eq!(loaded.workspaces[0].item_count, None);
    }

    // A chosen colour survives a save, and an entry written before colours
    // existed reads as having none — and is not written back with a `null`.
    #[test]
    fn a_colour_round_trips_and_is_optional_on_disk() {
        let root = tmp_root();
        fs::write(
            root.join(REGISTRY_FILE),
            r#"{"active":"default","workspaces":[{"id":"default","name":null},{"id":"a1b2","name":"Work"}]}"#,
        )
        .unwrap();
        let mut registry = Registry::load(&root);
        assert_eq!(registry.workspaces[0].color, None);
        assert_eq!(registry.workspaces[1].color, None);

        registry.recolor("a1b2", Some("rose".into())).unwrap();
        registry.save(&root).unwrap();
        let loaded = Registry::load(&root);
        assert_eq!(loaded.workspaces[0].color, None);
        assert_eq!(loaded.workspaces[1].color.as_deref(), Some("rose"));

        let json = fs::read_to_string(root.join(REGISTRY_FILE)).unwrap();
        assert_eq!(json.matches("color").count(), 1, "{json}");
    }

    #[test]
    fn recolor_sets_and_clears_and_refuses_an_unknown_id() {
        let mut registry = Registry::default();
        registry.recolor(PRIMARY_ID, Some("teal".into())).unwrap();
        assert_eq!(registry.workspaces[0].color.as_deref(), Some("teal"));

        registry.recolor(PRIMARY_ID, None).unwrap();
        assert_eq!(registry.workspaces[0].color, None);

        assert!(matches!(
            registry.recolor("missing", Some("teal".into())),
            Err(Error::NotFound)
        ));
    }

    // The question a restore asks: is this pack already a workspace here — any
    // workspace but the one asking, which the caller has already compared live.
    #[test]
    fn the_holder_of_a_vault_is_any_other_workspace_recording_its_id() {
        let registry = Registry {
            active: PRIMARY_ID.into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "a1b2".into(),
                    name: Some("Work".into()),
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
            ],
        };
        // Held by the primary, asked from the other workspace.
        assert_eq!(
            registry.holder_of("cafe", "a1b2").map(|w| w.id.as_str()),
            Some(PRIMARY_ID)
        );
        // The asking workspace does not count as a holder of its own vault.
        assert_eq!(registry.holder_of("cafe", PRIMARY_ID), None);
        // Nobody recorded this one; a workspace that never synced has no id.
        assert_eq!(registry.holder_of("beef", "a1b2"), None);
    }

    #[test]
    fn an_active_nobody_created_falls_back_to_the_primary() {
        let root = tmp_root();
        Registry {
            active: "gone".into(),
            workspaces: vec![Workspace {
                id: PRIMARY_ID.into(),
                name: None,
                vault_id: None,
                item_count: None,
                color: None,
            }],
        }
        .save(&root)
        .unwrap();

        // The list is kept as written; only the dangling pointer into it moves.
        let loaded = Registry::load(&root);
        assert_eq!(loaded.active, PRIMARY_ID);
        assert_eq!(loaded.workspaces.len(), 1);
    }

    #[test]
    fn unparseable_json_reads_as_a_single_workspace_rather_than_failing() {
        let root = tmp_root();
        fs::write(root.join(REGISTRY_FILE), "{ not json").unwrap();
        assert_eq!(Registry::load(&root), Registry::default());
    }

    // Three workspaces in registry order, the middle one named and syncing.
    fn three(active: &str) -> Registry {
        Registry {
            active: active.into(),
            workspaces: vec![
                Workspace {
                    id: PRIMARY_ID.into(),
                    name: None,
                    vault_id: Some("beef".into()),
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "a1b2".into(),
                    name: Some("Work".into()),
                    vault_id: Some("cafe".into()),
                    item_count: None,
                    color: None,
                },
                Workspace {
                    id: "c3d4".into(),
                    name: Some("Side".into()),
                    vault_id: None,
                    item_count: None,
                    color: None,
                },
            ],
        }
    }

    // The ordinary delete: one row goes, nothing else moves, and no files have
    // to follow it anywhere.
    #[test]
    fn deleting_a_workspace_that_is_not_the_primary_only_drops_its_row() {
        let deletion = three(PRIMARY_ID).without("a1b2").unwrap();

        assert_eq!(deletion.promoted, None);
        assert_eq!(deletion.registry.active, PRIMARY_ID);
        assert_eq!(
            deletion
                .registry
                .workspaces
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            [PRIMARY_ID, "c3d4"]
        );
    }

    // The root is the primary's directory, so someone has to move into it. The
    // first survivor does, under the primary's id — and keeps the name the user
    // gave it and the vault id a restore compares against.
    #[test]
    fn deleting_the_primary_promotes_the_first_survivor_into_it() {
        let deletion = three(PRIMARY_ID).without(PRIMARY_ID).unwrap();

        assert_eq!(deletion.promoted.as_deref(), Some("a1b2"));
        assert_eq!(
            deletion.registry.workspaces[0],
            Workspace {
                id: PRIMARY_ID.into(),
                name: Some("Work".into()),
                vault_id: Some("cafe".into()),
                item_count: None,
                color: None,
            }
        );
        // The one that was not promoted is untouched, and still second.
        assert_eq!(deletion.registry.workspaces[1].id, "c3d4");
    }

    // The next unlock resolves `active`, so it may never name a workspace that
    // has just gone.
    #[test]
    fn deleting_the_active_workspace_moves_active_to_the_first_survivor() {
        let deletion = three("c3d4").without("c3d4").unwrap();
        assert_eq!(deletion.registry.active, PRIMARY_ID);

        // Deleting the primary while it is active lands on the promoted one,
        // which is in the root and answers to the primary's id.
        let deletion = three(PRIMARY_ID).without(PRIMARY_ID).unwrap();
        assert_eq!(deletion.registry.active, PRIMARY_ID);
    }

    // The active workspace was the one promoted: it is still active, but its
    // files are in the root now and it answers to the primary's id.
    #[test]
    fn an_active_workspace_promoted_into_the_root_stays_active_as_the_primary() {
        let deletion = three("a1b2").without(PRIMARY_ID).unwrap();

        assert_eq!(deletion.promoted.as_deref(), Some("a1b2"));
        assert_eq!(deletion.registry.active, PRIMARY_ID);
    }

    // Untouched when this delete is about neither of them.
    #[test]
    fn an_active_workspace_nobody_moved_stays_where_it_was() {
        let deletion = three("c3d4").without("a1b2").unwrap();
        assert_eq!(deletion.registry.active, "c3d4");
        assert_eq!(deletion.promoted, None);
    }

    // A device always has a vault: there is no screen an install with none
    // could show, and no way back to one from there.
    #[test]
    fn the_only_workspace_cannot_be_deleted() {
        assert!(matches!(
            Registry::default().without(PRIMARY_ID),
            Err(Error::LastWorkspace)
        ));
    }

    #[test]
    fn a_workspace_that_is_not_in_the_registry_cannot_be_deleted() {
        assert!(matches!(
            three(PRIMARY_ID).without("gone"),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn the_primary_is_the_root_and_everything_else_is_a_subdirectory() {
        let root = Path::new("/data");
        assert_eq!(dir_of(root, PRIMARY_ID), root);
        assert_eq!(dir_of(root, "a1b2"), root.join("workspaces").join("a1b2"));
    }
}
