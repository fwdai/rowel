//! User preferences, owned here rather than in the webview.
//!
//! These used to be a handful of localStorage keys, which meant the backend and
//! the frontend disagreed about half of them: Rust enforced the auto-lock but
//! booted with its own default and waited for the frontend to push the stored
//! one. One file in the app data dir, one struct, one command to patch it.
//!
//! Nothing here is secret — it is what the UI looks like and how long a copied
//! value lingers — so it sits beside the KDF and lockout sidecars in plaintext
//! and is readable while the vault is locked.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use crate::error::Result;
use crate::storage;

pub const DEFAULT_AUTOLOCK_SECS: u64 = 60;
/// A day. The row offers far less, but `set_settings` is reachable from the
/// frontend (and `settings.json` is a file on disk), and a timeout measured in
/// years is indistinguishable from "never" — which is not a setting a password
/// manager should be talked into.
pub const MAX_AUTOLOCK_SECS: u64 = 24 * 60 * 60;
/// An hour. Same reasoning as the auto-lock bound, plus a hard one: the value
/// becomes a `Duration` added to an `Instant` in the clipboard timer, and that
/// addition panics on overflow. Clamped here, at the one place the number
/// enters the app.
pub const MAX_CLIPBOARD_MS: u64 = 60 * 60 * 1000;

/// The seed values for every new password, shared by Settings › Security and
/// the ⌘G dialog: the four character classes, the length, and the look-alike
/// filter. `exclude` has no control of its own yet; it is carried so a future
/// one inherits what is already stored. A file written before `lowercase`
/// existed reads it as on, which is what the generator always drew from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneratorDefaults {
    pub length: u32,
    pub numbers: bool,
    pub symbols: bool,
    pub uppercase: bool,
    pub lowercase: bool,
    pub exclude: String,
    pub exclude_similar_characters: bool,
}

impl Default for GeneratorDefaults {
    fn default() -> Self {
        Self {
            length: 20,
            numbers: true,
            symbols: true,
            uppercase: true,
            lowercase: true,
            exclude: String::new(),
            exclude_similar_characters: false,
        }
    }
}

/// The browser extension host: whether it listens at all. Off until the user
/// turns it on in Settings, which is also what writes the native messaging
/// manifests a browser finds the app by. Which extensions may talk to it is
/// not a preference: that list lives inside each vault (`browser::clients`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserSettings {
    pub enabled: bool,
}

/// Every preference the app has, with the defaults a fresh install starts from.
///
/// The enum-ish fields are plain strings: the frontend's union types narrow
/// them, and a value from a hand-edited file that no branch matches falls
/// through to the same arm the default does. The two timeouts are the fields
/// normalised here (`normalize`): they are enforced by Rust, so the value the
/// file holds and the frontend shows has to be the one the timers run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub autolock_secs: u64,
    pub clipboard_timeout_ms: u64,
    pub date_format: String,
    pub sort: String,
    pub theme: String,
    /// The accent colour, by name (`theme/index.ts` on the frontend holds the
    /// list and narrows an unknown one back to the default).
    pub accent: String,
    /// An explicit language choice. `None` means "follow the OS", which is what
    /// a fresh install does — see `locale::resolve_preferred`.
    pub locale: Option<String>,
    pub breach_check: bool,
    pub generator: GeneratorDefaults,
    pub browser: BrowserSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autolock_secs: DEFAULT_AUTOLOCK_SECS,
            clipboard_timeout_ms: 30_000,
            date_format: "MM/DD/YYYY".into(),
            sort: "recent".into(),
            theme: "light".into(),
            accent: "ink".into(),
            locale: None,
            breach_check: false,
            generator: GeneratorDefaults::default(),
            browser: BrowserSettings::default(),
        }
    }
}

