use std::fs;
use std::path::{Path, PathBuf};

use rowel_core::crypto::{seal_workspace_key, VaultKey};
use rowel_core::models::Entry;
use rowel_core::store::{migrate, SqliteStore, VaultStore};
use rowel_core::workspace::{Registry, Workspace};
use zeroize::Zeroizing;

use super::*;

// The app key the app enrolled: an Argon2id master, as every vault with a KDF
// sidecar has.
const APP_KEY: [u8; 32] = [7; 32];

// A container laid out as the app leaves it: the primary's vault in the data
// dir, with its KDF sidecar and the marker a Face ID enrolment writes.
fn container() -> (tempfile::TempDir, PathBuf) {
    let container = tempfile::tempdir().unwrap();
    let root = layout::app_group_root(container.path());
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(BIOMETRIC_FILE), "protected").unwrap();
    (container, root)
}

fn container_path(container: &tempfile::TempDir) -> String {
    container.path().to_string_lossy().into_owned()
}

// A vault at `dir` keyed by `material`, holding `entries`, sealed and
// upserted as the app saves them.
fn vault_at(dir: &Path, material: &[u8], entries: &[Entry]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(KDF_SIDECAR_FILE), "{}").unwrap();
    let key = VaultKey::from_material(Zeroizing::new(material.to_vec()), true);
    let store = SqliteStore::open(&dir.join(DB_FILE), key.sqlcipher_key().as_slice()).unwrap();
    let cipher = key.payload_cipher();
    for entry in entries {
        let payload = cipher.seal(entry).unwrap();
        store
            .upsert(&migrate::build_record(entry, payload).unwrap())
            .unwrap();
    }
}

fn login(id: &str, website: &str, username: &str, password: &str) -> Entry {
    serde_json::from_value(serde_json::json!({
        "id": id, "type": "login", "title": id,
        "website": website, "username": username, "password": password
    }))
    .unwrap()
}

fn open_primary(container: &tempfile::TempDir) -> Arc<Vault> {
    let location = vault_location(container_path(container)).unwrap();
    open_vault(location, APP_KEY.to_vec()).unwrap()
}

fn records(vault: &Vault, sites: &[&str]) -> Vec<String> {
    let mut records: Vec<String> = vault
        .credentials_for(sites.iter().map(|s| s.to_string()).collect())
        .unwrap()
        .into_iter()
        .map(|c| c.record)
        .collect();
    records.sort();
    records
}

// --- where the vault is ---------------------------------------------------

// No registry is the one-workspace install: the primary, in the data dir, and
// the keychain item the protected gate stores the app key under.
#[test]
fn with_no_registry_the_vault_is_the_primary_in_the_data_dir() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[]);

    assert_eq!(
        vault_location(container_path(&container)).unwrap(),
        VaultLocation {
            root: root.to_string_lossy().into_owned(),
            workspace: PRIMARY_ID.into(),
            keychain_service: keychain::SERVICE.into(),
            keychain_account: keychain::ACCOUNT.into(),
        }
    );
}

// The registry names the active workspace, whose vault is in its own
// directory, and opens through its key sealed under the app key.
#[test]
fn the_registry_names_the_workspace_that_is_opened() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[login("home", "home.test", "a", "p")]);
    let own_key = [9u8; 32];
    let dir = dir_of(&root, "w2");
    vault_at(&dir, &own_key, &[login("work", "work.test", "b", "q")]);
    fs::write(
        dir.join(WRAPPED_KEY_FILE),
        seal_workspace_key(&APP_KEY, "w2", &own_key).unwrap(),
    )
    .unwrap();
    let workspace = |id: &str| Workspace {
        id: id.into(),
        name: None,
        vault_id: None,
        item_count: None,
        color: None,
    };
    Registry {
        active: "w2".into(),
        workspaces: vec![workspace(PRIMARY_ID), workspace("w2")],
    }
    .save(&root)
    .unwrap();

    let location = vault_location(container_path(&container)).unwrap();
    assert_eq!(location.workspace, "w2");
    let vault = open_vault(location, APP_KEY.to_vec()).unwrap();
    assert_eq!(records(&vault, &[]), ["w2/work"]);
}

// A workspace with nothing sealed under the app key only opens with its own
// password, which the extension does not ask for.
#[test]
fn a_workspace_with_no_sealed_key_is_locked() {
    let (container, root) = container();
    vault_at(&dir_of(&root, "w2"), &[9; 32], &[]);
    let mut registry = Registry::default();
    registry.workspaces.push(Workspace {
        id: "w2".into(),
        name: None,
        vault_id: None,
        item_count: None,
        color: None,
    });
    registry.active = "w2".into();
    registry.save(&root).unwrap();

    assert!(matches!(
        vault_location(container_path(&container)),
        Err(AutofillError::Locked)
    ));
}

