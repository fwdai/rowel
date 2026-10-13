# Multi-provider sync: core + adapters

Status: proposal. Nothing here is implemented yet.

Goal: keep one sync engine and one set of rules about vault identity, naming
and conflict resolution, and plug cloud providers in underneath. Google Drive
is the first adapter (it exists today, fused into the core). Dropbox is the
second. A third should be a new directory under `sync/providers/` plus one
enum variant, and nothing else. iCloud is the third (§9), and it was almost
that: the storage half fit as planned, the account half needed one new
concept — a provider with no sign-in of its own.

## 1. What the code looks like today

The split is already half there. `sync/engine.rs` is pure over two traits
(`Remote`, `LocalVault`), `pack.rs`, `restore.rs` and `layout.rs` are pure, and
the identity decisions (`plan_vault_id`, `remote_only`, `pack_missing`,
`may_create_pack`, `may_adopt`, `restorable_packs`) are pure functions with
tests. Every one of those stays as it is.

What is Drive-shaped, and where:

| Concern | Today | Drive-specific part |
|---|---|---|
| REST transport | `sync/drive.rs` | all of it |
| OAuth | `sync/auth.rs` | endpoints, scope, client-id lookup, iOS reversed-client-id scheme. The PKCE helpers, `parse_redirect`, the loopback listener and the `TokenFile` generation guard are generic. |
| Token file | `storage::GDRIVE_FILE` = `auth/gdrive.swftx` | the name |
| `Remote` impl | `sync/mod.rs::DriveRemote` | folder-id caching, plus the deletion-marker semantics, which are *not* Drive-specific but live here |
| Vault-id resolution | `sync/mod.rs::resolve_vault_id` | calls `setup::find_packs(client, token)` with a Google token |
| Delete everywhere | `sync/mod.rs::delete_pack` | Drive calls, marker-first ordering (generic rule) |
| Onboarding probe / restore / autojoin | `commands/setup.rs`, `commands/autojoin.rs`, `commands/workspace.rs` | `setup::find_packs`, `find_pack_by_id`, `download_pack` take a Google token; `PendingDrive` holds `sync::Tokens` |
| Shares | `share/remote.rs` | `DriveShareRemote` (appProperties, anyone-with-link), `DrivePublicFetch` (Google API key); `share/mod.rs` is generic over the traits |
| Errors / copy | `rowel-core::error`, messages in `sync/mod.rs`, `engine.rs`, `commands/workspace.rs` | `DriveNotConnected`; sentences naming Google |
| Events | `setup:drive:pending/probed/error` | the names |
| Frontend | `store/drive.ts`, `store/app.ts::setupDrive`, `api/setup.ts`, `Start/Drive`, `Settings/Sync/DriveCard`, `Workspaces/RestoreFromDrive`, `GoogleDriveMark` | naming, and "Google Drive" baked into ~20 i18n strings |

## 2. Target shape

Domains, and which side of the line they sit on:

```
shared core                              per-provider adapter
-----------                              --------------------
naming        sync/layout.rs             (none: adapters use the names as given)
identity      plan_vault_id, may_adopt,  (none)
              remote_only, store::identity
resolution    sync/engine.rs             (none)
packaging     sync/pack.rs, restore.rs   (none)
account       sync/account.rs            OAuthConfig: endpoints, scope, client id,
              (OAuth2+PKCE flow, token      mobile redirect scheme
               file, refresh, generation)
storage       sync/store.rs              VaultStore impl over the provider's API
              (trait + PackRemote +
               deletion-marker rules)
sharing       share/mod.rs (verbs)       ShareRemote + PublicFetch impls
```

### 2.1 `Provider`

```rust
// sync/provider.rs
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider { GoogleDrive, Dropbox }
```

It names the provider in the token file, in every connect command, in the
`sync:status` payload and in the probe's `PackInfo`. Adding a provider is
adding a variant; the compiler then points at every `match`.

### 2.2 Account: one OAuth flow, data per provider

Both Google and Dropbox are OAuth2 authorization-code + PKCE with refresh
tokens, so the flow in `auth.rs` stays and becomes generic over a config:

