//! One directory per cloud provider: its REST surface, its [`super::VaultStore`]
//! and its OAuth client. Nothing in here decides which name to read or write —
//! that is the core's (see [`super::store`]).

// Unused until PR 2 wires its `Store` and `OAuthConfig` into the core.
#[allow(dead_code)]
pub mod dropbox;
pub mod gdrive;
