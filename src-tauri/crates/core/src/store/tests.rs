use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{migrate, record_hash, Record, SqliteStore, StoreError, VaultStore, SYNC_META_PREFIX};
use crate::crypto::{self, KdfParams, VaultKey};
use crate::models::Entry;

const KEY: &[u8] = &[0x11; 32];

// Low-cost Argon2id params keep the key-schedule tests fast; production uses the
// 64 MiB defaults.
fn argon2_params(salt: &[u8]) -> KdfParams {
    KdfParams::argon2id(salt, 256, 1, 1)
}

fn argon2_key(password: &str, params: &KdfParams) -> VaultKey {
    VaultKey::Argon2 {
        master: crypto::derive(password.as_bytes(), params).unwrap(),
    }
}

fn sample_entry() -> Entry {
    serde_json::from_value(serde_json::json!({
        "id": "1", "type": "login", "title": "Site",
        "website": "https://ex.com/login", "username": "alice",
        "password": "s3cret", "tags": ["work"]
    }))
    .unwrap()
}

// A login whose only credential is a passkey: no website and no password, which
// is exactly the shape a Bitwarden import of a passkey-only item lands in.
fn passkey_entry() -> Entry {
    serde_json::from_value(serde_json::json!({
        "id": "p1", "type": "login", "title": "Acme",
        "passkeys": [{
            "credentialId": "Y3JlZDE", "rpId": "acme.test", "rpName": "Acme",
            "userHandle": "dWgx", "userName": "alice",
            "userDisplayName": "Alice", "privateKey": "cGsx"
        }]
    }))
    .unwrap()
}

// A fresh, unique DB path under the temp dir for each test.
fn tmp_db() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rowel-store-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("vault.db")
}

fn rec(id: &str, payload: &[u8]) -> Record {
    Record {
        id: id.into(),
        kind: "login".into(),
        title: "Example".into(),
        tags: "[\"a\",\"b\"]".into(),
        url_host: "example.com".into(),
        created_at: 0,
        updated_at: 0,
        deleted_at: None,
        payload: payload.to_vec(),
        card_brand: None,
        favorite: false,
        has_passkey: false,
        file_name: None,
        var_count: None,
        username: None,
    }
}

// A record with the timestamps merge actually reasons about. `created_at` is
// held constant so a differing `updated_at` is the only axis under test.
fn stamped(id: &str, payload: &[u8], updated_at: i64) -> Record {
    Record {
        created_at: 100,
        updated_at,
        ..rec(id, payload)
    }
}

// An ordinary tombstone: `delete` only stamps `deleted_at`, so the payload is
// still there. Keeping it non-empty here is what separates this from the
// content-free shell `purge` leaves behind, which the merge treats as absorbing.
fn tombstone(id: &str, at: i64) -> Record {
    Record {
        deleted_at: Some(at),
        ..stamped(id, b"sealed-payload", at)
    }
}

// Seed rows with their timestamps intact (import, unlike upsert, does not stamp).
fn seeded(recs: &[Record]) -> SqliteStore {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.import(recs).unwrap();
    store
}

fn row(store: &SqliteStore, id: &str) -> Record {
    store
        .export_for_sync()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
}

#[test]
fn crud_round_trip() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.upsert(&rec("1", b"secret-blob")).unwrap();

    let got = store.get("1").unwrap().unwrap();
    assert_eq!(got.id, "1");
    assert_eq!(got.title, "Example");
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.get("missing").unwrap(), None);
}

#[test]
fn payload_is_byte_identical() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    // Arbitrary non-utf8 bytes: the store must never interpret the payload.
    let blob: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
    store.upsert(&rec("1", &blob)).unwrap();
    assert_eq!(store.get("1").unwrap().unwrap().payload, blob);
}

#[test]
fn delete_tombstones_and_hides_from_list() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.upsert(&rec("1", b"x")).unwrap();
    store.delete("1").unwrap();

    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.get("1").unwrap(), None);

    let exported = store.export_for_sync().unwrap();
    assert_eq!(exported.len(), 1);
    assert!(exported[0].deleted_at.is_some());
}

// The count the lock screen shows is the list's length, never the table's:
// archived rows and purged shells are tombstones, not items.
#[test]
fn count_live_counts_what_the_list_shows() {
    let store = seeded(&[
        rec("1", b"a"),
        rec("2", b"b"),
        tombstone("3", 1_000),
        Record {
            payload: Vec::new(),
            ..tombstone("4", 2_000)
        },
    ]);
    assert_eq!(store.count_live().unwrap(), 2);
    assert_eq!(
        store.count_live().unwrap() as usize,
        store.list().unwrap().len()
    );
}

#[test]
fn list_deleted_lists_tombstones_newest_first() {
    let store = seeded(&[
        stamped("live", b"x", 100),
        Record {
            deleted_at: Some(200),
            ..stamped("old", b"x", 200)
        },
        Record {
            deleted_at: Some(300),
            ..stamped("new", b"x", 300)
        },
    ]);

    let ids: Vec<String> = store
        .list_deleted()
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["new", "old"]);
}

#[test]
fn restore_clears_the_tombstone_and_bumps_updated_at() {
    let store = seeded(&[Record {
        deleted_at: Some(1000),
        ..stamped("1", b"x", 1000)
    }]);

    store.restore("1").unwrap();

    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.list_deleted().unwrap().is_empty());
    // The bump is what lets the restore beat the tombstone peers still hold.
    assert!(row(&store, "1").updated_at > 1000);
}

#[test]
fn purge_empties_the_row_and_drops_it_from_the_trash() {
    let store = seeded(&[Record {
        deleted_at: Some(1000),
        title: "Secret Thing".into(),
        username: Some("alice".into()),
        ..stamped("1", b"sealed-payload", 1000)
    }]);

    store.purge("1").unwrap();

    // The tombstone stays (sync still needs to know the id is gone) but carries
    // nothing, so the Trash no longer offers it.
    assert!(store.list_deleted().unwrap().is_empty());
    let purged = row(&store, "1");
    assert!(purged.deleted_at.is_some());
    assert!(purged.payload.is_empty());
    assert_eq!(purged.title, "");
    assert_eq!(purged.username, None);
    assert!(purged.updated_at > 1000);
}

#[test]
fn purge_only_touches_tombstones() {
    let store = seeded(&[stamped("1", b"sealed-payload", 1000)]);

    store.purge("1").unwrap();

    assert_eq!(store.get("1").unwrap().unwrap().payload, b"sealed-payload");
}