```rust
pub struct OAuthConfig {
    pub provider: Provider,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    /// Query pairs the consent URL needs beyond the standard ones
    /// (Google: scope, access_type=offline, prompt=consent;
    ///  Dropbox: token_access_type=offline, scope).
    pub consent_params: &'static [(&'static str, &'static str)],
    pub client_id: fn() -> Option<String>,      // env/option_env lookup
    pub client_secret: fn() -> Option<String>,  // desktop only; None for Dropbox
    pub mobile_scheme: fn(&AppHandle) -> Result<String>,
}
```

`Tokens` gains `provider: Provider`. `auth.rs` becomes `account.rs`; the
functions keep their signatures but take the config from the tokens' provider.
`TokenFile` (the generation-guarded read/write/reseal/disconnect) is untouched.

Token file on disk: `auth/<provider>.swftx`, at most one per workspace. The
existing `auth/gdrive.swftx` is Google's, so nothing migrates, and the
provider of a *locked* workspace is readable from the file name alone (the
lock screen and `WorkspaceStatus.synced` already read only the file's
presence). `sync_configured_in(dir)` becomes "any provider's file is present"
(as implemented: `storage::sync_provider_in(dir)`, which also says whose).

Refresh differences the config absorbs: Google may rotate the refresh token
(already handled: keep the old one when none comes back); Dropbox never
rotates, access tokens live ~4 h.

### 2.3 Storage: the `VaultStore` trait

The one thing an adapter has to provide. Names and bytes in, names and bytes
out; every rule about *which* name is the core's.

```rust
// sync/store.rs
pub struct RemoteEntry {
    /// Opaque, provider-specific: a Drive file id, a Dropbox `id:` value.
    /// Round-trips through the frontend as `PackInfo.id`.
    pub handle: String,
    pub name: String,
    pub size: Option<u64>,
    /// RFC 3339 UTC; the probe orders by it.
    pub modified: String,
    /// Drive `headRevisionId`, Dropbox `rev`. None = unknown.
    pub revision: Option<String>,
}

pub trait VaultStore {
    fn provider(&self) -> Provider;
    /// Everything directly inside `<root>/Vaults/`. A missing folder is `[]`.
    /// Must be complete: follow pagination to the last page.
    async fn list_vaults(&self) -> Result<Vec<RemoteEntry>>;
    /// The entry named `name` in `Vaults/`, or `None`.
    async fn stat(&self, name: &str) -> Result<Option<RemoteEntry>>;
    /// Contents by handle, refusing anything over `max_bytes`. No revision:
    /// the core takes it from the same `stat` that located the entry, so
    /// `fetch` and `head_revision` cannot drift apart.
    async fn read(&self, handle: &str, max_bytes: usize) -> Result<Vec<u8>>;
    /// A new entry named `name` in `Vaults/`, creating folders as needed.
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<Option<String>>;
    /// Overwrite an existing entry by handle. Kept apart from `create` so a
    /// "delete everywhere" landing between the core's existence check and the
    /// write fails (no such handle) instead of quietly re-creating the pack.
    async fn update(&self, handle: &str, bytes: &[u8]) -> Result<Option<String>>;
    /// Delete by handle. Already gone counts as deleted.
    async fn delete(&self, handle: &str) -> Result<()>;
}
```

(As implemented in PR 1. The first draft had a single name-based `write` and
a `read` that returned the revision; both were changed for the reasons in the
comments above.)

Runtime dispatch is an enum, not `Box<dyn>`: `async fn` traits are not
object-safe, and there will only ever be a handful of providers.

```rust
pub enum AnyStore { GoogleDrive(gdrive::Store), Dropbox(dropbox::Store) }
impl VaultStore for AnyStore { /* match and forward, one line per method */ }
```

What moves *out* of `DriveRemote` into the core, written once:

- `PackRemote<S: VaultStore> { store, vault_id }` implements
  `engine::Remote`. It holds the deletion-marker semantics (`pack_missing`,
  `may_create_pack`), the `head_revision` "degrade identically to fetch" rule
  and the `MAX_PACK_BYTES` cap.
