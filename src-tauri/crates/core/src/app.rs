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

/// The macOS App Group the desktop app shares with its Safari web extension
/// (`app.rowel.desktop.safari`, a sandboxed app extension). Its container,
/// `~/Library/Group Containers/<this>`, is the one directory both can reach,
/// so the browser host's socket lives there (`browser::socket_dir`).
///
/// Not [`APP_GROUP`]: a `group.`-prefixed id is iOS-style, and on macOS 15+
/// it is gated by a provisioning profile, which the Developer ID profile does
/// not carry. A macOS-style id — the team id, a dot, then any name — is
/// granted by the signature's team id alone. Granted by the
/// `com.apple.security.application-groups` entitlement on both
/// (`src-tauri/Entitlements.plist`, `src-tauri/gen/safari/project.yml`).
pub const DESKTOP_APP_GROUP: &str = "UFBL3F444A.app.rowel.desktop";

/// The vault's directory inside the App Group container: the app data dir's
/// own name, so the layout under it is the one the app always had.
pub const APP_GROUP_DATA_DIR: &str = "app.rowel.mobile";