// "Delete forever" has to hold for someone who later gets the database key
// too: the freed ciphertext must be zeroed, not left in a free page until it
// happens to be reused. The pragma is set per connection, so this checks the
// one `open` configures.
#[test]
fn the_store_zeroes_what_it_frees() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    assert_eq!(store.pragma_i64("secure_delete").unwrap(), 1);
}

#[test]
fn purge_beats_a_peer_that_still_holds_the_row() {
    let store = seeded(&[Record {
        deleted_at: Some(1000),
        ..stamped("1", b"sealed-payload", 1000)
    }]);
    store.purge("1").unwrap();

    // The peer never saw the purge and pushes the row back, payload included.
    // A hard DELETE would take it; the stamped empty row must win instead.
    store
        .merge_records(&[Record {
            deleted_at: Some(1000),
            ..stamped("1", b"sealed-payload", 1000)
        }])
        .unwrap();

    assert!(row(&store, "1").payload.is_empty());
    assert!(store.list_deleted().unwrap().is_empty());
}

// The sibling case, and the one last-writer-wins gets wrong: the peer did not
// just re-push the row, it *edited* it after the purge, so its copy is newer.
// LWW would hand the sealed payload back live — "Delete forever" has to beat a
// newer stamp, not merely an equal one.
#[test]
fn purge_beats_a_peer_that_edited_the_row_afterwards() {
    let store = seeded(&[Record {
        deleted_at: Some(1000),
        ..stamped("1", b"sealed-payload", 1000)
    }]);
    store.purge("1").unwrap();

    store
        .merge_records(&[stamped("1", b"edited-after-the-purge", i64::MAX)])
        .unwrap();

    assert!(row(&store, "1").payload.is_empty());
    assert!(row(&store, "1").deleted_at.is_some());
    assert!(store.list().unwrap().is_empty());
    assert!(store.list_deleted().unwrap().is_empty());
}

// The other direction, and the reason the guard is symmetric: the purge has to
// *travel*. Were the shell only absorbing locally, the peer that holds a newer
// edit would keep its payload, the two digests would never match, and the pair
// would push at each other forever.
#[test]
fn a_purged_shell_destroys_the_payload_on_the_peer_too() {
    let purged = seeded(&[Record {
        deleted_at: Some(1000),
        ..stamped("1", b"sealed-payload", 1000)
    }]);
    purged.purge("1").unwrap();
    let peer = seeded(&[stamped("1", b"edited-after-the-purge", i64::MAX)]);

    peer.merge_records(&purged.export_for_sync().unwrap())
        .unwrap();

    assert!(row(&peer, "1").payload.is_empty());
    assert!(peer.list().unwrap().is_empty());
    // Both sides converged on the shell, so nothing is left to push.
    assert_eq!(peer.state_digest().unwrap(), purged.state_digest().unwrap());
    // And replaying it changes nothing.
    assert_eq!(
        peer.merge_records(&purged.export_for_sync().unwrap())
            .unwrap(),
        0
    );
}

#[test]
fn set_favorite_round_trips_and_bumps_updated_at() {
    let store = seeded(&[stamped("1", b"x", 1000)]);
    assert!(!store.list().unwrap()[0].favorite);

    store.set_favorite("1", true).unwrap();

    assert!(store.list().unwrap()[0].favorite);
    // Last-writer-wins on updated_at is the only reason a star propagates.
    assert!(row(&store, "1").updated_at > 1000);

    store.set_favorite("1", false).unwrap();
    assert!(!store.list().unwrap()[0].favorite);
}

// The Trash offers no star (`favorite-toggle` is not rendered on a tombstone),
// so this only guards the store — but the bump would give a deleted row a fresh
// `updated_at` for the sync merge to carry, which is worth refusing outright.
#[test]
fn set_favorite_only_touches_live_rows() {
    let store = seeded(&[Record {
        deleted_at: Some(1000),
        ..stamped("1", b"x", 1000)
    }]);

    store.set_favorite("1", true).unwrap();

    let got = row(&store, "1");
    assert!(!got.favorite);
    assert_eq!(got.updated_at, 1000);
}

// The commands that report back a row they just wrote read it through
// `row_meta`, which — like the write itself — has to see rows `list` hides.
#[test]
fn row_meta_sees_a_tombstone_that_list_hides() {
    let store = seeded(&[stamped("1", b"x", 1000)]);
    store.delete("1").unwrap();

    assert!(store.list().unwrap().is_empty());
    let meta = store.row_meta("1").unwrap().unwrap();
    assert!(meta.deleted_at.is_some());
    assert_eq!(meta.title, "Example");
    assert_eq!(store.row_meta("missing").unwrap(), None);
}

#[test]
fn saving_an_entry_leaves_the_star_alone() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.upsert(&rec("1", b"x")).unwrap();
    store.set_favorite("1", true).unwrap();

    // An ordinary save carries favorite: false (build_record never reads a star),
    // so `upsert` must not write the column over the top of set_favorite.
    store.upsert(&rec("1", b"edited")).unwrap();

    assert!(store.list().unwrap()[0].favorite);
    assert_eq!(store.get("1").unwrap().unwrap().payload, b"edited");
}

#[test]
fn favorite_travels_through_a_sync_merge() {
    let store = seeded(&[stamped("1", b"x", 1000)]);

    store
        .merge_records(&[Record {
            favorite: true,
            ..stamped("1", b"x", 2000)
        }])
        .unwrap();

    assert!(store.list().unwrap()[0].favorite);
}

#[test]
fn favorite_is_part_of_a_records_identity() {
    let plain = stamped("1", b"x", 1000);
    let starred = Record {
        favorite: true,
        ..plain.clone()
    };
    assert_ne!(record_hash(&plain), record_hash(&starred));
}

#[test]
fn build_record_flags_an_entry_that_holds_a_passkey() {
    let plain = migrate::build_record(&sample_entry(), b"sealed".to_vec()).unwrap();
    assert!(!plain.has_passkey);

    let with_key = migrate::build_record(&passkey_entry(), b"sealed".to_vec()).unwrap();
    assert!(with_key.has_passkey);
}

fn env_entry(file_name: Option<&str>, body: &str) -> Entry {
    let mut value = serde_json::json!({
        "id": "e1", "type": "env", "title": "api · production", "body": body
    });
    if let Some(name) = file_name {
        value["fileName"] = serde_json::Value::String(name.into());
    }
    serde_json::from_value(value).unwrap()
}