- `resolve_vault_id`, `delete_pack` (marker first, then the pack),
  `find_packs`/`restorable_packs`, `find_pack_by_id`, `download_pack` all
  become generic over `&impl VaultStore`.

Engine tests gain nothing new; `PackRemote` gets its own tests against a
`FakeStore` (an in-memory map), which is also what later adapters are checked
against for contract conformance.

What stays in the Drive adapter: `q`-escaping, pagination, multipart upload,
`appProperties`, and **oldest-wins** among same-name siblings. That last rule
is Drive's alone (Dropbox paths are unique) and is exactly the kind of thing
the trait boundary should hide.

### 2.4 Credentials reach the store two ways

Today `DriveRemote` reads the sealed token file per call (refresh written back
under the generation guard), while onboarding, autojoin and `sync_adopt_pending`
hold tokens in memory and refresh them in place. Both stay, as one enum the
adapter is built with:

```rust
pub enum Auth {
    /// The active workspace's sealed file; refreshes are written back if the
    /// connection generation still matches.
    Sealed { app: AppHandle, cryptor: Cryptor },
    /// Consent's in-memory result; refreshed in place, handed back to the
    /// caller afterwards (`into_tokens`).
    Pending(Mutex<Tokens>),
}
```

`sync::store_for(auth) -> AnyStore` picks the adapter from
`tokens.provider`. Callers that passed `&mut Tokens` take them back out when
done; `replace_pending_tokens` keeps working unchanged.

### 2.5 Sharing

`share/mod.rs` is already generic. `DriveShareRemote` and `DrivePublicFetch`
move under `providers/gdrive/share.rs`. `share::remote_for(app, cryptor)`
returns `Option<AnyShareRemote>`: `None` for a provider without share support,
and the sweep at the end of a run becomes a no-op there.

Dropbox *can* support shares later (the `file_properties` API is the
`appProperties` analogue; `sharing/create_shared_link_with_settings` makes a
link the recipient fetches with `dl=1` and no account), but the recipient
needs to know which provider to fetch from, so the link format has to grow a
provider tag (`rowel://share#v2.<provider>.<ref>.<key>`). That is its own PR;
until then shares are Drive-only and the Settings row is hidden for other
providers.

### 2.6 Commands and events

Every connect command takes `provider: Provider`:
`setup_connect`, `workspace_connect`, `sync_connect`. The pending-tokens
machinery (`PendingDrive` → `PendingAccount`, the attempt counter, `may_adopt`,
`still_the_same_workspace`) does not change. `sync:status` gains
`provider: Provider | null`. Events become `setup:account:*`.

Error copy that names Google (`account_has_vaults_error`,
`foreign_vault_error`, `disconnected_mid_restore_error`,
`unreadable_tokens_error`) takes the provider's display name;
`Error::DriveNotConnected` becomes `AccountNotConnected` (frontend kind
`accountNotConnected`).

## 3. Dropbox adapter

API facts the adapter is built on (checked against the Dropbox docs on
2026-10-12; sources under "Verified against Dropbox docs" below):

- App type: **App folder** access. Every path is relative to
  `/Apps/<app name>/`, so `layout::ROOT_FOLDER` is implied and the adapter
  works under `/Vaults/`. Scopes: `files.metadata.read`,
  `files.content.read`, `files.content.write` (`sharing.write` later).
- OAuth: authorize `https://www.dropbox.com/oauth2/authorize` with
  `token_access_type=offline`, `code_challenge_method=S256`; token
  `https://api.dropboxapi.com/oauth2/token`. PKCE, no client secret. Env:
  `DROPBOX_APP_KEY` (add to `.env.example` and to Fletch's env config).
  Desktop redirect: the exact URI, port included, must be registered in the
  App Console; there are no wildcard ports. So the flow cannot bind port 0
  like Google's: it tries a fixed list (`oauth::LOOPBACK_PORTS` = 53682,
  53683, 53684) and uses the first that binds, redirecting to
  `http://127.0.0.1:<port>/auth/callback`. All three go into the console.
  Mobile redirect: `db-<APP_KEY>://2/token` (what SwiftyDropbox sends on both
  the authorize call and the code exchange; cancel lands on
  `db-<APP_KEY>://2/cancel`). The scheme `db-<APP_KEY>` goes into
  `tauri.ios.conf.json`'s deep-link schemes beside the Google one, and
  `redirect_matches` picks by scheme.
