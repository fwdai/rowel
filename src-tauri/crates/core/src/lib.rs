//! The vault core: the entry model, the crypto, the SQLCipher store and the
//! passkey authenticator, with the importers and exporters that read and write
//! the same entries. No Tauri and no webview, so a process that is not the app
//! — the iOS AutoFill extension — can open the vault too.

pub mod app;
pub mod cards;
pub mod crypto;
pub mod error;
pub mod import;
pub mod models;
// How a stored `otp` value is spelled — read by the generator, the importers
// and the exporters alike, so it sits below all three.
pub mod otp;
// Owner-only file permissions, on Unix and Windows alike.
pub mod owner_only;
// The WebAuthn authenticator core.
pub mod passkey;
pub mod store;