// The marker names the gate, and with it the item's account.
#[test]
fn a_prompt_enrolment_is_read_from_its_own_account() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[]);
    fs::write(root.join(BIOMETRIC_FILE), "prompt").unwrap();

    let location = vault_location(container_path(&container)).unwrap();
    assert_eq!(location.keychain_account, keychain::ACCOUNT_PROMPT);
}

#[test]
fn no_face_id_enrolment_is_locked() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[]);
    fs::remove_file(root.join(BIOMETRIC_FILE)).unwrap();

    assert!(matches!(
        vault_location(container_path(&container)),
        Err(AutofillError::Locked)
    ));
}

// An empty container is no vault — and opening it anyway creates none.
#[test]
fn an_empty_container_has_no_vault_and_is_left_empty() {
    let (container, root) = container();
    assert!(matches!(
        vault_location(container_path(&container)),
        Err(AutofillError::NoVault)
    ));

    let location = VaultLocation {
        root: root.to_string_lossy().into_owned(),
        workspace: PRIMARY_ID.into(),
        keychain_service: keychain::SERVICE.into(),
        keychain_account: keychain::ACCOUNT.into(),
    };
    assert!(matches!(
        open_vault(location, APP_KEY.to_vec()),
        Err(AutofillError::NoVault)
    ));
    assert!(!root.join(DB_FILE).exists());
}

// A master-password change the app was stopped in the middle of: the app
// rolls the vault back at its next unlock, and until then the extension keeps
// out — before it asks for Face ID, and at the open.
#[test]
fn a_password_change_in_flight_keeps_the_vault_closed() {
    let (container, root) = container();
    vault_at(
        &root,
        &APP_KEY,
        &[login("github", "github.com", "alice", "pw")],
    );
    let location = vault_location(container_path(&container)).unwrap();
    fs::write(root.join(DB_REKEY_BACKUP_FILE), "snapshot").unwrap();

    assert!(matches!(
        vault_location(container_path(&container)),
        Err(AutofillError::Recovering)
    ));
    assert!(matches!(
        open_vault(location.clone(), APP_KEY.to_vec()),
        Err(AutofillError::Recovering)
    ));

    fs::remove_file(root.join(DB_REKEY_BACKUP_FILE)).unwrap();
    assert!(open_vault(location, APP_KEY.to_vec()).is_ok());
}

#[test]
fn a_key_that_does_not_open_the_vault_is_the_wrong_key() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[]);
    let location = vault_location(container_path(&container)).unwrap();

    assert!(matches!(
        open_vault(location, vec![8; 32]),
        Err(AutofillError::WrongKey)
    ));
}

// --- which logins are offered ---------------------------------------------

fn matching_vault() -> (tempfile::TempDir, Arc<Vault>) {
    let (container, root) = container();
    vault_at(
        &root,
        &APP_KEY,
        &[
            login("github", "https://github.com/login", "alice", "pw1"),
            login("gist", "https://gist.github.com", "alice", "pw2"),
            login("bank", "https://bank.co.uk", "bob", "pw3"),
            login("suffix", "https://co.uk", "eve", "pw4"),
            login("lookalike", "https://notgithub.com", "mallory", "pw5"),
        ],
    );
    let vault = open_primary(&container);
    (container, vault)
}

// A site gets its own logins and its parents', whether iOS names it as a
// domain or as the page's URL; a lookalike gets neither.
#[test]
fn a_site_is_offered_its_own_and_its_parents_logins() {
    let (_container, vault) = matching_vault();

    assert_eq!(records(&vault, &["github.com"]), ["default/github"]);
    assert_eq!(
        records(&vault, &["gist.github.com"]),
        ["default/gist", "default/github"]
    );
    assert_eq!(
        records(&vault, &["https://www.github.com/login?next=/"]),
        ["default/github"]
    );
    assert!(records(&vault, &["example.com"]).is_empty());
}

// A login saved under a public suffix is everyone's parent, so it is no one's.
#[test]
fn a_login_under_a_public_suffix_is_not_offered_beneath_it() {
    let (_container, vault) = matching_vault();

    assert_eq!(records(&vault, &["online.bank.co.uk"]), ["default/bank"]);
}

// With no site to go by, every login is on offer; and nothing in the list is
// a secret.
#[test]
fn no_identifiers_offers_every_login_by_name() {
    let (_container, vault) = matching_vault();

    let all = vault.credentials_for(vec![]).unwrap();
    assert_eq!(all.len(), 5);
    let github = all.iter().find(|c| c.record == "default/github").unwrap();
    assert_eq!(
        github,
        &Credential {
            record: "default/github".into(),
            title: "github".into(),
            user: "alice".into(),
            host: "github.com".into(),
        }
    );
}

// A login that signs in with an email lists by it, as QuickType shows it.
#[test]
fn a_login_with_no_username_is_listed_by_its_email() {
    let (container, root) = container();
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "e1", "type": "login", "title": "Acme", "website": "acme.test",
        "email": "alice@acme.test", "password": "pw"
    }))
    .unwrap();
    vault_at(&root, &APP_KEY, &[entry]);

    let vault = open_primary(&container);
    let listed = vault.credentials_for(vec!["acme.test".into()]).unwrap();
    assert_eq!(listed[0].user, "alice@acme.test");
    assert_eq!(
        vault
            .password("default/e1".into(), vec!["acme.test".into()])
            .unwrap(),
        Password {
            user: "alice@acme.test".into(),
            password: "pw".into(),
        }
    );
}