- Listing: `files/list_folder {path:"/Vaults"}` then `list_folder/continue`
  while `has_more`. Entries carry `.tag`, `name`, `id`, `rev`, `size`,
  `server_modified`. A 409 `path/not_found` is an empty folder.
- Stat: `files/get_metadata {path:"/Vaults/<name>"}`; 409 `path/not_found`
  is `None`. Paths are case-insensitive and unique, so no oldest-wins.
- Read: `files/download` on `content.dropboxapi.com` with `Dropbox-API-Arg`;
  the `rev` comes back in the `Dropbox-API-Result` header. Stream with the
  existing `read_capped` loop.
- Write: `files/upload` with `mode: overwrite`, `mute: true`; returns `rev`.
  Parent folders are created implicitly. Single-call uploads are capped at
  150 MiB; `MAX_PACK_BYTES` is 256 MiB. First cut: refuse past 150 MiB with a
  clear error; upload sessions (`upload_session/start|append_v2|finish`) are
  a follow-up.
- Delete: `files/delete_v2`; not found (reported as `path_lookup/not_found`
  on this route, not `path/not_found`) is success.
- Errors: 409 with `error_summary` (match by tag prefix: the suffix is a
  random run of dots); 429 with `Retry-After` (surface as a run failure, the
  scheduler retries).
- Hardening available later for free: `mode: {".tag":"update","update":rev}`
  turns the engine's optimistic head check into a server-side compare-and-swap.

Module layout:

```
src-tauri/src/sync/
  mod.rs            run(), resolve_vault_id, delete_pack, publish_remote_vaults — generic
  provider.rs       enum Provider, display names, OAuthConfig lookup
  account.rs        (was auth.rs) OAuth2+PKCE flow, TokenFile, refresh
  store.rs          VaultStore, RemoteEntry, AnyStore, PackRemote, FakeStore (test)
  setup.rs          PackInfo, restorable_packs, find_packs/download — generic
  engine.rs pack.rs restore.rs layout.rs   unchanged
  providers/
    gdrive/ api.rs (today's drive.rs)  mod.rs (Store + OAuthConfig)  share.rs
    dropbox/ api.rs                    mod.rs (Store + OAuthConfig)
```

### Verified against Dropbox docs

Checked 2026-10-12. The client is `sync/providers/dropbox/{api,oauth}.rs`.

1. **Desktop loopback redirect: fixed, registered ports.** `redirect_uri`
   must exactly match one registered in the App Console, port included
   ("even localhost must be listed"); Dropbox staff confirmed there is no
   wildcard or variable-port loopback rule. Hence `LOOPBACK_PORTS`
   (53682-53684) with `http://127.0.0.1:<port>/auth/callback`, each
   registered in the console.
   <https://docs.dropboxapi.com/dropbox-api/docs/get-started/authorization.md>,
   <https://docs.dropboxapi.com/dropbox-api/docs/oauth>,
   <https://community.dropbox.com/discussions/101000014/redirect-uri-with-variable-loopback-port/197621>
2. **Mobile redirect: `db-<APP_KEY>://2/token`.** SwiftyDropbox sends it as
   `redirect_uri` on authorize and on the token exchange; cancel is
   `db-<APP_KEY>://2/cancel`. Only the OS-level scheme is registered. That
   the server accepts it without an App Console entry is inferred from the
   SDK working, not documented: confirm on the first device run.
   <https://github.com/dropbox/SwiftyDropbox> (`OAuthImpl.swift`, README)
