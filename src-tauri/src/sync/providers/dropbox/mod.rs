//! Dropbox, with **App folder** access.
//!
//! The app is registered as an App folder app, so Dropbox gives it one folder,
//! `/Apps/<app name>/` in the user's account, and every path the API takes or
//! returns is relative to it. The app cannot see, list or touch anything
//! outside. That folder stands in for `layout::ROOT_FOLDER`, so the layout
//! inside it is just:
//!
//! ```text
//! /Vaults/<vault-id>.rowel      the live pack (layout::vault_file_name)
//! /Vaults/<vault-id>.deleted    the deletion marker (layout::deleted_marker_name)
//! ```
//!
//! Dropbox paths are unique and case-insensitive, so unlike Drive there are no
//! same-name siblings to choose between. Entries also carry a stable `id:...`
//! that every call here accepts in place of a path.
//!
//! - [`api`]: the v2 REST calls (list, metadata, download, upload, delete).
//! - [`oauth`]: endpoints, consent parameters, app key and redirect scheme.

pub mod api;
pub mod oauth;