// The list subtitle is `fileName · N vars`, drawn from columns so the row needs
// no reveal — so both have to come off the plaintext at save time, and a name
// the file never had has to stay absent rather than read as an empty string.
#[test]
fn build_record_stamps_an_env_files_name_and_variable_count() {
    let body = "# Database\nDATABASE_URL=postgres://x\nexport DB_POOL=10\n\nPORT = 3000 # dev\n";
    let named =
        migrate::build_record(&env_entry(Some(".env.production"), body), b"s".to_vec()).unwrap();
    assert_eq!(named.file_name.as_deref(), Some(".env.production"));
    assert_eq!(named.var_count, Some(3));

    let pasted = migrate::build_record(&env_entry(None, body), b"s".to_vec()).unwrap();
    assert_eq!(pasted.file_name, None);
    assert_eq!(pasted.var_count, Some(3));

    // A blank name is no name.
    let blank = migrate::build_record(&env_entry(Some("  "), body), b"s".to_vec()).unwrap();
    assert_eq!(blank.file_name, None);

    // Every other kind leaves both NULL, so the columns say nothing about it.
    let login = migrate::build_record(&sample_entry(), b"s".to_vec()).unwrap();
    assert_eq!((login.file_name, login.var_count), (None, None));
}

#[test]
fn build_record_stamps_a_logins_username() {
    let login = migrate::build_record(&sample_entry(), b"s".to_vec()).unwrap();
    assert_eq!(login.username.as_deref(), Some("alice"));

    let bare = migrate::build_record(&passkey_entry(), b"s".to_vec()).unwrap();
    assert_eq!(bare.username.as_deref(), Some(""));

    let env = migrate::build_record(&env_entry(None, "A=1\n"), b"s".to_vec()).unwrap();
    assert_eq!(env.username, None);
}

// What counts as a variable line, and what does not: comments, blank lines,
// a bare word, a name starting with a digit, and `export` glued to `=`.
#[test]
fn env_var_count_reads_assignment_lines_only() {
    let count = |body: &str| migrate::derived_var_count(&env_entry(None, body)).unwrap();
    assert_eq!(count(""), 0);
    assert_eq!(count("# only a comment\n\n"), 0);
    assert_eq!(count("A=1\nB=\n_C = 3\n  export D=4\n"), 4);
    assert_eq!(count("1A=1\nno equals\n=oops\nexport\n"), 0);
    // `export=1` names a variable called `export`; `export  X=1` names `X`.
    assert_eq!(count("export=1\nexport\tX=1\n"), 2);
    // CRLF endings do not break the line walk.
    assert_eq!(count("A=1\r\nB=2\r\n"), 2);
}

// An imported passkey-only login has no website, and the relying-party id is
// the site the credential is bound to — otherwise the row has no subtitle and
// typing the site's name finds nothing.
#[test]
fn build_record_falls_back_to_the_passkeys_rp_id_for_the_host() {
    let record = migrate::build_record(&passkey_entry(), b"sealed".to_vec()).unwrap();
    assert_eq!(record.url_host, "acme.test");

    // A website of its own still wins: it is what the user typed.
    let mut entry = passkey_entry();
    entry.website = Some("https://login.acme.example/in".into());
    let record = migrate::build_record(&entry, b"sealed".to_vec()).unwrap();
    assert_eq!(record.url_host, "login.acme.example");
}

#[test]
fn has_passkey_round_trips_through_upsert_get_and_list() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store
        .upsert(&migrate::build_record(&passkey_entry(), b"sealed".to_vec()).unwrap())
        .unwrap();

    assert!(store.get("p1").unwrap().unwrap().has_passkey);
    assert!(store.list().unwrap()[0].has_passkey);
    assert!(store.row_meta("p1").unwrap().unwrap().has_passkey);
}

// The mirror image of `saving_an_entry_leaves_the_star_alone`: this flag IS
// derived from the payload being written, so removing the last passkey has to
// clear it rather than leave a glyph on a row that no longer has one.
#[test]
fn saving_an_entry_updates_the_passkey_flag() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store
        .upsert(&migrate::build_record(&passkey_entry(), b"sealed".to_vec()).unwrap())
        .unwrap();

    let mut stripped = passkey_entry();
    stripped.passkeys = None;
    store
        .upsert(&migrate::build_record(&stripped, b"sealed".to_vec()).unwrap())
        .unwrap();

    assert!(!store.list().unwrap()[0].has_passkey);
}

#[test]
fn has_passkey_survives_import_and_export_for_sync() {
    let flagged = Record {
        has_passkey: true,
        ..stamped("1", b"x", 1000)
    };
    let store = seeded(std::slice::from_ref(&flagged));

    assert_eq!(store.export_for_sync().unwrap(), vec![flagged]);
    assert!(store.list().unwrap()[0].has_passkey);
}

#[test]
fn has_passkey_is_part_of_a_records_identity() {
    let plain = stamped("1", b"x", 1000);
    let flagged = Record {
        has_passkey: true,
        ..plain.clone()
    };
    assert_ne!(record_hash(&plain), record_hash(&flagged));
}

#[test]
fn upsert_stamps_updated_at() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    // Seed via import so the low timestamp is preserved, then upsert to bump it.
    let mut r = rec("1", b"x");
    r.updated_at = 1000;
    store.import(&[r]).unwrap();

    store.upsert(&rec("1", b"y")).unwrap();
    assert!(store.get("1").unwrap().unwrap().updated_at > 1000);
}

#[test]
fn import_preserves_timestamps() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    let mut r = rec("1", b"x");
    r.created_at = 111;
    r.updated_at = 222;
    store.import(&[r]).unwrap();

    let got = &store.export_for_sync().unwrap()[0];
    assert_eq!(got.created_at, 111);
    assert_eq!(got.updated_at, 222);
}

#[test]
fn meta_get_set() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    assert_eq!(store.meta_get("kdf").unwrap(), None);
    store.meta_set("kdf", "argon2id").unwrap();
    assert_eq!(store.meta_get("kdf").unwrap().as_deref(), Some("argon2id"));
    // Schema DDL versioning lives in `user_version`, not the app `meta` table.
    assert_eq!(store.meta_get("schema_version").unwrap(), None);
}

#[test]
fn meta_set_many_writes_every_pair() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store
        .meta_set_many(&[("vault_name", "Work"), ("vault_name_updated_ms", "7")])
        .unwrap();
    assert_eq!(
        store.meta_get("vault_name").unwrap().as_deref(),
        Some("Work")
    );
    assert_eq!(
        store.meta_get("vault_name_updated_ms").unwrap().as_deref(),
        Some("7")
    );
}

