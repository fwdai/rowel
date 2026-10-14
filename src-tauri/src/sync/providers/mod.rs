//! One directory per cloud provider: its REST surface, its [`super::VaultStore`]
//! and its OAuth client. Nothing in here decides which name to read or write —
//! that is the core's (see [`super::store`]).

pub mod dropbox;
pub mod gdrive;
pub mod icloud;

/// The first deep-link scheme in this build's `plugins.deep-link.mobile`
/// configuration that starts with `prefix`. A phone build registers one scheme
/// per provider it can sign in with (a reversed Google client id, Dropbox's
/// `db-<APP_KEY>`), injected at build time (`scripts/release-ios.sh`), so each
/// provider picks out its own rather than taking the first.
#[cfg(mobile)]
pub(crate) fn deep_link_scheme(app: &tauri::AppHandle, prefix: &str) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("deep-link")
        .and_then(|c| c.get("mobile")?.as_array())
        .and_then(|domains| {
            domains.iter().find_map(|d| {
                d.get("scheme")?
                    .as_array()?
                    .iter()
                    .filter_map(|s| s.as_str())
                    .find(|s| s.starts_with(prefix))
                    .map(str::to_owned)
            })
        })
}
