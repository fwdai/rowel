use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;

use crate::error::Error;
use crate::session::Session;

/// Why a mobile consent flow was started — which decides what happens once the
/// redirect comes back, and which events the frontend is told through.
///
/// One purpose, because every connect is now keyless: the tokens are handed
/// back and probed before anything is sealed under a vault key, so there is
/// nothing to tell apart at the redirect. Kept as an enum so a flow that does
/// need to be told apart later has somewhere to say so.
#[cfg(mobile)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPurpose {
    /// Connect an account without sealing its tokens under any vault, so the
    /// user can be shown what is up there and choose what to do with it: the
    /// first run, the Settings flow that adds a workspace by restoring one of
    /// the account's other vaults, and a sync connect on the open vault (which
    /// joins the account only if the account is empty or already holds it —
    /// see `commands::sync::sync_adopt_pending`). What asked is the frontend's
    /// to remember; the redirect does the same thing either way and leaves the
    /// tokens in [`AppState::pending_account`].
    Setup,
}

/// A mobile consent flow waiting for the browser to come back.
///
/// The OAuth redirect arrives as a deep link long after the command that
/// started the flow returned, so what the exchange needs has to survive in
/// between: the PKCE verifier the code is redeemed with, and why the flow was
/// started.
///
/// It has an identity and an age. The `state` nonce went out in the consent URL
/// and must come back on the redirect, so a stray URL on the same scheme cannot
/// consume this in place of Google's callback; `started` is what lets a flow
/// nobody finished expire instead of pending forever.
#[cfg(mobile)]
pub struct PendingAuth {
    pub verifier: String,
    pub state: String,
    pub purpose: AuthPurpose,
    /// Whose consent page was opened: which redirect (and cancel landing) to
    /// recognise, and which client redeems the code.
    pub provider: crate::sync::Provider,
    pub started: std::time::Instant,
}

/// OAuth tokens produced by one particular consent attempt — any provider's;
/// the tokens name theirs.
///
/// The attempt travels with the tokens so an async consumer cannot mistake a
/// newer account's pending credentials for the ones it probed before an await.
pub struct PendingAccount {
    pub attempt: u64,
    pub tokens: crate::sync::Tokens,
}

/// The sequence number every `sync:status` snapshot carries, counted once for
/// the whole process rather than per workspace.
///
/// The frontend keeps whichever of two snapshots has the higher sequence
/// (`store/app.ts`), because a launch probe can resolve after the event of a
/// transition it predates. A workspace switch hands it a snapshot about a
/// *different* workspace, which has been through a different number of
/// transitions of its own — so a per-workspace counter would have the switched-to
/// status discarded as stale whenever the workspace being left had seen more of
/// them. Counted here, anything the backend produces later reads as later,
/// whichever workspace it describes.
static SYNC_SEQ: AtomicU64 = AtomicU64::new(0);

/// Sync as the frontend sees it, for one workspace. Every transition in
/// `commands::sync` updates this and re-emits the whole thing as `sync:status`,
/// so the frontend mirrors one value instead of reconstructing it from an order
/// of events. Process lifetime, not session: a lock does not un-happen the last
/// successful run.
///
/// One of these per workspace, reached through [`AppState::sync_run`]: each has
/// its own Drive connection, so each has its own last run to report.
#[derive(Default)]
pub struct SyncRun {
    /// A consent flow is out with the browser.
    pub pending: bool,
    pub in_progress: bool,
    /// What the last connect or run failed with, until the next one starts.
    pub error: Option<SyncFailure>,
    /// RFC 3339 time of the last run of *this workspace* that succeeded in this
    /// process.
    pub last_synced_at: Option<String>,
    /// Where this workspace's last transition fell in the process-wide order
    /// ([`SYNC_SEQ`]). Every snapshot carries it, so two of them can be put in
    /// order by whoever holds them.
    pub seq: u64,
}

impl SyncRun {
    /// Record a transition. Every change goes through here so the sequence
    /// advances with it; a snapshot taken before the change reads as older.
    pub fn transition(&mut self, change: impl FnOnce(&mut SyncRun)) {
        change(self);
        self.seq = SYNC_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    }

    /// A run ended. A failed run leaves the previous timestamp standing: the
    /// vault is still current as of whenever it last landed. Both the timestamp
    /// and the error belong to this workspace alone — another workspace's
    /// success never stands in for this one's, and its failure is not this
    /// one's to report.
    pub fn finish(&mut self, error: Option<SyncFailure>) {
        self.in_progress = false;
        if error.is_none() {
            self.last_synced_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
        }
        self.error = error;
    }