// The point of the batch: a pair that only means anything together must never
// be left half-written. A name under the stamp of the name before it is a
// rename every other device then reads as stale and refuses.
#[test]
fn a_failed_batch_writes_none_of_its_pairs() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store
        .refuse_meta_key_for_test("vault_name_updated_ms")
        .unwrap();

    let failed = store.meta_set_many(&[("vault_name", "Work"), ("vault_name_updated_ms", "7")]);

    assert!(failed.is_err());
    assert_eq!(store.meta_get("vault_name").unwrap(), None);
    assert_eq!(store.meta_get("vault_name_updated_ms").unwrap(), None);
}

// The one open failure that may reach the user as "wrong password": it is the
// only one that names the key, and the only one the unlock lockout counts.
#[test]
fn wrong_key_fails() {
    let path = tmp_db();
    SqliteStore::open(&path, KEY)
        .unwrap()
        .upsert(&rec("1", b"x"))
        .unwrap();
    match SqliteStore::open(&path, &[0x22; 32]) {
        Err(StoreError::WrongKey) => {}
        Err(other) => panic!("expected WrongKey, got error: {other}"),
        Ok(_) => panic!("expected WrongKey, but the open succeeded"),
    }
}

#[test]
fn reopen_after_commit_persists() {
    let path = tmp_db();
    {
        let store = SqliteStore::open(&path, KEY).unwrap();
        store.upsert(&rec("1", b"durable")).unwrap();
    }
    let store = SqliteStore::open(&path, KEY).unwrap();
    assert_eq!(store.get("1").unwrap().unwrap().payload, b"durable");
}

#[cfg(unix)]
#[test]
fn file_mode_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let path = tmp_db();
    SqliteStore::open(&path, KEY).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn snapshot_is_encrypted_and_reopens() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.upsert(&rec("1", b"durable")).unwrap();

    let snap = tmp_db().with_file_name("snapshot.db");
    store.snapshot_to(&snap, KEY).unwrap();

    // The snapshot reopens with the same key and carries the committed row.
    let restored = SqliteStore::open(&snap, KEY).unwrap();
    assert_eq!(restored.get("1").unwrap().unwrap().payload, b"durable");
    // It is genuinely encrypted: a wrong key cannot open it.
    assert!(SqliteStore::open(&snap, &[0x22; 32]).is_err());
}

#[test]
fn rekey_reencrypts_db_in_place() {
    let path = tmp_db();
    let store = SqliteStore::open(&path, KEY).unwrap();
    store.upsert(&rec("1", b"durable")).unwrap();

    const KEY2: &[u8] = &[0x22; 32];
    store.rekey(KEY2).unwrap();
    // The live handle keeps working after rekey.
    assert_eq!(store.get("1").unwrap().unwrap().payload, b"durable");
    // Rekey checkpoints: the re-encrypted pages are in the main file and no
    // frame of the old-key database is left behind in the WAL. Without this,
    // `vault.db` alone still opens under the previous password.
    let wal = path.with_file_name("vault.db-wal");
    assert_eq!(wal.metadata().map(|m| m.len()).unwrap_or(0), 0);
    drop(store);

    // Reopening requires the new key; the old key no longer opens it.
    assert!(SqliteStore::open(&path, KEY).is_err());
    let store = SqliteStore::open(&path, KEY2).unwrap();
    assert_eq!(store.get("1").unwrap().unwrap().payload, b"durable");
}

// The one open failure that must NOT read as a wrong key: a vault stamped by
// a newer build (user_version above this binary's migration list).
#[test]
fn future_schema_fails_as_schema_newer_not_wrong_key() {
    let path = tmp_db();
    {
        let store = SqliteStore::open(&path, KEY).unwrap();
        store.set_user_version(99).unwrap();
    }
    match SqliteStore::open(&path, KEY) {
        Err(StoreError::SchemaNewer) => {}
        Err(other) => panic!("expected SchemaNewer, got error: {other}"),
        Ok(_) => panic!("expected SchemaNewer, but the open succeeded"),
    }
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const MISS_TTL_MS: i64 = 7 * DAY_MS;

#[test]
fn favicon_round_trips_an_icon_and_a_miss() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    // Nothing recorded is not the same answer as "recorded: no icon".
    assert_eq!(store.get_favicon("ex.com", MISS_TTL_MS).unwrap(), None);

    store
        .put_favicon("ex.com", Some("data:image/png;base64,AA"))
        .unwrap();
    store.put_favicon("nope.com", None).unwrap();

    assert_eq!(
        store.get_favicon("ex.com", MISS_TTL_MS).unwrap(),
        Some(Some("data:image/png;base64,AA".into()))
    );
    assert_eq!(
        store.get_favicon("nope.com", MISS_TTL_MS).unwrap(),
        Some(None)
    );
}

// One row per host: a host that gains an icon after a recorded miss keeps the
// icon, not both answers.
#[test]
fn favicon_put_replaces_the_previous_answer() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.put_favicon("ex.com", None).unwrap();
    store
        .put_favicon("ex.com", Some("data:image/png;base64,BB"))
        .unwrap();
    assert_eq!(
        store.get_favicon("ex.com", MISS_TTL_MS).unwrap(),
        Some(Some("data:image/png;base64,BB".into()))
    );

    store.put_favicon("ex.com", None).unwrap();
    assert_eq!(
        store.get_favicon("ex.com", MISS_TTL_MS).unwrap(),
        Some(None)
    );
}

// A miss is only as good as its TTL: past it the host is due for a retry, and
// the caller must see the same "nothing usable" it sees for a host never
// looked up. A hit has no TTL at all.
#[test]
fn a_miss_expires_but_a_hit_does_not() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.put_favicon("nope.com", None).unwrap();
    store
        .put_favicon("ex.com", Some("data:image/png;base64,CC"))
        .unwrap();

    // A TTL of zero makes every recorded miss stale, whatever the clock says.
    assert_eq!(store.get_favicon("nope.com", 0).unwrap(), None);
    assert_eq!(
        store.get_favicon("ex.com", 0).unwrap(),
        Some(Some("data:image/png;base64,CC".into()))
    );
}