3. **Upload limit: 150 MiB.** files/upload: "Do not use this to upload a file
   larger than 150 MiB" (the stone spec says "150 MB"; the client uses the
   binary reading, `150 * 1024 * 1024`).
   <https://docs.dropboxapi.com/dropbox-api/api-reference/user-endpoints/files/upload.md>
4. **Not-found `error_summary`.** A 409 body carries `error`,
   `error_summary` and optionally `user_message`. `error_summary` is the tag
   path joined by `/`, then a random number of trailing dots, so it is
   matched by prefix: `path/not_found/` for `list_folder`, `get_metadata`
   and `download`; `path_lookup/not_found/` for `delete_v2`. 429 may carry
   `Retry-After` (seconds); 401 means re-auth; 400 is not retryable.
   <https://docs.dropboxapi.com/dropbox-api/docs/error-handling.md>
5. **`id:` in place of a path.** `download`, `get_metadata`, `list_folder`,
   `delete_v2` and `upload` all accept `id:...` in `path`. IDs are
   case-sensitive; the app folder's root lists as `""`, not `"/"`.
   <https://github.com/dropbox/dropbox-api-spec/blob/master/files.stone>

Also confirmed: `Dropbox-API-Result` on `files/download` is the file's
`FileMetadata` JSON (`rev` always present). `Dropbox-API-Arg` must be
header-safe, with U+007F and everything non-ASCII written as `\uXXXX` UTF-16
escapes (surrogate pairs past the BMP); the client escapes it
(<https://docs.dropboxapi.com/dropbox-api/docs/technical-reference/json-encoding.md>).
OAuth: authorize `https://www.dropbox.com/oauth2/authorize`, token
`https://api.dropboxapi.com/oauth2/token`, `token_access_type=offline` for a
refresh token, space-separated `scope`, PKCE S256, no client secret on the
exchange or the refresh (`grant_type=refresh_token`, `refresh_token`,
`client_id`).

## 4. Behaviour that falls out without new code

- **Moving a vault between providers**: disconnect, connect the other. The
  vault keeps its id; `plan_vault_id` answers `Use(id)`; the first push creates
  `Vaults/<id>.rowel` in the new account. If the new account already holds
  *other* vaults, the existing `Refuse`/`VaultNotInAccount` rule applies and
  the user restores instead. No special casing.
- **Mixed devices**: one account per workspace, provider stored with it;
  devices on the same vault must use the same account, as today.
- **One vault on two providers at once**: not supported (one token file per
  workspace). Say so in the UI rather than build it.

## 5. Invariants every adapter must keep

Stated here because they are what the Drive code's comments defend:

1. Listings are complete (follow pagination).
2. `stat`/`read`/`head_revision` never answer from a cache; folder ids may be
   cached per instance.
3. Downloads are capped before buffering (`read_capped`).
4. No lock is held across a network call; adapters are driven with
   `block_on` only on blocking-pool threads (`commands::detached`/`blocking`).
5. Marker before pack on delete; the core does it, the adapter just writes.
6. A revision token is "unknown" rather than fabricated when the API has none.

## 6. PR plan

Dependency order: 1 → 2 → 3 → 4.

### PR 1 — Extract the core from Drive (no behaviour change)

Scope: `Provider`, `OAuthConfig`, `account.rs`, `VaultStore`/`AnyStore`/
`PackRemote`, `Auth`, token file per provider name, Drive moved under
`providers/gdrive/`, every core function generic. Commands keep their names
and hardcode `Provider::GoogleDrive`; frontend untouched.

Files: `sync/{mod,auth→account,drive→providers/gdrive/api,setup}.rs`, new
`sync/{provider,store}.rs`, `share/{mod,remote}.rs`, `storage.rs` (token file
name helpers), `state.rs` (`PendingDrive` → `PendingAccount`),
`commands/{sync,setup,autojoin,workspace,share,auth}.rs` (call sites only).

Acceptance: `cargo test` green with existing tests unchanged in intent; new
`PackRemote` tests over `FakeStore` cover marker semantics; manual pass on
Drive: onboarding restore, connect from Settings, two-device sync, delete
everywhere, autojoin, share create/revoke/sweep.

### PR 2 — Dropbox adapter (backend only)

Scope: `providers/dropbox/{api,mod}.rs`, `Provider::Dropbox` arms,
`DROPBOX_APP_KEY`, iOS scheme entry, commands accept `provider` with
`#[serde(default)]` = GoogleDrive so the frontend still works.

Files: new `sync/providers/dropbox/*`, `sync/provider.rs`, `commands/*.rs`
signatures, `.env.example`, `tauri.ios.conf.json`, `docs/releasing*.md` (env).

Acceptance: unit tests for listing/metadata/error parsing and the
`Dropbox-API-Result` header; `FakeStore` contract tests reused; manual
two-device Dropbox run of the same checklist as PR 1 minus shares; a Drive
workspace and a Dropbox workspace side by side on one device.

As implemented (PR 2b, the wiring):

- Connect commands take `provider: Option<Provider>`: a Tauri command
  argument can only be omitted when it is an `Option`, so `None` stands in for
  `#[serde(default)]` and means Google Drive.
- One token file per workspace: a connect's write removes every other
  provider's file first, under the generation guard, and moves the generation
  only when it actually removed one (a refresh of the displaced connection
  must not write it back). Reconnecting the same provider moves nothing, as
  before: autojoin and a workspace restore read a moved generation as "the
  account was disconnected".
- iOS: `db-<DROPBOX_APP_KEY>` is injected by the release build beside Google's
  scheme, never committed (a placeholder is an illegal scheme); the app reads
  the key back off the scheme. `db-<key>://2/cancel` ends a pending consent as
  denied.
- `update` overwrites by `id:` (no `update <rev>` mode in `api.rs` yet);
  `create` uploads in `overwrite` mode, so two first pushes racing leave the
  later one, which the other device's next full-state merge absorbs.

### PR 3 — Frontend provider choice and neutral naming

Scope: `setup:drive:*` → `setup:account:*`; `setupDrive` → `setupAccount`
with `provider`; `SetupDriveFile` → `RemotePack`; API wrappers take
`provider`; `SyncStatus.provider`; Welcome offers "Continue with Google Drive"
and "Continue with Dropbox"; Settings › Sync shows a picker when disconnected
and the connected provider's mark otherwise; Workspaces restore flow likewise;
`ProviderMark` (`GoogleDriveMark` + new `DropboxMark`); i18n strings
parameterised with `{{provider}}`; Shares row only for Drive.

Files: `src/api/{setup,sync,workspace,errors}.ts`, `src/store/{app,drive,
events,workspace}.ts`, `src/components/Start/{Welcome,Drive,index}.tsx`,
`Settings/Sections/Sync/*`, `Settings/Sections/Workspaces/*`,
`elements/{DriveVaults,DriveRestoreForm,GoogleDriveMark}.tsx`, locales,
`src/test/{ipc,start,settings,workspaces}.test.*`, backend `events.rs` and
command names.

Acceptance: existing specs pass under new names; new specs for the picker and
for a Dropbox-connected status; locale files updated for every language.

### PR 4 — Dropbox sharing (later)

Scope: `providers/dropbox/share.rs` (`file_properties` template, shared
links, public `dl=1` fetch), link format v2 with provider tag, `ShareTooNew`
path for old builds, Shares row enabled for Dropbox.

### Later / optional

- Upload sessions for packs over 150 MiB.
- Conditional write on Dropbox (`update rev`): done in PR 4 for `update`;
  `create` still uses overwrite, so two devices' simultaneous first push
  leaves the later pack and the next merge recovers the other.
- A connect that fails mid-write after removing the other provider's token
  file leaves the session flagged connected with no file. Only reachable if an
  already-connected vault connects a second provider, which the UI does not
  offer; write the new file before removing the old one if that ever changes.
- Move the pure sync modules (`engine`, `pack`, `layout`, identity rules)
  into `rowel-core` once a second consumer exists.

## 7. The IPC contract after PR 3

Serialized `Provider`: `"googleDrive" | "dropbox"` (camelCase, like every
other payload).

Commands (old → new):

| Old | New | Args |
|---|---|---|
| `setup_drive_connect` | `setup_connect` | `{ provider }` |
| `setup_drive_disconnect` | `setup_disconnect` | — |
| `setup_restore_from_drive` | `setup_restore_from_account` | `{ password, fileId }` |
| `workspace_drive_connect` | `workspace_connect` | `{ provider }` |
| `workspace_restore_from_drive` | `workspace_restore_from_pending` | `{ name, password, fileId }` |
| `workspace_restore_from_account` | unchanged | `{ name, password, fileId }` |
| `sync_connect` | unchanged | `{ provider }` |
| `sync_adopt_pending`, `sync_disconnect`, `sync_now` | unchanged | — |

Events: `setup:drive:pending|probed|error` → `setup:account:pending|probed|error`.
The probed payload is `{ files: RemotePack[] }` with
`RemotePack = { id, name, vaultId, size, modifiedTime, provider }`
(`SetupDriveFile` plus `provider`). `workspaces:remote` carries the same
shape.

`SyncStatus` gains `provider: Provider | null` (null when not configured).
`WorkspaceStatus` (in `app_status`) gains `provider: Provider | null`, read
from the token file's name so it is known while the workspace is locked.

Error kind `driveNotConnected` → `accountNotConnected`.

## 8. Open decisions (resolved 2026-10-12: all four proposals accepted)

1. Dropbox app-folder vs full-Dropbox access (proposal: app folder).
2. Rename commands/events in PR 3 or keep `setup_drive_*` names forever
   (proposal: rename in PR 3, in one place, with the frontend).
3. Onboarding entry: two buttons on Welcome, or one "Restore from cloud" that
   leads to a picker (proposal: two buttons; the phone screen has room).
4. Shares: Drive-only until PR 4 (proposal: yes).

## 9. iCloud adapter

Status: implemented (`sync/providers/icloud/`), Apple platforms only.

iCloud is not an HTTP service with an OAuth client. There is no sign-in: the
account is whichever Apple ID the device is signed into, and the app reaches
it through CloudKit, a system framework that needs entitlements rather than
tokens. Two things follow, one on each side of the core/adapter line.

### 9.1 Storage: CloudKit, not iCloud Drive

The adapter is a `VaultStore` over CloudKit's **private database**, default
zone, in the container `iCloud.app.rowel` shared by the macOS and iOS apps.
Not an iCloud Drive (ubiquity) container: that is a local folder the system
syncs in the background, so a `stat` there answers from a cache by definition,
nothing carries a server revision, and conflicts surface as `NSFileVersion`
siblings — every one of the invariants in §5 that `head_revision` relies on.
CloudKit is a round trip per call, which is exactly what `PackRemote` assumes.

One record type, `VaultEntry`. The **record name is the entry name** the core
passes (`<vault-id>.rowel`, `<vault-id>.deleted`); fields `pack` (a `CKAsset`
holding the bytes) and `size` (Int64). The handle the core holds is the record
name, the revision is the record's `recordChangeTag`, and the time the probe
orders by is its `modificationDate`.

| `VaultStore` | CloudKit |
|---|---|
| `list_vaults` | `CKQueryOperation` on the type, `TRUEPREDICATE`, cursor followed to the end; `desiredKeys = [size]` so a listing never downloads an asset |
| `stat` | fetch by record id; `UnknownItem` is `None` |
| `read` | the fetched record's asset file, size-checked against the cap before it is read whole |
| `create` | a new record saved with `ifServerRecordUnchanged`: one already there fails (`ServerRecordChanged`) rather than being overwritten |
| `update(entry)` | fetch, compare the server's change tag to the revision the run pulled, refuse on a mismatch, then save with `ifServerRecordUnchanged` — a compare-and-swap, like Dropbox's `update <rev>` |
| `delete` | delete by record id; `UnknownItem` counts as deleted |

The schema has to exist in the container's **Production** environment before
a distribution build can use it; CloudKit creates types on the fly only in
Development. The one-time console steps are in `docs/releasing.md`.

### 9.2 Account: a provider without a sign-in

`Provider::oauth()` is `Option<&OAuthConfig>`, `None` for iCloud, and
`signs_in_with_oauth()` reads it. Everything that needed an OAuth client
already took the config from the provider, so the OAuth paths are unchanged
and simply never see iCloud:

- **Connect** is `account::device_account_tokens`: ask CloudKit for the
  account status, refuse unless it is `Available` (with a sentence saying
  what to do — sign in to iCloud in System Settings), and hand back `Tokens`
  holding nothing but the provider. Desktop reaches it through
  `obtain_tokens`; the phone's `connect_pending` takes it explicitly, since
  its consent flow is cut in two around Safari and there is no Safari here.
  Both report through the same `setup:account:*` events.
- **The token file** `auth/icloud.swftx` holds `{"provider":"iCloud",
  "account":"<user record name>"}` and nothing else, and is still what says
  "this workspace syncs through iCloud". One file per workspace, the
  generation guard, disconnect, re-seal and the locked-workspace provider all
  read it unchanged. `is_configured` accepts a file with no grant when the
  provider has none to hold.
- **The account is bound, not assumed.** An OAuth token is useless in any
  other account; iCloud's "token" would follow whichever Apple ID the device
  is signed into, and `plan_vault_id` keeps a vault's id whatever the account
  holds — so a device that changed Apple ID would push the vault into the new
  account's empty database on its next run. The connect therefore records
  CloudKit's user record name for the container (opaque, stable per Apple
  ID), and every store checks the device's current one against it once per
  run, one extra round trip; a mismatch fails every call with "signed into a
  different iCloud account; disconnect, then connect again". A file with no
  account recorded is treated the same way.
