//! User facing application name — the single place the backend spells it.
//!
//! Identifiers that happen to share the name (`productName`/bundle id in
//! `tauri.conf.json`, the Google Drive folder `sync::layout::ROOT_FOLDER`, the HTTP
//! user agent) are deliberately separate literals: they are persisted or
//! observed outside the app, so a change to them is a migration, not a rename.
//! The frontend has its own copy of this name in `src/lib/app.ts`.

pub const APP_NAME: &str = "Rowel";