// The cache is vault data, so it is inside the encrypted database and travels
// with it — a reopen finds it, and it is subject to the same key.
#[test]
fn favicons_survive_a_reopen() {
    let path = tmp_db();
    {
        let store = SqliteStore::open(&path, KEY).unwrap();
        store
            .put_favicon("ex.com", Some("data:image/png;base64,DD"))
            .unwrap();
    }
    let store = SqliteStore::open(&path, KEY).unwrap();
    assert_eq!(
        store.get_favicon("ex.com", MISS_TTL_MS).unwrap(),
        Some(Some("data:image/png;base64,DD".into()))
    );
}

// The favicons table arrived as a migration, so a vault created by an earlier
// build gains it on open rather than failing every lookup.
#[test]
fn the_favicons_table_is_migrated_into_an_existing_vault() {
    let path = tmp_db();
    {
        let store = SqliteStore::open(&path, KEY).unwrap();
        store.upsert(&rec("1", b"x")).unwrap();
        // Rewind to the schema the build before this table stamped, and drop
        // the table it did not have.
        store.drop_favicons_for_test().unwrap();
    }
    let store = SqliteStore::open(&path, KEY).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    store.put_favicon("ex.com", None).unwrap();
    assert_eq!(
        store.get_favicon("ex.com", MISS_TTL_MS).unwrap(),
        Some(None)
    );
}

#[test]
fn reopening_a_migrated_db_is_idempotent() {
    let path = tmp_db();
    {
        let store = SqliteStore::open(&path, KEY).unwrap();
        store.upsert(&rec("1", b"x")).unwrap();
    }
    // Opening an already-at-latest DB re-runs no migration and does not error.
    let store = SqliteStore::open(&path, KEY).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    // schema DDL versioning lives in user_version, not the app `meta` table.
    assert_eq!(store.meta_get("schema_version").unwrap(), None);
}

// save_entry then reveal_entry, exercised through the Argon2id key schedule:
// seal a plaintext entry under the payload key, upsert one row, unseal it back.
#[test]
fn save_then_reveal_round_trips_one_row() {
    let key = argon2_key("pw", &argon2_params(b"salt-a-01234567890123456789012345"));
    let cipher = key.payload_cipher();
    let store = SqliteStore::open(&tmp_db(), &*key.sqlcipher_key()).unwrap();

    let entry = sample_entry();
    let payload = cipher.seal(&entry).unwrap();
    let record = migrate::build_record(&entry, payload).unwrap();
    store.upsert(&record).unwrap();
    // Saving the same id again updates the single row (no whole-vault rewrite).
    store.upsert(&record).unwrap();

    let list = store.list().unwrap();
    assert_eq!(list.len(), 1);
    // The list carries only non-secret metadata — the password never appears.
    assert_eq!(list[0].url_host, "ex.com");
    let meta_json = serde_json::to_string(&(
        &list[0].title,
        &list[0].tags,
        &list[0].url_host,
        &list[0].kind,
    ))
    .unwrap();
    assert!(!meta_json.contains("s3cret"));

    // reveal_entry: unseal the stored payload with the session payload key.
    let rec = store.get("1").unwrap().unwrap();
    let revealed = cipher.unseal(&rec.id, &rec.payload).unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));
    assert_eq!(revealed.username.as_deref(), Some("alice"));
}

// Setup writes an Argon2id descriptor; unlock re-derives from that (serialized)
// descriptor and opens the same DB. A wrong password derives a different key and
// the open fails.
#[test]
fn argon2_descriptor_reproduces_key_and_opens() {
    let path = tmp_db();
    let params = argon2_params(b"salt-b-01234567890123456789012345");

    // setup: derive, open, save a sealed entry.
    let key = argon2_key("master-pw", &params);
    {
        let store = SqliteStore::open(&path, &*key.sqlcipher_key()).unwrap();
        let cipher = key.payload_cipher();
        let payload = cipher.seal(&sample_entry()).unwrap();
        store
            .upsert(&migrate::build_record(&sample_entry(), payload).unwrap())
            .unwrap();
    }

    // The persisted descriptor is Argon2id and round-trips through JSON (sidecar).
    let sidecar = params.to_json().unwrap();
    assert!(sidecar.contains("\"algo\":\"argon2id\""));
    let reloaded = KdfParams::from_json(&sidecar).unwrap();

    // unlock: re-derive from the reloaded descriptor and reopen.
    let key2 = argon2_key("master-pw", &reloaded);
    let store2 = SqliteStore::open(&path, &*key2.sqlcipher_key()).unwrap();
    let revealed = key2
        .payload_cipher()
        .unseal("1", &store2.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));

    // A wrong password derives a different SQLCipher key → open fails.
    let wrong = argon2_key("wrong-pw", &reloaded);
    assert!(SqliteStore::open(&path, &*wrong.sqlcipher_key()).is_err());
}

// Back-compat: a DB created with the legacy deterministic key (no sidecar) still
// opens via the legacy `VaultKey` fallback, and its payloads unseal.
#[test]
fn legacy_sidecarless_vault_opens() {
    let path = tmp_db();
    let key = VaultKey::legacy_from_password("master-pw");
    {
        let store = SqliteStore::open(&path, &*key.sqlcipher_key()).unwrap();
        let payload = key.payload_cipher().seal(&sample_entry()).unwrap();
        store
            .upsert(&migrate::build_record(&sample_entry(), payload).unwrap())
            .unwrap();
    }

    let key2 = VaultKey::legacy_from_password("master-pw");
    let store2 = SqliteStore::open(&path, &*key2.sqlcipher_key()).unwrap();
    let revealed = key2
        .payload_cipher()
        .unseal("1", &store2.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));
}

// change_master_password: re-seal every payload under the new payload key, rekey
// the DB to the new SQLCipher key, and write a fresh descriptor. The new
// descriptor + new password opens and reveals; the old key no longer opens.
#[test]
fn change_password_reseals_rekeys_and_new_descriptor_opens() {
    let path = tmp_db();
    let old = argon2_key(
        "old-pw",
        &argon2_params(b"salt-c-01234567890123456789012345"),
    );
    let store = SqliteStore::open(&path, &*old.sqlcipher_key()).unwrap();
    let payload = old.payload_cipher().seal(&sample_entry()).unwrap();
    store
        .upsert(&migrate::build_record(&sample_entry(), payload).unwrap())
        .unwrap();

    // Fresh Argon2id params (new salt) → new key; re-seal old→new, import, rekey.
    let new_params = argon2_params(b"salt-d-01234567890123456789012345");
    let new = argon2_key("new-pw", &new_params);
    let (old_cipher, new_cipher) = (old.payload_cipher(), new.payload_cipher());
    let resealed: Vec<Record> = store
        .export_for_sync()
        .unwrap()
        .into_iter()
        .map(|mut r| {
            let entry = old_cipher.unseal(&r.id, &r.payload).unwrap();
            r.payload = new_cipher.seal(&entry).unwrap();
            r
        })
        .collect();
    store.import(&resealed).unwrap();
    store.rekey(&*new.sqlcipher_key()).unwrap();
    drop(store);

    // The old key no longer opens; the new descriptor + password does.
    assert!(SqliteStore::open(&path, &*old.sqlcipher_key()).is_err());
    let reopened = argon2_key(
        "new-pw",
        &KdfParams::from_json(&new_params.to_json().unwrap()).unwrap(),
    );
    let store2 = SqliteStore::open(&path, &*reopened.sqlcipher_key()).unwrap();
    let revealed = reopened
        .payload_cipher()
        .unseal("1", &store2.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));
}