    /// The snapshot the frontend gets. `configured` is the session's to answer
    /// and `provider` the token file's, so the caller supplies both.
    pub fn status(&self, configured: bool, provider: Option<crate::sync::Provider>) -> SyncStatus {
        SyncStatus {
            configured,
            provider,
            pending: self.pending,
            in_progress: self.in_progress,
            error: self.error.as_ref().map(|e| e.message.clone()),
            error_kind: self.error.as_ref().map(|e| e.kind.to_string()),
            last_synced_at: self.last_synced_at.clone(),
            seq: self.seq,
        }
    }
}

/// A failed connect or run, kept as both halves.
///
/// The message is what the user reads; the kind is [`crate::error::Error::kind`],
/// so a screen can recognise one particular failure — a vault deleted from the
/// account on another device, say — without matching on English prose the way
/// every rejection the frontend gets is spared from doing.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    pub kind: &'static str,
    pub message: String,
}

impl From<&Error> for SyncFailure {
    fn from(error: &Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

/// The whole of what the frontend knows about sync, carried by every
/// `sync:status` event and by the launch probe. Owned here, not by the
/// frontend — the backend is what starts and ends every flow, so it is the one
/// that can say.
///
/// Two snapshots can reach the frontend out of order: a probe taken just
/// before a transition, resolving after the event that transition emitted.
/// `seq` is what lets the frontend keep the newer one without guessing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// This vault has a provider connected.
    pub configured: bool,
    /// Which provider, read off the workspace's token file; `None` (`null`)
    /// whenever `configured` is false.
    pub provider: Option<crate::sync::Provider>,
    pub pending: bool,
    pub in_progress: bool,
    pub error: Option<String>,
    /// The kind of the failure `error` describes — see [`SyncFailure`]. Always
    /// set together with it.
    pub error_kind: Option<String>,
    pub last_synced_at: Option<String>,
    /// Monotonic within the process; see [`SyncRun::seq`].
    pub seq: u64,
}

pub struct AppState {
    pub session: Mutex<Session>,
    /// The app-level unlock: every workspace's key once the app is open, so a
    /// switch opens the next database without a prompt (see `crate::appkey`).
    /// Outside `session` because it outlives one — a switch ends the session
    /// and keeps the ring — and ends with every lock instead.
    pub keyring: Mutex<crate::appkey::Keyring>,
    /// Held across every step that decides the ring's app key and then reads or
    /// writes the sealed-key sidecars by it (`crate::appkey`): a proof of one
    /// app key and a password change that replaces it run whole, one after the
    /// other, rather than the slower one writing sidecars under a key the
    /// faster one has already retired. Taken before `session`, never inside it,
    /// so a lock never waits on filesystem work.
    pub sidecars: Mutex<()>,
    /// Which workspace every vault path resolves to right now.
    ///
    /// Held in memory rather than read from the registry on each path lookup:
    /// it is consulted on essentially every file access, and it is also what
    /// keeps a switch atomic — the session is cleared and this is set together,
    /// so nothing can address one workspace's database with another's key.
    pub active_workspace: Mutex<String>,
    /// Held while the paths move (`commands::workspace`) and while a sync run or
    /// consent flow claims them (`commands::sync`): the flow takes its key and
    /// raises its flag under this lock, and a switch checks those flags and
    /// moves `active_workspace` under it, so neither can slip in between the
    /// other's check and its act. Never held across I/O.
    pub workspace_lock: Mutex<()>,
    // A sync run is in flight. Held outside `session` on purpose: the run takes
    // and releases the session lock repeatedly (never across a network call),
    // so the "one at a time" guard cannot live behind that same lock.
    //
    // Behind an `Arc` so the claim on it can be a guard the run *owns* for its
    // whole length (`commands::sync::RunClaim`), rather than a flag raised here
    // and lowered by hand on the thread that happens to end the run.
    pub syncing: Arc<AtomicBool>,
    /// A run was asked for while `syncing` was held. The run in flight may have
    /// packed before the write that asked, so it is run once more when it ends
    /// (`commands::sync::spawn_run`), rather than leaving that write for the
    /// next unlock. Raised *before* the claim is tried and lowered when it is
    /// won, so no request can fall between a run's release and its check.
    pub sync_rerun: AtomicBool,
    /// Which Drive connection is current, and the guard on the token file.
    ///
    /// A token refresh reads the file, awaits a network round trip, and writes
    /// the refreshed tokens back — and cannot hold a lock across that await
    /// (see `workspace_lock`). A disconnect landing in that window would
    /// otherwise have its delete undone by the write-back, leaving the account
    /// connected again at the next unlock. So the refresh reads this before the
    /// round trip and writes back only if it is unchanged — with the compare and
    /// the write under this lock, as are the disconnect's bump and delete, and
    /// the password change's re-seal. A bare compare would leave a gap between
    /// it and the write for the disconnect to land in. Held across local file
    /// operations only, never a network call.
    pub sync_generation: Mutex<u64>,
    /// Sync as reported to the frontend, one entry per workspace; also what
    /// `commands::workspace` reads to refuse a switch while a consent flow or a
    /// run is out. Reached through [`AppState::sync_run`], never directly.
    pub sync_runs: Mutex<HashMap<String, SyncRun>>,
    /// Tokens for an account connected during first-run onboarding.
    ///
    /// Memory only, and deliberately so: they are sealed under the vault key,
    /// and on a fresh install that key does not exist yet. They live here from
    /// the moment consent is granted until the restore or create that follows
    /// produces a key to write them under (or the user backs out, which drops
    /// them). Outside `session` because there is no session to put them in.
    pub pending_account: Mutex<Option<PendingAccount>>,
    /// A first-run create or restore is writing the vault. Same shape as
    /// `syncing`: the check for "no vault yet" and the writes that follow it
    /// are not one step, so two overlapping requests could both pass the check
    /// and interleave a database with the other's KDF sidecar.
    pub setup_busy: AtomicBool,
    /// Which first-run Drive connect is current. Bumped when one starts and
    /// again when the user backs out, so a consent that completes after it was
    /// abandoned can tell it is stale and drop its tokens instead of adopting
    /// an account the user already walked away from.
    pub setup_attempt: AtomicU64,
    /// A backup the OS asked the app to open (a double-clicked `.rowel` or
    /// `.swftx`) that the webview has not collected yet. Parked because the
    /// request can land before the webview is listening — at launch it always
    /// does — and handed over once through `commands::app::take_opened_file`.
    /// See `crate::opened`.
    pub pending_open: Mutex<Option<String>>,
    #[cfg(mobile)]
    pub pending_auth: Mutex<Option<PendingAuth>>,
    /// How long a resumed app waits for a redirect before writing the pending
    /// consent flow off (`commands::sync::on_resume`). One re-armable timer
    /// rather than a sleeping thread per activation; the latest resume wins.
    #[cfg(mobile)]
    pub consent_grace: std::sync::Arc<crate::timer::Timer>,
    /// The grants issued to agents through the browser extension, and each
    /// client's recent asking (`crate::agents`). Memory only, and cleared with
    /// every lock: a grant outlives nothing. Taken after `session`, never
    /// before it, since writing a grant's audit row needs the open store.
    #[cfg(desktop)]
    pub agents: Mutex<crate::agents::AgentState>,
    /// Armed to the earliest moment a grant expires, and re-armed from its own
    /// callback (`agents::rearm`). One timer for every grant, like the auto-lock.
    #[cfg(desktop)]
    pub agent_expiry: std::sync::Arc<crate::timer::Timer>,
    /// This install's id, the audit log's writer id (`storage::install_id`),
    /// read or minted once per process and then fixed: two first calls cannot
    /// mint two ids, and a root that will not take the file still leaves the
    /// process with one id.
    #[cfg(desktop)]
    pub install_id: std::sync::OnceLock<String>,
    /// The browser host's connections, which the lock signals and the
    /// agents' wake-ups reach (`browser::server::hub`): one for the app, fed
    /// by both of its listeners.
    #[cfg(desktop)]
    pub hub: rowel_broker::hub::Hub,
}

impl AppState {
    /// The active workspace's run state, created empty the first time it is
    /// touched.
    ///
    /// Sync is per workspace, not per process: each connects its own Drive
    /// account, so one must never show another's "last synced" or another's
    /// error. A flow never spans a switch — `commands::workspace::guard_sync_idle`
    /// refuses one while a consent flow or a run is out — so the active
    /// workspace is always the one a transition belongs to, and the one a status
    /// describes.
    ///
    /// A closure rather than a borrow of the entry: it lives inside the map's
    /// lock, and handing the entry out would mean handing the lock out with it.
    pub fn sync_run<R>(&self, with: impl FnOnce(&mut SyncRun) -> R) -> R {
        // The id is read and released before the map is locked, so the two are
        // never held at once and no order between them has to be agreed on.
        let active = self.active_workspace.lock().unwrap().clone();
        with(self.sync_runs.lock().unwrap().entry(active).or_default())
    }
}

// Hand-written only because `active_workspace` starts at the primary rather than
// at `String::default()`. `lib.rs` overwrites it from the registry at startup.
impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::default(),
            keyring: Mutex::default(),
            sidecars: Mutex::default(),
            active_workspace: Mutex::new(crate::workspace::PRIMARY_ID.to_string()),
            workspace_lock: Mutex::default(),
            syncing: Arc::default(),
            sync_rerun: AtomicBool::default(),
            sync_generation: Mutex::default(),
            sync_runs: Mutex::default(),
            pending_account: Mutex::default(),
            setup_busy: AtomicBool::default(),
            setup_attempt: AtomicU64::default(),
            pending_open: Mutex::default(),
            #[cfg(mobile)]
            pending_auth: Mutex::default(),
            #[cfg(mobile)]
            consent_grace: crate::timer::Timer::spawn(),
            #[cfg(desktop)]
            agents: Mutex::default(),
            #[cfg(desktop)]
            agent_expiry: crate::timer::Timer::spawn(),
            #[cfg(desktop)]
            install_id: std::sync::OnceLock::new(),
            #[cfg(desktop)]
            hub: rowel_broker::hub::Hub::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // What a workspace switch does to the state these tests are about: the
    // session goes and the paths — and with them the run state — point
    // elsewhere (`commands::workspace::workspace_select`).
    fn switch_to(state: &AppState, id: &str) {
        *state.active_workspace.lock().unwrap() = id.to_string();
    }

