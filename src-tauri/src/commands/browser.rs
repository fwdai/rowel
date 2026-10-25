//! The browser extension host's switches: on or off, the extensions let in,
//! the answer to one asking — and, for agents, the Agents switch, the answer
//! to a grant ask, and the rules and grants Settings lists.

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::agents::{self, GrantRow, RuleRow};
use crate::browser::{self, manifest, Client, GrantAnswer, Pairing, StepUp};
use crate::error::{Error, Result};
use crate::settings::{self, AgentsSettings, BrowserSettings, MAX_STANDING_DAYS};
use crate::state::AppState;
use crate::{biometrics, session, storage};

use super::blocking;

/// The host as Settings shows it. `clients` are the open vault's — the list
/// is kept inside each vault — so it is empty while locked.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub enabled: bool,
    pub keepassxc_compat: bool,
    pub agents: AgentsSettings,
    pub browsers: Vec<manifest::Status>,
    pub clients: Vec<Client>,
}

fn status(app: &AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(app)?;
    let host = settings::current(app).browser;
    Ok(BrowserStatus {
        enabled: host.enabled,
        keepassxc_compat: host.keepassxc_compat,
        agents: host.agents,
        browsers: manifest::status(&root, host.keepassxc_compat),
        clients: browser::clients(app),
    })
}

// The group goes back whole: a settings patch replaces a top-level key, so
// sending one field would reset the other to its default. Settings sends one
// change at a time, so nothing lands between the read and the write.
fn save(app: &AppHandle, change: impl FnOnce(&mut BrowserSettings)) -> Result<BrowserSettings> {
    let mut host = settings::current(app).browser;
    change(&mut host);
    Ok(settings::set(app, &json!({ "browser": host }))?.browser)
}

#[tauri::command]
pub fn browser_status(app: AppHandle) -> Result<BrowserStatus> {
    status(&app)
}

/// Turn the host on — writing the manifests every browser here finds it by,
/// and listening — or off, taking the manifests back. The listener stays up
/// once started (see `browser::server`); off means every connection, the
/// ones up included, is refused at its next request.
#[tauri::command]
pub fn browser_set_enabled(enabled: bool, app: AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(&app)?;
    let host = save(&app, |host| host.enabled = enabled)?;
    if enabled {
        manifest::install(&root, host.keepassxc_compat);
        browser::server::start(&app);
    } else {
        manifest::remove(&root);
    }
    status(&app)
}

/// Also register under KeePassXC's host name, for the stock KeePassXC-Browser
/// extension — or stop, taking that manifest back and leaving Rowel's own.
/// With the host off there is nothing to write; the choice waits for it.
#[tauri::command]
pub fn browser_set_keepassxc_compat(enabled: bool, app: AppHandle) -> Result<BrowserStatus> {
    let root = storage::root_dir(&app)?;
    let host = save(&app, |host| host.keepassxc_compat = enabled)?;
    if host.enabled {
        manifest::install(&root, host.keepassxc_compat);
    }
    status(&app)
}

/// Let agents use logins through the extension — every fill in agent mode a
/// grant — or stop. What a change of the switch does to the grants up and
/// the ask on screen is the settings write's own business
/// (`settings::set` → `agents::change_access`), the same whichever command
/// patched it.
#[tauri::command]
pub fn browser_set_agents_enabled(enabled: bool, app: AppHandle) -> Result<BrowserStatus> {
    save(&app, |host| host.agents.enabled = enabled)?;
    status(&app)
}

/// What the grant dialog's "for N days" starts at, 1 to 90.
#[tauri::command]
pub fn browser_set_agents_standing_days(days: u32, app: AppHandle) -> Result<BrowserStatus> {
    if !(1..=MAX_STANDING_DAYS).contains(&days) {
        return Err(Error::Other(format!(
            "a standing rule lasts between 1 and {MAX_STANDING_DAYS} days"
        )));
    }
    save(&app, |host| host.agents.default_standing_days = days)?;
    status(&app)
}

