//! The browser extension host without the app around it, so the desktop app,
//! a headless broker and a server can all link the same code: the sealed
//! channel of the KeePassXC-Browser protocol (`protocol`), the native
//! messaging framing (`frame`), what the extension may ask for and the
//! [`actions::Host`] a vault answers it through (`actions`), the passkey
//! ceremonies (`passkeys`), where the socket is (`socket`), the lockbox-side
//! functions every host shares (`lockbox`), and what agents get: grants
//! (`grant`), the policy that issues them (`policy`), the placeholder they name
//! a secret by (`reference`), the signals that wake an extension for an
//! agent's grant (`signal`) and the chained log of what was done with them
//! (`audit`). The client half of the channel (`client`) is here too, so the
//! CLI speaks it with the same code the tests drive the host with.
//! Nothing here knows Tauri, a window or a session; the app's `browser` module
//! is one host of it.

pub mod actions;
pub mod audit;
pub mod client;
pub mod frame;
pub mod grant;
pub mod lockbox;
pub mod passkeys;
pub mod policy;
pub mod protocol;
pub mod reference;
pub mod signal;
pub use signal::{Signal, ROWEL_SIGNAL};
// Desktop only, as the app's `browser` module is: it is where `interprocess`
// and `dirs` are linked, and neither has a place on a phone.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod socket;
// The scripted host and extension the tests run against; `test-support` lends
// them to the app's own tests.
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
#[cfg(test)]
mod tests;
