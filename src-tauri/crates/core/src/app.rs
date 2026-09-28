//! User facing application name — the single place the backend spells it.
//!
//! Identifiers that happen to share the name (`productName`/bundle id in
//! `tauri.conf.json`, the Google Drive folder `sync::layout::ROOT_FOLDER`, the HTTP
//! user agent) are deliberately separate literals: they are persisted or
//! observed outside the app, so a change to them is a migration, not a rename.
//! The frontend has its own copy of this name in `src/lib/app.ts`.

pub const APP_NAME: &str = "Rowel";

/// The iOS App Group the app shares with its AutoFill extension
/// (`app.rowel.mobile.autofill`). It names two things at once: the file
/// container the vault lives in (so the extension can open it), and the
/// keychain access group the biometric key is stored under (so the extension
/// can read it) — an app-group id is a valid `kSecAttrAccessGroup` on iOS
/// without a keychain-sharing entitlement of its own. Granted by the
/// `com.apple.security.application-groups` entitlement on both targets.
pub const APP_GROUP: &str = "group.app.rowel.mobile";

/// The vault's directory inside the App Group container: the app data dir's
/// own name, so the layout under it is the one the app always had.
pub const APP_GROUP_DATA_DIR: &str = "app.rowel.mobile";
