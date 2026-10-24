//! What the host has to tell an extension that did not ask: a fill grant an
//! agent obtained for a page the extension may have open, and a grant that has
//! ended and whose sites are to be signed out of.
//!
//! A signal carries an item's title, which is personal, so it never travels in
//! the clear the way `database-locked` does. The host pushes [`ROWEL_SIGNAL`],
//! which says nothing but "ask", through the same unsolicited channel; the
//! extension answers it with a sealed `get-signals`, and the queued signals come
//! back inside the sealed reply. One request, one reply stays the rule for
//! everything that carries content.

use serde::{Deserialize, Serialize};

use crate::grant::VisibleItem;

/// The clear, content-free wake-up `server::signal` pushes beside
/// `database-locked`; the extension answers it with a sealed `get-signals`.
pub const ROWEL_SIGNAL: &[u8] = br#"{"action":"rowel-signal"}"#;

/// One queued signal, tagged by `signal` on the wire (`fill-grant`,
/// `grant-ended`), its fields camelCase like every other message. There is no
/// `set-agent-mode`: the extension derives agent mode from the fill grants it
/// holds, on with the first and off with the last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "signal",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Signal {
    /// An agent's fill grant is active for `origin`: the extension with a tab
    /// there asks `get-logins { url, grantId }` and fills `item`.
    FillGrant {
        grant_id: String,
        origin: String,
        item: VisibleItem,
        auto_submit: bool,
        expires_at_ms: i64,
    },
    /// The grant ended, however it did: the extension forgets it and signs
    /// out of `origins`.
    GrantEnded {
        grant_id: String,
        origins: Vec<String>,
    },
}