// Crash-recovery composition for change_master_password: snapshot the old-keyed
// DB, then re-key it (simulating a change that then "crashes"), then restore the
// snapshot back over the DB. The recovered DB must open under the OLD key with the
// original payload intact — proving the snapshot is a valid rollback point.
#[test]
fn snapshot_then_rekey_then_restore_recovers_old_key() {
    let path = tmp_db();
    let backup = path.with_file_name("vault.db.rekey-backup");
    let old = argon2_key(
        "old-pw",
        &argon2_params(b"salt-f-01234567890123456789012345"),
    );

    {
        let store = SqliteStore::open(&path, &*old.sqlcipher_key()).unwrap();
        let payload = old.payload_cipher().seal(&sample_entry()).unwrap();
        store
            .upsert(&migrate::build_record(&sample_entry(), payload).unwrap())
            .unwrap();
        // Recovery point, then the destructive rekey.
        store.snapshot_to(&backup, &*old.sqlcipher_key()).unwrap();
        let new = argon2_key(
            "new-pw",
            &argon2_params(b"salt-g-01234567890123456789012345"),
        );
        store.rekey(&*new.sqlcipher_key()).unwrap();
    } // connection closed

    // "Crash" rollback: drop stale WAL/SHM, copy the snapshot back over the DB.
    for suffix in ["-wal", "-shm"] {
        let mut p = path.clone().into_os_string();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
    std::fs::copy(&backup, &path).unwrap();

    // The restored DB opens with the OLD key and the payload is intact.
    let store = SqliteStore::open(&path, &*old.sqlcipher_key()).unwrap();
    let revealed = old
        .payload_cipher()
        .unseal("1", &store.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));
}

// import_swftx re-seals a `.swftx` encrypted under a *different* master password
// into the open store: expose under the source key, re-seal under the current
// payload key, upsert. Reveal with the current key must return the original
// plaintext, and importing twice must merge by id (no duplicate rows).
#[test]
fn import_swftx_reseals_across_passwords_and_upserts_by_id() {
    use crate::crypto::{hash_secret, Cryptor};
    use crate::models::VaultData;

    let src = Cryptor::new(&hash_secret("source-pw"));
    let plain: Vec<Entry> = ["1", "2"]
        .iter()
        .map(|id| {
            serde_json::from_value(serde_json::json!({
                "id": id, "type": "login", "title": format!("Site {id}"),
                "website": "https://example.com/login", "username": "alice",
                "password": "hunter2", "tags": ["work"]
            }))
            .unwrap()
        })
        .collect();

    // The source `.swftx`: entries obscured and sealed under the source password.
    let blob = src
        .encrypt_data(&VaultData {
            entries: plain.iter().map(|e| src.obscure(e).unwrap()).collect(),
        })
        .unwrap();

    // Decrypt with the source cryptor (as the command does), then re-seal each
    // entry under the current session's Argon2id payload key.
    let file: VaultData = src.decrypt_data(&blob).unwrap();
    let key = argon2_key(
        "current-pw",
        &argon2_params(b"salt-e-01234567890123456789012345"),
    );
    let cipher = key.payload_cipher();
    let store = SqliteStore::open(&tmp_db(), &*key.sqlcipher_key()).unwrap();
    for r in migrate::reseal_swftx(&file.entries, &src, &cipher).unwrap() {
        store.upsert(&r).unwrap();
    }
    // Re-importing the same file merges by id — the count stays at 2.
    for r in migrate::reseal_swftx(&file.entries, &src, &cipher).unwrap() {
        store.upsert(&r).unwrap();
    }
    assert_eq!(store.list().unwrap().len(), 2);

    // Reveal with the current payload key returns the original plaintext secrets.
    let revealed = cipher
        .unseal("1", &store.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("hunter2"));
    assert_eq!(revealed.username.as_deref(), Some("alice"));
}

// A `.swftx` backup has to carry the star. It is a column, never a payload
// field, so the export re-attaches it (`export_entry`) and the import threads it
// back through `build_record` — miss either half and a backup silently unstars
// the whole vault.
#[test]
fn swftx_round_trip_preserves_a_starred_record() {
    use crate::crypto::{hash_secret, Cryptor};
    use crate::models::VaultData;

    let key = argon2_key(
        "current-pw",
        &argon2_params(b"salt-h-01234567890123456789012345"),
    );
    let cipher = key.payload_cipher();
    let source = SqliteStore::open(&tmp_db(), &*key.sqlcipher_key()).unwrap();
    let entry = sample_entry();
    source
        .upsert(&migrate::build_record(&entry, cipher.seal(&entry).unwrap()).unwrap())
        .unwrap();
    source.set_favorite("1", true).unwrap();

    // export_vault: project every live row back to a plaintext entry, obscured
    // and sealed under the backup password.
    let out = Cryptor::new(&hash_secret("backup-pw"));
    let blob = out
        .encrypt_data(&VaultData {
            entries: source
                .export_for_sync()
                .unwrap()
                .iter()
                .map(|r| migrate::export_entry(r, &cipher, &out).unwrap())
                .collect(),
        })
        .unwrap();

    // import_backup: decrypt the file and re-seal it into a fresh vault.
    let file: VaultData = out.decrypt_data(&blob).unwrap();
    let restored = SqliteStore::open(&tmp_db(), &*key.sqlcipher_key()).unwrap();
    restored
        .import(&migrate::reseal_swftx(&file.entries, &out, &cipher).unwrap())
        .unwrap();

    let list = restored.list().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].favorite);
    // The secrets still round-trip, so the star did not ride in on a broken export.
    let revealed = cipher
        .unseal("1", &restored.get("1").unwrap().unwrap().payload)
        .unwrap();
    assert_eq!(revealed.password.as_deref(), Some("s3cret"));
}