impl Settings {
    /// Put the durable value where the timer will run it: a zero — the one
    /// value the type allows that means nothing — goes back to the default,
    /// and anything past a day is a day. Applied on every read and write, so
    /// what the file holds, what the frontend shows and what `autolock` arms
    /// are the same number.
    fn normalize(mut self) -> Self {
        self.autolock_secs = match self.autolock_secs {
            0 => DEFAULT_AUTOLOCK_SECS,
            secs => secs.min(MAX_AUTOLOCK_SECS),
        };
        // A zero is the frontend's "never clear" (`services/copy.ts` sends no
        // delay at all for it), so it is left alone; every other value is
        // capped, because it is armed as a real timer.
        self.clipboard_timeout_ms = self.clipboard_timeout_ms.min(MAX_CLIPBOARD_MS);
        self
    }
}

/// The current settings, so a read is a lock rather than a file open. Written
/// through `set`, which holds the lock across the merge, the file write and the
/// swap — two writes landing together must each see the other's keys.
#[derive(Default)]
pub struct SettingsState(Mutex<Settings>);

/// Read `settings.json`, falling back to the defaults for a missing or unusable
/// file. Never fails: a preferences file is not worth refusing to start over.
pub fn load(app: &AppHandle) -> Settings {
    let Ok(path) = storage::settings_path(app) else {
        return Settings::default();
    };
    let Ok(json) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    serde_json::from_str::<Settings>(&json)
        .unwrap_or_else(|error| {
            log::warn!("settings.json is not readable, using defaults: {error}");
            Settings::default()
        })
        .normalize()
}

/// The settings as they stand, out of managed state.
pub fn current(app: &AppHandle) -> Settings {
    app.state::<SettingsState>().0.lock().unwrap().clone()
}

/// Seed managed state from disk. Called once at startup, before anything reads.
pub fn hydrate(app: &AppHandle, settings: Settings) {
    *app.state::<SettingsState>().0.lock().unwrap() = settings;
}

/// Read the file into managed state and put the stored auto-lock in force.
///
/// Runs at startup — the auto-lock is enforced here, so the timeout the user
/// picked has to be armed before a session can exist rather than waiting for
/// the frontend to push it — and again after an E2E reset has taken the file
/// away, which would otherwise leave the previous spec's preferences in memory.
pub fn boot(app: &AppHandle) {
    let stored = load(app);
    crate::autolock::set_timeout(app, stored.autolock_secs);
    hydrate(app, stored);
}

/// Apply `patch` over the current settings, persist the result and hand it back.
///
/// The patch is a partial object keyed the way the struct serializes, so the
/// merge is a top-level key overwrite on the JSON form. `generator` is replaced
/// whole — the frontend sends the group it edited, not a single knob.
///
/// One transaction under the lock: a slider drag lands several of these at
/// once, and merging each from its own stale snapshot would let the last write
/// drop the keys the others carried. A failed write leaves the state as it was.
///
/// The auto-lock is re-armed here too, still under the lock, so the timer
/// always runs the value the file ends up holding: two overlapping changes
/// persist in one order and would otherwise be allowed to arm in the other.
/// The tray menu is relabelled the same way and for the same reason: it is the
/// one piece of UI i18next cannot reach, so Rust has to be told the language
/// changed.
pub fn set(app: &AppHandle, patch: &Value) -> Result<Settings> {
    let state = app.state::<SettingsState>();
    let mut guard = state.0.lock().unwrap();
    let merged = merge(&guard, patch)?;
    storage::write_settings(app, &serde_json::to_string_pretty(&merged)?)?;
    if merged.autolock_secs != guard.autolock_secs {
        crate::autolock::set_timeout(app, merged.autolock_secs);
    }
    // Handed the new choice rather than left to read it back: this still holds
    // the lock the settings live behind.
    #[cfg(desktop)]
    if merged.locale != guard.locale {
        crate::tray::relabel(app, merged.locale.as_deref());
    }
    *guard = merged.clone();
    Ok(merged)
}

