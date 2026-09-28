//! The one probe the frontend runs at launch.
//!
//! Everything here is cheap, non-secret and answerable while locked, which is
//! why it can be one round trip instead of the eight commands it replaces.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::Result;
use crate::secure_store::{self, GateMode};
use crate::settings::Settings;
use crate::state::{AppState, SyncStatus};
use crate::{biometrics, locale, scan, settings, storage, window, workspace};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    /// A SQLite vault DB exists. A legacy `vault.swftx` alone does NOT count:
    /// the app starts fresh and offers an explicit import instead.
    initialized: bool,
    version: String,
    /// The locale to open in: the stored choice narrowed to a shipped
    /// catalogue, or the OS's (see `locale::resolve_preferred`).
    locale: String,
    /// Every preference, hydrated into the frontend's prefs store at boot.
    settings: Settings,
    /// Sync exactly as `sync:status` carries it, so a webview that has just
    /// booted and one that has been listening hold the same value — and the
    /// frontend has one place to read "does this vault sync" from instead of
    /// two that drift apart.
    sync: SyncStatus,
    scan_supported: bool,
    biometric: Biometric,
    /// Every vault on this install — exactly one until the user makes a second,
    /// which is what lets the frontend leave the whole feature out of the UI
    /// until there is something to switch between.
    workspaces: Vec<workspace::WorkspaceStatus>,
    /// Which of them the fields above describe.
    active_workspace: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Biometric {
    /// The platform gates a secure store, the hardware is there, and the user
    /// has opted in.
    available: bool,
    /// Whether enrolling *could* work here — what onboarding asks before
    /// offering it, where `available` would always say no.
    can_enroll: bool,
    /// "face", "touch" or "none", so the UI can name the gate rather than
    /// guess it from the platform (a Touch ID iPad is not Face ID).
    #[serde(rename = "type")]
    kind: &'static str,
    /// Which gate the enrolled key actually sits behind, or `null` when there
    /// is no enrollment.
    mode: Option<String>,
}

/// Run the probe off the main thread: it stats a handful of files and asks the
/// OS about biometrics, none of which needs the UI thread and all of which the
/// window would otherwise wait behind.
#[tauri::command(async)]
pub fn app_status(app: AppHandle) -> Result<AppStatus> {
    snapshot(&app)
}

/// The probe's answer, callable from Rust as well as over IPC.
pub fn snapshot(app: &AppHandle) -> Result<AppStatus> {
    let state = app.state::<AppState>();
    // Every path below resolves through the active workspace, and this runs off
    // the main thread now, so a `workspace_select` could otherwise land between
    // two of the reads and hand back one workspace's sync state under another's
    // identity. Taken first, as every holder takes it, and held for the whole
    // answer: the reads under it are the registry and a few metadata stats, the
    // same class of I/O the switch itself does while holding it.
    let _paths = state.workspace_lock.lock().unwrap();
    let gate = biometrics::probe();
    let hardware = secure_store::is_supported() && gate.available;
    let marker = storage::biometric_marker(app);

    // The session flag only exists after an unlock; while locked, answer from
    // the persisted (non-secret) settings so e.g. the lock screen can say
    // where the vault lives.
    let session = state.session.lock().unwrap();
    let sync_configured = if session.is_unlocked() {
        session.sync_configured
    } else {
        storage::sync_configured(app)
    };
    drop(session);

    // The same snapshot `sync:status` carries, built the same way — and for the
    // same workspace, since both read the active one's run state.
    let sync = state.sync_run(|run| run.status(sync_configured));

    let settings = settings::current(app);

    let root = storage::root_dir(app)?;
    let registry = workspace::Registry::load(&root);
    // The in-memory id, not the registry's: it is what every path above was
    // resolved through, so it is what `initialized` and the rest are about.
    let active_workspace = workspace::active_id(app);
    let primary = active_workspace == workspace::PRIMARY_ID;
    // The enrolled key is the app key (the primary's). It opens the primary
    // outright, and any other workspace whose own key is sealed under it on
    // this device (`crate::appkey`) — one that is not has to be opened with
    // its password once first.
    let openable = primary || crate::appkey::is_wrapped(&root, &active_workspace);
    // Enrolling stores the app key, so it has to be in hand: it is whenever the
    // primary is open or has been opened this session.
    let app_key_known = primary || state.keyring.lock().unwrap().app_key().is_some();

    Ok(AppStatus {
        initialized: storage::db_exists(app),
        version: app.package_info().version.to_string(),
        locale: locale::resolve_preferred(settings.locale.as_deref()),
        settings,
        sync,
        scan_supported: scan::is_supported(),
        biometric: Biometric {
            available: hardware && storage::biometric_enrolled(app) && openable,
            can_enroll: hardware && app_key_known,
            kind: gate.kind,
            mode: marker.map(|m| GateMode::from_marker(&m).as_marker().to_string()),
        },
        workspaces: crate::workspace::statuses(&registry, &root),
        active_workspace,
    })
}

