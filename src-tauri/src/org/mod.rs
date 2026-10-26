//! Org lockboxes: the parts of the app that speak to a company's Rowel Server
//! as a *member*, rather than as a sync client.
//!
//! A member signs in to the server once per install ([`client`]: an OAuth2
//! authorization-code sign-in the server fronts, federating to the company's
//! identity provider on its own side, so the app never meets the provider;
//! [`connect`] is the desktop flow that runs it), enrolls this device's X25519
//! key with it ([`enrollment`]), and keeps the session and the device's
//! standing sealed under the app key ([`connections`]). Every org lockbox the
//! server then hands the member is one more non-primary workspace
//! ([`lockboxes`]): its vault id is the server's lockbox id, its pack syncs
//! through `sync::providers::rowel` under that session, and its master is
//! unsealed by the enrollment key rather than derived from a password
//! (`KdfParams::External`). While the vault is open, the server's approvals
//! for this member reach the app's own grant dialog ([`approvals`]).
//!
//! The pieces take `(root, app_key)` and talk HTTP; the places an `AppHandle`
//! appears are wrappers that fetch those two from the running app. The
//! commands over all of it are `commands::org`.

use std::path::Path;

#[cfg(desktop)]
pub(crate) mod approvals;
pub(crate) mod client;
#[cfg(desktop)]
pub(crate) mod connect;
pub(crate) mod connections;
pub(crate) mod enrollment;
#[cfg(test)]
pub(crate) mod fake_server;
pub(crate) mod lockboxes;
#[cfg(test)]
mod tests;

/// Every root-level file sealed under the app key — the server connections
/// and this device's enrollment key — re-sealed under `new` after a change
/// of the primary's key replaced `old` (`appkey::replace_app_key`). Best
/// effort, each on its own: a file that will not move is logged, and goes
/// as stale at the next primary unlock ([`remove_stale`]), after which the
/// member signs in again.
pub(crate) fn reseal_under(root: &Path, old: &[u8], new: &[u8]) {
    if let Err(e) = connections::reseal(root, old, new) {
        log::warn!("could not re-seal the server connections under the new app key: {e}");
    }
    if let Err(e) = enrollment::reseal(root, old, new) {
        log::warn!("could not re-seal the enrollment key under the new app key: {e}");
    }
}

/// The same two files removed where `app_key`, just proved to be the app
/// key, does not open them: they were sealed under one this device no
/// longer has, and nothing else recovers them.
pub(crate) fn remove_stale(root: &Path, app_key: &[u8]) {
    connections::remove_if_stale(root, app_key);
    enrollment::remove_if_stale(root, app_key);
}