fn merge(base: &Settings, patch: &Value) -> Result<Settings> {
    let mut json = serde_json::to_value(base)?;
    if let (Some(target), Some(fields)) = (json.as_object_mut(), patch.as_object()) {
        for (key, value) in fields {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(serde_json::from_value::<Settings>(json)?.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_what_a_fresh_install_shows() {
        let settings = Settings::default();
        assert_eq!(settings.autolock_secs, 60);
        assert_eq!(settings.clipboard_timeout_ms, 30_000);
        assert_eq!(settings.date_format, "MM/DD/YYYY");
        assert_eq!(settings.sort, "recent");
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.accent, "ink");
        assert_eq!(settings.locale, None);
        assert!(!settings.breach_check);
        assert_eq!(settings.generator, GeneratorDefaults::default());
        assert_eq!(settings.generator.length, 20);
        assert!(!settings.browser.enabled);
    }

    #[test]
    fn round_trips_through_camel_case_json() {
        let settings = Settings {
            locale: Some("de-DE".into()),
            breach_check: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"clipboardTimeoutMs\""));
        assert!(json.contains("\"excludeSimilarCharacters\""));
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn a_partial_or_junk_file_degrades_to_the_defaults() {
        // `#[serde(default)]` on the struct: a file written by an older build
        // keeps whatever it does carry and fills the rest in.
        let partial: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(partial.theme, "dark");
        assert_eq!(partial.autolock_secs, 60);

        assert!(serde_json::from_str::<Settings>("{").is_err());
    }

    #[test]
    fn a_patch_overwrites_only_the_keys_it_names() {
        let base = Settings::default();
        let merged = merge(
            &base,
            &serde_json::json!({ "autolockSecs": 900, "locale": "fr-FR" }),
        )
        .unwrap();

        assert_eq!(merged.autolock_secs, 900);
        assert_eq!(merged.locale, Some("fr-FR".into()));
        assert_eq!(merged.clipboard_timeout_ms, base.clipboard_timeout_ms);
        assert_eq!(merged.generator, base.generator);
    }

    #[test]
    fn a_generator_patch_replaces_the_group() {
        let merged = merge(
            &Settings::default(),
            &serde_json::json!({
                "generator": {
                    "length": 32,
                    "numbers": true,
                    "symbols": false,
                    "uppercase": true,
                    "exclude": "",
                    "excludeSimilarCharacters": true
                }
            }),
        )
        .unwrap();

        assert_eq!(merged.generator.length, 32);
        assert!(!merged.generator.symbols);
        assert!(merged.generator.exclude_similar_characters);
    }

    #[test]
    fn clearing_the_locale_goes_back_to_following_the_os() {
        let pinned = Settings {
            locale: Some("uk-UA".into()),
            ..Settings::default()
        };
        let merged = merge(&pinned, &serde_json::json!({ "locale": null })).unwrap();
        assert_eq!(merged.locale, None);
    }

    // The value handed out is the value the timer runs, so a hand-edited file
    // cannot show one timeout and enforce another.
    #[test]
    fn the_auto_lock_is_normalised_on_the_way_in() {
        let zero = merge(
            &Settings::default(),
            &serde_json::json!({ "autolockSecs": 0 }),
        )
        .unwrap();
        assert_eq!(zero.autolock_secs, DEFAULT_AUTOLOCK_SECS);

        let week = merge(
            &Settings::default(),
            &serde_json::json!({ "autolockSecs": 7 * 24 * 60 * 60 }),
        )
        .unwrap();
        assert_eq!(week.autolock_secs, MAX_AUTOLOCK_SECS);

        let stored: Settings = serde_json::from_str(r#"{"autolockSecs":0}"#).unwrap();
        assert_eq!(stored.normalize().autolock_secs, DEFAULT_AUTOLOCK_SECS);
    }

    // The clipboard delay is armed as a `Duration` on an `Instant`, which
    // panics if it overflows; a hand-edited file must not be able to reach it.
    #[test]
    fn the_clipboard_timeout_is_normalised_on_the_way_in() {
        let huge = merge(
            &Settings::default(),
            &serde_json::json!({ "clipboardTimeoutMs": u64::MAX }),
        )
        .unwrap();
        assert_eq!(huge.clipboard_timeout_ms, MAX_CLIPBOARD_MS);

        // Zero is "never clear", not a timeout to cap or replace.
        let never: Settings = serde_json::from_str(r#"{"clipboardTimeoutMs":0}"#).unwrap();
        assert_eq!(never.normalize().clipboard_timeout_ms, 0);
    }
}