    // Every transition the app makes goes through `SyncRun::transition`, so the
    // tests below reach the run state the way `commands::sync` does.
    fn finish(state: &AppState, error: Option<&str>) {
        let failure = error.map(|why| SyncFailure::from(&Error::Other(why.into())));
        state.sync_run(|run| run.transition(|run| run.finish(failure)));
    }

    #[test]
    fn a_workspace_reports_its_own_last_run_and_never_anothers() {
        let state = AppState::default();

        // A syncs, successfully.
        switch_to(&state, "a");
        finish(&state, None);
        let a = state.sync_run(|run| run.status(true, None));
        assert!(a.last_synced_at.is_some());
        assert_eq!(a.error, None);

        // B has never synced: neither A's timestamp nor A's (absent) error is
        // any part of what B has to say about itself.
        switch_to(&state, "b");
        let before = state.sync_run(|run| run.status(true, None));
        assert_eq!(before.last_synced_at, None);
        assert_eq!(before.error, None);

        // A failed run keeps the previous timestamp — B's, which is none. The
        // error is B's own.
        finish(&state, Some("no network"));
        let failed = state.sync_run(|run| run.status(true, None));
        assert_eq!(failed.last_synced_at, None);
        assert_eq!(failed.error.as_deref(), Some("no network"));

        // And nothing that happened to B happened to A.
        switch_to(&state, "a");
        let back = state.sync_run(|run| run.status(true, None));
        assert_eq!(back.last_synced_at, a.last_synced_at);
        assert_eq!(back.error, None);
    }

