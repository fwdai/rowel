//! Dropbox OAuth 2 constants. Data only: the authorization-code + PKCE flow
//! itself is the shared one, fed these values.
//!
//! The app is a public client: it has an app key and no secret, and proves
//! itself with the PKCE verifier instead (`code_challenge_method=S256`). The
//! code exchange sends `client_id` and `code_verifier`, a refresh sends
//! `grant_type=refresh_token`, `refresh_token` and `client_id`; neither ever
//! sends a client secret.
//!
//! Verified against the Dropbox docs (2026-10-12, recorded in
//! `docs/sync-providers-design.md` §3):
//! - endpoints, `token_access_type=offline`, space-separated `scope`, PKCE
//!   without a secret: <https://docs.dropboxapi.com/dropbox-api/docs/oauth>;
//! - redirect URIs must match a registered one exactly, port included, with
//!   no wildcard ports, hence [`LOOPBACK_PORTS`];
//! - the mobile redirect is what SwiftyDropbox sends (`OAuthImpl.swift`).

/// Consent page.
pub const AUTH_URL: &str = "https://www.dropbox.com/oauth2/authorize";
/// Code exchange and refresh.
pub const TOKEN_URL: &str = "https://api.dropboxapi.com/oauth2/token";

/// What the app may do inside its app folder: list and stat, read, write.
/// `sharing.write` joins when shares do.
pub const SCOPE: &str = "files.metadata.read files.content.read files.content.write";

/// Query pairs the consent URL needs beyond the standard OAuth/PKCE ones
/// (`client_id`, `response_type=code`, `redirect_uri`, `state`,
/// `code_challenge`, `code_challenge_method=S256`). `offline` is what gets a
/// long-lived refresh token alongside the short-lived (~4 h) access token.
/// Dropbox does not rotate refresh tokens.
pub const CONSENT_PARAMS: &[(&str, &str)] = &[("token_access_type", "offline"), ("scope", SCOPE)];

/// Where the app key comes from.
pub const APP_KEY_ENV: &str = "DROPBOX_APP_KEY";

/// The app key: `DROPBOX_APP_KEY` at run time, else the value baked in at
/// build time, else `None` (Dropbox sync not configured in this build). The
/// key is public; it identifies the app, it does not authenticate it.
pub fn app_key() -> Option<String> {
    std::env::var(APP_KEY_ENV)
        .ok()
        .or_else(|| option_env!("DROPBOX_APP_KEY").map(String::from))
        .filter(|key| !key.is_empty())
}

// --- desktop: loopback redirect ---

/// Dropbox matches `redirect_uri` exactly against the URIs registered in the
/// App Console, port included ("even localhost must be listed"); there is no
/// wildcard or any-port loopback rule like Google's. So the desktop flow
/// cannot bind port 0: it tries these in order and listens on the first that
/// binds. **Every one of them must be registered in the App Console** as
/// `http://127.0.0.1:<port>/auth/callback`, or consent fails with an
/// "invalid redirect_uri" page.
pub const LOOPBACK_PORTS: [u16; 3] = [53682, 53683, 53684];
/// The loopback host. An IP literal, not `localhost`, so the redirect cannot
/// resolve to IPv6 while the listener is on IPv4 (or the other way round).
pub const LOOPBACK_HOST: &str = "127.0.0.1";
/// The callback path on the loopback listener.
pub const LOOPBACK_CALLBACK: &str = "/auth/callback";

/// `http://127.0.0.1:<port>/auth/callback`, for one of [`LOOPBACK_PORTS`].
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://{LOOPBACK_HOST}:{port}{LOOPBACK_CALLBACK}")
}

// --- mobile: custom-scheme redirect ---

/// The URL scheme the official Dropbox SDKs register for the redirect on
/// mobile: `db-<APP_KEY>`. It is per app, so no other Dropbox app's redirect
/// lands on ours. Only the OS-level scheme needs registering (in
/// `tauri.ios.conf.json`'s deep-link schemes); that Dropbox accepts it without
/// an App Console entry is inferred from the SDK working, not documented.
pub const MOBILE_SCHEME_PREFIX: &str = "db-";

/// What follows the scheme in the mobile redirect URI. SwiftyDropbox sends
/// `db-<APP_KEY>://2/token` as `redirect_uri` on both the authorize call and
/// the code exchange.
pub const MOBILE_REDIRECT_PATH: &str = "://2/token";

/// Where SwiftyDropbox lands a cancelled consent: `db-<APP_KEY>://2/cancel`.
/// A deep link on our scheme with this path is "the user said no".
pub const MOBILE_CANCEL_PATH: &str = "://2/cancel";

/// `db-<APP_KEY>`, the scheme to list in `tauri.ios.conf.json`'s deep-link
/// schemes.
pub fn mobile_scheme(app_key: &str) -> String {
    format!("{MOBILE_SCHEME_PREFIX}{app_key}")
}

/// `db-<APP_KEY>://2/token`, the `redirect_uri` the mobile consent request
/// sends and the code exchange repeats.
pub fn mobile_redirect_uri(app_key: &str) -> String {
    format!("{}{MOBILE_REDIRECT_PATH}", mobile_scheme(app_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_asks_for_a_refresh_token_and_the_file_scopes() {
        assert!(CONSENT_PARAMS.contains(&("token_access_type", "offline")));
        let (_, scope) = CONSENT_PARAMS.iter().find(|(k, _)| *k == "scope").unwrap();
        let scopes: Vec<&str> = scope.split(' ').collect();
        assert_eq!(
            scopes,
            [
                "files.metadata.read",
                "files.content.read",
                "files.content.write"
            ]
        );
    }

    #[test]
    fn the_desktop_redirect_is_a_fixed_registered_loopback_uri() {
        assert_eq!(
            loopback_redirect_uri(53682),
            "http://127.0.0.1:53682/auth/callback"
        );
        // Distinct, so a busy port always leaves another one to try.
        let mut ports = LOOPBACK_PORTS.to_vec();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(ports.len(), LOOPBACK_PORTS.len());
        assert!(!LOOPBACK_PORTS.contains(&0), "port 0 cannot be registered");
    }

    #[test]
    fn the_mobile_redirect_is_on_the_apps_own_scheme() {
        assert_eq!(mobile_scheme("abc123"), "db-abc123");
        assert_eq!(mobile_redirect_uri("abc123"), "db-abc123://2/token");
        let url = url::Url::parse(&mobile_redirect_uri("abc123")).unwrap();
        assert_eq!(url.scheme(), "db-abc123");
        assert_eq!(url.host_str(), Some("2"));
        assert_eq!(url.path(), "/token");
    }
}