// A wrong source password fails the file decrypt before the store is touched.
#[test]
fn import_swftx_wrong_source_password_errors() {
    use crate::crypto::{hash_secret, Cryptor};
    use crate::models::VaultData;

    let src = Cryptor::new(&hash_secret("source-pw"));
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "id": "1", "type": "login", "title": "Site", "password": "hunter2"
    }))
    .unwrap();
    let blob = src
        .encrypt_data(&VaultData {
            entries: vec![src.obscure(&entry).unwrap()],
        })
        .unwrap();

    let wrong = Cryptor::new(&hash_secret("not-the-password"));
    assert!(wrong.decrypt_data::<VaultData>(&blob).is_err());
}

// ---- sync merge primitives ------------------------------------------------

// The length prefixes exist so that shifting a byte across a field boundary is
// a different preimage; without them ("ab","c") and ("a","bc") would collide,
// and the merge tie-break would call two different records equal.
#[test]
fn record_hash_separates_adjacent_fields() {
    let mut a = rec("1", b"x");
    a.title = "ab".into();
    a.tags = "c".into();
    let mut b = a.clone();
    b.title = "a".into();
    b.tags = "bc".into();

    assert_ne!(record_hash(&a), record_hash(&b));
    // Same content, same hash — the id is deliberately not part of it.
    assert_eq!(record_hash(&a), record_hash(&a.clone()));
    assert_eq!(
        record_hash(&a),
        record_hash(&Record {
            id: "2".into(),
            ..a
        })
    );
}

#[test]
fn merge_newer_incoming_wins() {
    let store = seeded(&[stamped("1", b"old", 1000)]);
    assert_eq!(
        store.merge_records(&[stamped("1", b"new", 2000)]).unwrap(),
        1
    );

    let got = row(&store, "1");
    assert_eq!(got.payload, b"new");
    assert_eq!(got.updated_at, 2000);
}

#[test]
fn merge_older_incoming_is_a_no_op() {
    let store = seeded(&[stamped("1", b"local", 2000)]);
    let before = row(&store, "1");

    assert_eq!(
        store
            .merge_records(&[stamped("1", b"stale", 1000)])
            .unwrap(),
        0
    );
    // Byte-identical, timestamps included: the loser must not touch the row.
    assert_eq!(row(&store, "1"), before);
}

#[test]
fn merge_inserts_unknown_record_verbatim() {
    let store = seeded(&[]);
    let mut incoming = stamped("1", b"fresh", 222);
    incoming.created_at = 111;

    assert_eq!(store.merge_records(&[incoming]).unwrap(), 1);

    let got = row(&store, "1");
    // created_at is carried over, not re-stamped to now as upsert would.
    assert_eq!((got.created_at, got.updated_at), (111, 222));
    assert_eq!(got.payload, b"fresh");
}

#[test]
fn merge_newer_tombstone_beats_older_edit() {
    let store = seeded(&[stamped("1", b"live", 1000)]);
    assert_eq!(store.merge_records(&[tombstone("1", 2000)]).unwrap(), 1);

    assert_eq!(store.get("1").unwrap(), None);
    assert_eq!(row(&store, "1").deleted_at, Some(2000));
}

// The mirror case: an edit made after the delete resurrects the entry. That is
// LWW working, not a bug — the user's later intent wins.
#[test]
fn merge_newer_edit_beats_older_tombstone() {
    let store = seeded(&[tombstone("1", 1000)]);
    assert_eq!(
        store
            .merge_records(&[stamped("1", b"revived", 2000)])
            .unwrap(),
        1
    );

    let got = store.get("1").unwrap().unwrap();
    assert_eq!(got.payload, b"revived");
    assert_eq!(got.deleted_at, None);
}

#[test]
fn merge_tie_on_identical_content_writes_nothing() {
    let store = seeded(&[stamped("1", b"same", 1000)]);
    let mine = store.export_for_sync().unwrap();
    assert_eq!(store.merge_records(&mine).unwrap(), 0);
}

// The convergence property. On an exact timestamp tie the two sides hold
// different content, so *someone* must yield — and both must pick the same
// side, or they diverge permanently and push at each other forever.
#[test]
fn merge_tie_picks_the_same_winner_on_both_sides() {
    let a = seeded(&[stamped("1", b"aaa", 5000)]);
    let b = seeded(&[stamped("1", b"bbb", 5000)]);
    let (from_a, from_b) = (a.export_for_sync().unwrap(), b.export_for_sync().unwrap());

    a.merge_records(&from_b).unwrap();
    b.merge_records(&from_a).unwrap();

    assert_eq!(a.state_digest().unwrap(), b.state_digest().unwrap());
    // Exactly one of the two payloads survived on both sides.
    let payload = row(&a, "1").payload;
    assert_eq!(payload, row(&b, "1").payload);
    assert!(payload == b"aaa" || payload == b"bbb");
}

// A backfill stamps a derived column without moving `updated_at`, so the
// stamped row and a peer's pre-column copy tie on time. The stamped side must
// win — in both directions — or the hash can pick the NULL side every time.
#[test]
fn merge_tie_prefers_the_row_whose_derived_columns_are_stamped() {
    // A payload whose NULL copy beats the stamped one on hash alone.
    let (unstamped, backfilled) = (0u8..)
        .map(|n| {
            let unstamped = stamped("1", &[n], 5000);
            let backfilled = Record {
                username: Some("alice".into()),
                ..unstamped.clone()
            };
            (unstamped, backfilled)
        })
        .find(|(u, b)| record_hash(u) > record_hash(b))
        .unwrap();

    let a = seeded(std::slice::from_ref(&backfilled));
    assert_eq!(
        a.merge_records(std::slice::from_ref(&unstamped)).unwrap(),
        0
    );
    assert_eq!(row(&a, "1").username.as_deref(), Some("alice"));

    let b = seeded(std::slice::from_ref(&unstamped));
    assert_eq!(
        b.merge_records(std::slice::from_ref(&backfilled)).unwrap(),
        1
    );
    assert_eq!(row(&b, "1").username.as_deref(), Some("alice"));
}