- **The store** takes an `Auth` like every adapter, so `into_tokens` and the
  pending-store round trip work, but never asks it for an access token.
- **Refresh** cannot happen: `fresh_access_token` is only reached from an
  OAuth adapter's `access_token`, and a provider without a config fails it
  rather than silently continuing.

### 9.3 Availability: the first provider that is not everywhere

Google and Dropbox exist on every build. iCloud exists on Apple targets, and
on macOS only in a binary **signed with the iCloud entitlements**: `tauri dev`
and an unsigned build have none, and CloudKit aborts the process with an
Objective-C exception on first use rather than returning an error. So:

- `Provider::supported()` is `false` off Apple; `true` on iOS (the committed
  entitlements carry the keys, and `check-ipa-entitlements.sh` refuses an IPA
  that lost them); on macOS it reads the running binary's own signature
  (`SecTaskCopyValueForEntitlement` for `com.apple.developer.icloud-services`)
  once and caches the answer.
- `app_status` carries `providers: Provider[]`, the providers this build can
  connect, in picker order. Every picker in the frontend renders that list,
  never the static one. A connect command for an unsupported provider is
  refused before an attempt begins (`ensure_offered`).
- Moving a workspace that syncs through iCloud to Windows or Linux leaves it
  connected to a provider that build cannot reach; every run fails with the
  same "only available on Apple devices" error until the user disconnects.
  Saying so in the UI is a follow-up.

### 9.4 Entitlements

Both apps ask for `com.apple.developer.icloud-services = [CloudKit]` and
`com.apple.developer.icloud-container-identifiers = [iCloud.app.rowel]`. On
macOS both are profile-gated like `keychain-access-groups`, so the Developer
ID profile must grant them, which it does only once the App ID has the iCloud
capability — `check-macos-entitlements.mjs` fails the release until then. The
AutoFill and Safari extensions do not sync and do not get them.

### 9.5 Not done

- Shares: none through iCloud, like Dropbox (the link format has no provider
  tag yet). `CKShare` would be the mechanism.
- Packs larger than a single `CKAsset` comfortably holds are not special-cased;
  `MAX_PACK_BYTES` (256 MiB) is within CloudKit's asset limit.
- A `CKSubscription` push could trigger a run when another device writes;
  today a run is scheduled exactly as for the other providers.
