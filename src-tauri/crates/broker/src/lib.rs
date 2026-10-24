//! The browser extension host without the app around it, so the desktop app,
//! a headless broker and a server can all link the same code: the sealed
//! channel of the KeePassXC-Browser protocol (`protocol`), the native
//! messaging framing (`frame`), what the extension may ask for and the
//! [`actions::Host`] a vault answers it through (`actions`), the passkey
//! ceremonies (`passkeys`), where the socket is (`socket`), and the
//! lockbox-side functions every host shares (`lockbox`). Nothing here knows
//! Tauri, a window or a session; the app's `browser` module is one host of it.

pub mod actions;
pub mod frame;
pub mod lockbox;
pub mod passkeys;
pub mod protocol;
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