/// What the webview is handed before its own scripts run, as
/// `window.__ROWEL_BOOT__`.
///
/// Only the two answers the *first painted frame* needs: the theme (inside the
/// settings) and the language the catalogue is loaded for. Everything else stays
/// in `app_status`, which the frontend probes for anyway — an injected script is
/// frozen at window creation, so anything that can change while the app runs
/// would be stale after a webview reload.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Boot {
    /// The stored choice narrowed to a shipped catalogue, or the OS's.
    locale: String,
    settings: Settings,
}

/// The `window.__ROWEL_BOOT__` assignment to inject into the main window.
pub fn boot_script(app: &AppHandle) -> String {
    let settings = settings::current(app);
    script(&Boot {
        locale: locale::resolve_preferred(settings.locale.as_deref()),
        settings,
    })
}

/// Serialised twice on purpose: once to JSON, then that JSON string again to
/// get a JS string literal. Parsing the literal back at runtime is what keeps a
/// quote, a backslash or a line separator inside a user-typed value (the
/// generator's exclude list) from ending the expression it sits in.
fn script(boot: &Boot) -> String {
    let json = serde_json::to_string(boot).unwrap_or_else(|_| "{}".into());
    let literal = serde_json::to_string(&json).unwrap_or_else(|_| "\"{}\"".into());
    format!("window.__ROWEL_BOOT__ = Object.freeze(JSON.parse({literal}));")
}

/// The frontend saying its first frame is up. The window is hidden until this
/// lands (or the fallback fires), so the reveal and the splash choreography
/// start together instead of racing.
#[tauri::command]
pub fn app_ready(app: AppHandle) {
    window::reveal(&app);
}

/// The backup the OS asked the app to open before the webview was listening
/// for `file:opened` — a launch by double-clicking one. Handed over once (see
/// `crate::opened`); `null` on every ordinary launch.
#[tauri::command]
pub fn take_opened_file(app: AppHandle) -> Option<String> {
    crate::opened::take(&app)
}

/// Apply a partial settings object and hand the whole merged result back, so a
/// caller that patched one key ends up holding exactly what is on disk. The
/// auto-lock is re-armed inside `settings::set`, under its lock, rather than by
/// the frontend: the file is the only place the value changes.
#[tauri::command]
pub fn set_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings> {
    settings::set(&app, &patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The payload is pasted into a JS statement, so the one thing that can go
    // wrong is a value escaping its literal. Anything a user can type ends up
    // in `generator.exclude`.
    #[test]
    fn the_boot_script_round_trips_through_a_json_parse() {
        let mut settings = Settings::default();
        settings.generator.exclude = "'\"\\\n</script>\u{2028}".into();
        let boot = Boot {
            locale: "uk-UA".into(),
            settings,
        };

        let generated = script(&boot);
        let literal = generated
            .strip_prefix("window.__ROWEL_BOOT__ = Object.freeze(JSON.parse(")
            .and_then(|rest| rest.strip_suffix("));"))
            .expect("the assignment keeps its shape");

        // What the webview does: read the literal, then parse what it holds.
        let json: String = serde_json::from_str(literal).expect("a valid JS string literal");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON inside");

        assert_eq!(parsed["locale"], "uk-UA");
        assert_eq!(parsed["settings"]["theme"], "light");
        assert_eq!(
            parsed["settings"]["generator"]["exclude"],
            boot.settings.generator.exclude
        );
    }
}