// Two real edits of one login in the same millisecond — one drops its last
// passkey — both carry a stamped username, so the rank ties and the hash
// decides, exactly as before. Keeping a passkey must not be a systematic win.
#[test]
fn merge_tie_between_two_stamped_edits_is_still_decided_by_hash() {
    let edit = |payload: &[u8], has_passkey: bool| Record {
        username: Some("alice".into()),
        has_passkey,
        ..stamped("1", payload, 5000)
    };
    // A pair where the passkey-keeping side loses on hash.
    let (keeps, drops) = (0u8..)
        .map(|n| (edit(&[n], true), edit(&[n, n], false)))
        .find(|(keeps, drops)| record_hash(drops) > record_hash(keeps))
        .unwrap();

    let store = seeded(std::slice::from_ref(&keeps));
    assert_eq!(
        store.merge_records(std::slice::from_ref(&drops)).unwrap(),
        1
    );
    assert!(!row(&store, "1").has_passkey);
}

#[test]
fn merge_is_idempotent() {
    let source = seeded(&[stamped("1", b"x", 1000), stamped("2", b"y", 2000)]);
    let target = seeded(&[stamped("1", b"older", 500)]);
    let export = source.export_for_sync().unwrap();

    assert_eq!(target.merge_records(&export).unwrap(), 2);
    let digest = target.state_digest().unwrap();

    // Replaying the same batch is a no-op — every record now ties itself.
    assert_eq!(target.merge_records(&export).unwrap(), 0);
    assert_eq!(target.state_digest().unwrap(), digest);
}

#[test]
fn merge_is_commutative_across_divergent_stores() {
    let a = seeded(&[
        stamped("1", b"a1", 100), // b wins
        stamped("2", b"a2", 300), // a wins
        stamped("3", b"a3", 100), // only in a
    ]);
    let b = seeded(&[
        stamped("1", b"b1", 200),
        stamped("2", b"b2", 200),
        stamped("4", b"b4", 100), // only in b
    ]);
    let (from_a, from_b) = (a.export_for_sync().unwrap(), b.export_for_sync().unwrap());

    a.merge_records(&from_b).unwrap();
    b.merge_records(&from_a).unwrap();

    assert_eq!(a.state_digest().unwrap(), b.state_digest().unwrap());
    assert_eq!(row(&a, "1").payload, b"b1");
    assert_eq!(row(&a, "2").payload, b"a2");
    // Records held by only one side are additions, never conflicts.
    assert_eq!(a.export_for_sync().unwrap().len(), 4);
}

#[test]
fn merge_preserves_timestamps_end_to_end() {
    let mut original = stamped("1", b"x", 222);
    original.created_at = 111;
    let a = seeded(&[original]);
    let b = seeded(&[]);

    b.merge_records(&a.export_for_sync().unwrap()).unwrap();

    assert_eq!(row(&b, "1"), row(&a, "1"));
}

#[test]
fn state_digest_matches_exactly_on_equal_state() {
    let rows = [stamped("1", b"x", 100), stamped("2", b"y", 200)];
    let a = seeded(&rows);
    let b = seeded(&rows);
    assert_eq!(a.state_digest().unwrap(), b.state_digest().unwrap());

    // Any content change moves the digest.
    b.merge_records(&[stamped("2", b"changed", 300)]).unwrap();
    assert_ne!(a.state_digest().unwrap(), b.state_digest().unwrap());
}

// Tombstones are state: a vault that has seen a delete is not the same vault as
// one that never held the row, and the digest has to say so or the delete never
// propagates.
#[test]
fn state_digest_counts_tombstones() {
    let a = seeded(&[stamped("1", b"x", 100)]);
    let b = seeded(&[stamped("1", b"x", 100), tombstone("2", 100)]);
    assert_ne!(a.state_digest().unwrap(), b.state_digest().unwrap());

    b.purge_tombstones_before(200).unwrap();
    assert_eq!(a.state_digest().unwrap(), b.state_digest().unwrap());
}

#[test]
fn purge_tombstones_before_spares_live_and_recent_rows() {
    let store = seeded(&[
        stamped("live", b"x", 1000),
        tombstone("old", 1000),
        tombstone("recent", 9000),
    ]);

    assert_eq!(store.purge_tombstones_before(5000).unwrap(), 1);

    let mut ids: Vec<String> = store
        .export_for_sync()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["live", "recent"]);
}

// Restore scrubs the source device's sync bookkeeping by prefix. The match must
// be a literal prefix — `_` is a LIKE wildcard, so a naive `LIKE 'sync_%'` would
// also eat `syncopation`.
// The database is owner-only from the moment it exists — not chmod'ed after the
// open and the migrations, when the file (and its WAL) had already been on disk
// under the umask for the whole open. The `-wal` and `-shm` files SQLite adds
// take the main file's mode, so a write after the open has to leave them 0600
// as well; `snapshot_to` creates its destination the same way.
#[cfg(unix)]
#[test]
fn the_database_and_its_wal_are_owner_only_from_creation() {
    use std::os::unix::fs::PermissionsExt;
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let sibling = |path: &std::path::Path, suffix: &str| {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        PathBuf::from(s)
    };

    let path = tmp_db();
    let store = SqliteStore::open(&path, KEY).unwrap();
    store.upsert(&rec("a", b"payload")).unwrap();

    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&sibling(&path, "-wal")), 0o600);
    assert_eq!(mode(&sibling(&path, "-shm")), 0o600);

    let snapshot = path.with_file_name("snapshot.db");
    store.snapshot_to(&snapshot, KEY).unwrap();
    assert_eq!(mode(&snapshot), 0o600);

    // A database that already exists with a looser mode is tightened before it
    // is opened, so a vault written by an older build catches up on first open.
    drop(store);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let _reopened = SqliteStore::open(&path, KEY).unwrap();
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn meta_delete_prefix_removes_only_the_namespace() {
    let store = SqliteStore::open(&tmp_db(), KEY).unwrap();
    store.meta_set("sync_last_digest", "deadbeef").unwrap();
    store.meta_set("sync_remote_rev", "r1").unwrap();
    store.meta_set("syncopation", "keep").unwrap();
    store.meta_set("kdf", "argon2id").unwrap();

    assert_eq!(store.meta_delete_prefix(SYNC_META_PREFIX).unwrap(), 2);

    assert_eq!(store.meta_get("sync_last_digest").unwrap(), None);
    assert_eq!(store.meta_get("sync_remote_rev").unwrap(), None);
    assert_eq!(
        store.meta_get("syncopation").unwrap().as_deref(),
        Some("keep")
    );
    assert_eq!(store.meta_get("kdf").unwrap().as_deref(), Some("argon2id"));
}