/// The user's answer to the client asking to connect under `id`: the name
/// they gave it and, for an extension, whether an AI agent drives that
/// browser — or a `null` name to refuse. Nothing happens when no ask is up
/// under that id — a dialog answering late, after its ask timed out and
/// another took its place, approves nothing, not even the same client's
/// retry.
#[tauri::command]
pub fn browser_respond(
    id: String,
    name: Option<String>,
    driven_by_agent: Option<bool>,
) -> Result<()> {
    let pairing = name.map(|name| Pairing {
        name,
        driven_by_agent: driven_by_agent.unwrap_or(false),
    });
    browser::respond(&id, pairing);
    Ok(())
}

/// The user's answer to the passkey ceremony asked under `id`: `allow`, and
/// for a sign-in which of the ask's accounts (`account`, the newest when
/// unsaid). Nothing happens when no ask is up under it — a dialog answering
/// late, after its ask timed out and another took its place, approves nothing.
#[tauri::command]
pub fn browser_passkey_respond(id: String, allow: bool, account: Option<usize>) -> Result<()> {
    browser::respond_passkey(&id, allow.then(|| account.unwrap_or(0)));
    Ok(())
}

/// The user's answer to the grant ask up under `id`: the item the agent may
/// use and, for "allow for N days", how many — or `null` to refuse. For a
/// step-up ask a yes counts only once the person is verified: the biometric
/// prompt where the device has one, else the master `password`, checked
/// against the key the open lockbox is unlocked with. A verification that
/// fails is the refusal, delivered to the waiting request at once; the
/// extension may ask again, which is a new dialog. As with the other two,
/// nothing happens when no ask is up under `id`.
#[tauri::command]
pub async fn agent_grant_respond(
    id: String,
    answer: Option<GrantAnswer>,
    password: Option<Zeroizing<String>>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<()> {
    if answer.is_some() && browser::grant_ask_needs_step_up(&id) {
        if let Err(e) = verify_person(&app, &state, password).await {
            browser::respond_grant(&id, None);
            return Err(e);
        }
    }
    browser::respond_grant(&id, answer);
    Ok(())
}

// The person at the keyboard, proved the way the platform can. The password
// path derives from the open lockbox's own KDF sidecar — the derivation
// `unlock` runs — and compares the key material with what the session holds,
// in constant time; no second store is opened. Argon2id and the OS prompt
// both block, so both run off the command's thread.
async fn verify_person(
    app: &AppHandle,
    state: &AppState,
    password: Option<Zeroizing<String>>,
) -> Result<()> {
    match StepUp::current() {
        StepUp::Biometric => blocking(biometrics::authenticate).await,
        StepUp::Password => {
            let password = password.ok_or(Error::InvalidPassword)?;
            let handle = app.clone();
            let candidate = blocking(move || session::derive_key(&handle, &password)).await?;
            super::vault::prove_password(&state.session.lock().unwrap(), &candidate)
        }
    }
}

/// The open lockbox's standing rules, with the names Settings shows beside
/// the ids. Empty while locked.
#[tauri::command]
pub fn agent_rules(app: AppHandle) -> Result<Vec<RuleRow>> {
    agents::list_rules(&app)
}

/// Every grant the app still holds: the active ones and the ones that ended
/// within the hour. Empty while locked.
#[tauri::command]
pub fn agent_grants(app: AppHandle) -> Vec<GrantRow> {
    agents::list_grants(&app)
}

/// Take a standing rule back. The next fill it covered asks again.
#[tauri::command]
pub fn agent_revoke_rule(id: String, app: AppHandle) -> Result<()> {
    agents::revoke_rule(&app, &id)
}

/// End a grant before its window closes. The agent's next request under it
/// is refused.
#[tauri::command]
pub fn agent_end_grant(id: String, app: AppHandle) -> Result<()> {
    agents::end_grant(&app, &id)
}

/// Mark a connected extension as one in a browser an AI agent drives, or not.
/// Marked, every fill it asks for is a grant, whatever the extension's own
/// agent mode says.
#[tauri::command]
pub fn browser_set_driven_by_agent(
    key: String,
    driven: bool,
    app: AppHandle,
) -> Result<BrowserStatus> {
    browser::set_driven_by_agent(&app, &key, driven)?;
    status(&app)
}

/// Take an extension's access to the open vault back. Its next request — on
/// a connection that is up as much as on a new one — fails the association
/// check, and it has to ask, and be let in, again.
#[tauri::command]
pub fn browser_forget_client(key: String, app: AppHandle) -> Result<BrowserStatus> {
    browser::forget(&app, &key)?;
    status(&app)
}