    // The frontend keeps the snapshot with the higher sequence, so the one a
    // switch announces has to outrank the workspace it left however many
    // transitions each of them has been through.
    #[test]
    fn a_switched_to_workspace_outranks_the_one_the_frontend_is_holding() {
        let state = AppState::default();
        switch_to(&state, "a");
        for _ in 0..3 {
            state.sync_run(|run| run.transition(|_| {}));
        }
        let held = state.sync_run(|run| run.status(true, None));

        // One transition against a workspace that has never had one: exactly
        // what `commands::sync::switched` does after the repoint.
        switch_to(&state, "b");
        state.sync_run(|run| run.transition(|_| {}));
        assert!(state.sync_run(|run| run.status(true, None)).seq > held.seq);
    }

    // `provider` is additive to the payload the frontend already reads: the
    // existing fields keep their names, and a vault with no connection says
    // `null`.
    #[test]
    fn the_status_names_the_connected_provider() {
        let run = SyncRun::default();
        let connected =
            serde_json::to_value(run.status(true, Some(crate::sync::Provider::Dropbox))).unwrap();
        assert_eq!(connected["configured"], true);
        assert_eq!(connected["provider"], "dropbox");
        assert!(connected.get("inProgress").is_some());
        assert!(connected.get("lastSyncedAt").is_some());

        let local = serde_json::to_value(run.status(false, None)).unwrap();
        assert_eq!(local["provider"], serde_json::Value::Null);
    }
}