// --- filling --------------------------------------------------------------

#[test]
fn a_record_fills_its_own_name_and_password() {
    let (_container, vault) = matching_vault();

    assert_eq!(
        vault
            .password(
                "default/bank".into(),
                vec!["https://bank.co.uk/login".into()]
            )
            .unwrap(),
        Password {
            user: "bob".into(),
            password: "pw3".into(),
        }
    );
}

// Gone, another workspace's, or no record at all: nothing to fill.
#[test]
fn a_record_that_is_not_here_is_not_found() {
    let (_container, vault) = matching_vault();

    for record in ["default/gone", "w2/bank", "bank", ""] {
        assert!(
            matches!(
                vault.password(record.into(), vec![]),
                Err(AutofillError::NotFound)
            ),
            "{record}"
        );
    }
}

// The site iOS is filling for is checked against the vault at the fill, not
// taken from the suggestion: the record of a login moved to another site
// since the app published it is refused for the old one — and admitted for
// the new one, its parents, or when iOS names no site at all.
#[test]
fn a_record_fills_only_for_a_site_its_login_is_for() {
    let (_container, vault) = matching_vault();
    let fill = |sites: &[&str]| {
        vault.password(
            "default/gist".into(),
            sites.iter().map(|s| s.to_string()).collect(),
        )
    };

    assert!(matches!(
        fill(&["notgithub.com"]),
        Err(AutofillError::NotFound)
    ));
    assert!(matches!(
        fill(&["github.com"]),
        Err(AutofillError::NotFound)
    ));
    assert_eq!(fill(&["gist.github.com"]).unwrap().password, "pw2");
    assert_eq!(
        fill(&["https://gist.github.com/alice"]).unwrap().password,
        "pw2"
    );
    assert_eq!(
        fill(&["example.com", "gist.github.com"]).unwrap().password,
        "pw2"
    );
    assert_eq!(fill(&[]).unwrap().password, "pw2");
}

// A login with a passkey on it, as the app names it for the site.
fn with_passkey(id: &str, website: &str, username: &str, password: &str) -> Entry {
    let mut entry = login(id, website, username, password);
    entry.passkeys = Some(vec![serde_json::from_value(serde_json::json!({
        "credentialId": "Y3JlZA", "rpId": website, "userHandle": "dXNlcg",
        "userName": username, "userDisplayName": "Alice Example"
    }))
    .unwrap()]);
    entry
}

// A login without a password is not a password to offer, in the list or at
// the fill, by the rule the app's QuickType identities go by: a passkey on
// the row, or no username, and the payload decides — the passkey-only kind an
// import brings, and an email login with nothing to fill, are left out. A
// login with a username and no passkey the listing vouches for, as the app
// does, so it is offered, and fills what it has.
#[test]
fn a_login_with_no_password_is_not_offered_as_one() {
    let (container, root) = container();
    let email_only: Entry = serde_json::from_value(serde_json::json!({
        "id": "mail", "type": "login", "title": "mail", "website": "acme.test",
        "email": "alice@acme.test"
    }))
    .unwrap();
    vault_at(
        &root,
        &APP_KEY,
        &[
            with_passkey("keyed", "acme.test", "alice", ""),
            with_passkey("both", "acme.test", "alice", "pw0"),
            email_only,
            login("bare", "acme.test", "carol", ""),
            login("filled", "acme.test", "bob", "pw"),
        ],
    );

    let vault = open_primary(&container);
    assert_eq!(
        records(&vault, &["acme.test"]),
        ["default/bare", "default/both", "default/filled"]
    );
    assert_eq!(records(&vault, &[]), records(&vault, &["acme.test"]));
    let fill = |record: &str| vault.password(record.into(), vec!["acme.test".into()]);
    assert!(matches!(
        fill("default/keyed"),
        Err(AutofillError::NotFound)
    ));
    assert!(matches!(fill("default/mail"), Err(AutofillError::NotFound)));
    assert_eq!(fill("default/both").unwrap().password, "pw0");
    assert_eq!(fill("default/bare").unwrap().password, "");
}

// A login in the trash is gone as far as filling goes, whatever iOS still
// holds for it.
#[test]
fn a_deleted_login_is_not_filled() {
    let (container, root) = container();
    vault_at(&root, &APP_KEY, &[login("old", "acme.test", "alice", "pw")]);
    let vault = open_primary(&container);
    vault.store.delete("old").unwrap();

    assert!(records(&vault, &["acme.test"]).is_empty());
    assert!(matches!(
        vault.password("default/old".into(), vec!["acme.test".into()]),
        Err(AutofillError::NotFound)
    ));
}
