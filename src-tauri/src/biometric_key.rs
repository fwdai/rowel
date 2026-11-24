//! Android's half of the biometric gate ([`GateMode::KeystoreKey`]): the calls
//! into `BiometricKeyPlugin`, the Kotlin class in the Gradle project
//! (`gen/android/app/src/main/java/app/rowel/mobile`) that holds the Android
//! Keystore key and puts up the system prompt.
//!
//! The Keystore and `BiometricPrompt` are Java APIs, so the work is Kotlin's
//! and this module is the wire: four commands, their JSON, and what a refusal
//! means to the rest of the app. The wire is all that is compiled off Android,
//! for the tests.
//!
//! No capability names the plugin, so its commands cannot be invoked from the
//! webview — Tauri checks the ACL for every `plugin:` command before it looks
//! for a handler. Rust is the only caller.
//!
//! Every call blocks its thread until Kotlin answers, and Kotlin runs on
//! Android's UI thread (Tauri dispatches plugin commands through the main
//! looper) — where a prompt stays up for as long as the user takes. So a call
//! must never come from that thread, which would wait on itself. None does: the
//! secure-store calls run on the blocking pool (`commands::blocking`), and the
//! status probe runs inside async commands (`app_status`, `enable_biometric`),
//! on a runtime worker.
//!
//! [`GateMode::KeystoreKey`]: rowel_core::keychain::GateMode::KeystoreKey

use crate::error::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// What `status` answers: whether a strong biometric can authenticate right
/// now, and which sensors the device declares.
///
/// The sensors are hardware the device has, not what the user enrolled or what
/// the prompt will ask for — Android tells an app neither.
#[derive(Deserialize)]
pub struct Status {
    pub available: bool,
    pub fingerprint: bool,
    pub face: bool,
    pub iris: bool,
}

/// The key material on its way to Kotlin, base64 in a JSON string: the plugin
/// bridge carries JSON and nothing else.
#[derive(Serialize)]
struct StoreArgs<'a> {
    material: &'a str,
}

/// The key material on its way back. Deserialized straight into a buffer that
/// is scrubbed when it drops: the string Tauri parsed out of the response is
/// moved into it, not copied. The response text Tauri read out of the JVM to
/// parse is not ours to scrub.
#[derive(Deserialize)]
struct Retrieved {
    material: Zeroizing<String>,
}

fn encode(key: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(STANDARD.encode(key))
}

fn decode(material: &str) -> Result<Zeroizing<Vec<u8>>> {
    STANDARD.decode(material).map(Zeroizing::new).map_err(|e| {
        Error::Other(format!(
            "the biometric key plugin answered with bad base64: {e}"
        ))
    })
}

/// A refusal from Kotlin (`BiometricKey.Reason`) as the app's error.
///
/// `invalidated` is the Keystore key gone for good — a biometric was enrolled,
/// the last one removed or the screen lock turned off, and Android destroyed
/// the key — and Kotlin has already deleted what was left. It reads as `NotFound` like a missing item
/// does everywhere else, which is what clears the enrollment marker
/// (`commands::auth::unenroll_on`).
///
/// The rest — `noneEnrolled`, `lockout`, `unavailable`, `failed`, and anything
/// without a code, which is an exception Tauri caught around the command — leave
/// the enrollment where it is and carry the system's own words for what
/// happened.
fn refusal(code: Option<&str>, message: Option<String>) -> Error {
    match code {
        Some("cancelled") => Error::Cancelled,
        Some("notFound" | "invalidated") => Error::NotFound,
        _ => Error::Other(message.unwrap_or_else(|| "biometric authentication failed".into())),
    }
}

#[cfg(target_os = "android")]
pub use plugin::{init, remove, retrieve, status, store};

#[cfg(target_os = "android")]
mod plugin {
    use super::*;
    use serde::de::DeserializeOwned;
    use std::sync::OnceLock;
    use tauri::plugin::mobile::PluginInvokeError;
    use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
    use tauri::Wry;

    // Set once, when the plugin is set up. A static rather than managed state
    // because `secure_store::Platform` is reached without an `AppHandle`.
    static HANDLE: OnceLock<PluginHandle<Wry>> = OnceLock::new();

    /// The Tauri plugin whose setup instantiates the Kotlin class. Its name is
    /// only what the two sides find each other by.
    pub fn init() -> TauriPlugin<Wry> {
        Builder::new("biometric-key")
            .setup(|_app, api| {
                let handle =
                    api.register_android_plugin("app.rowel.mobile", "BiometricKeyPlugin")?;
                let _ = HANDLE.set(handle);
                Ok(())
            })
            .build()
    }

    fn call<T: DeserializeOwned>(command: &str, payload: impl Serialize) -> Result<T> {
        let handle = HANDLE
            .get()
            .ok_or_else(|| Error::Other("the biometric key plugin is not registered".into()))?;
        handle
            .run_mobile_plugin(command, payload)
            .map_err(|e| match e {
                PluginInvokeError::InvokeRejected(rejected) => {
                    refusal(rejected.code.as_deref(), rejected.message)
                }
                other => Error::Other(other.to_string()),
            })
    }

    pub fn status() -> Result<Status> {
        call("status", ())
    }

    /// Seal `key` under a fresh Keystore key, replacing any earlier
    /// enrollment. Puts up the biometric prompt: the Keystore key needs one for
    /// every use, encrypting included.
    ///
    /// The base64 copy made here is scrubbed. The ones Tauri makes of it on the
    /// way — a `serde_json::Value`, then its serialized text — are not ours to
    /// scrub, and neither is anything on the JVM side but the decoded bytes.
    pub fn store(key: &[u8]) -> Result<()> {
        let material = encode(key);
        call(
            "store",
            StoreArgs {
                material: &material,
            },
        )
    }

    /// Unseal the stored key. Puts up the biometric prompt.
    pub fn retrieve() -> Result<Zeroizing<Vec<u8>>> {
        let retrieved: Retrieved = call("retrieve", ())?;
        decode(&retrieved.material)
    }

    /// Delete the Keystore key and the sealed blob. No prompt, and nothing
    /// stored is not an error.
    pub fn remove() -> Result<()> {
        call("remove", ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_dismissed_prompt_is_a_cancellation() {
        assert!(matches!(
            refusal(Some("cancelled"), Some("Authentication cancelled".into())),
            Error::Cancelled
        ));
    }

    // Both read as the enrollment being gone, which is what un-enrolls.
    #[test]
    fn a_missing_or_invalidated_key_is_not_found() {
        assert!(matches!(refusal(Some("notFound"), None), Error::NotFound));
        assert!(matches!(
            refusal(
                Some("invalidated"),
                Some("Key permanently invalidated".into())
            ),
            Error::NotFound
        ));
    }

    // A lockout passes, sensors come back, a fingerprint can be enrolled: none
    // of them may read as `NotFound` and cost the user their enrollment, or as
    // `Cancelled` and be swallowed by the lock screen.
    #[test]
    fn every_other_refusal_keeps_the_enrollment_and_its_message() {
        for code in ["noneEnrolled", "lockout", "unavailable", "failed", "new"] {
            let error = refusal(Some(code), Some("Too many attempts".into()));
            assert!(
                matches!(&error, Error::Other(m) if m == "Too many attempts"),
                "{code}"
            );
        }
        // An exception Tauri caught around the command: a message, no code.
        assert!(matches!(
            refusal(None, Some("KeyStoreException".into())),
            Error::Other(m) if m == "KeyStoreException"
        ));
        assert!(matches!(refusal(None, None), Error::Other(_)));
    }

    #[test]
    fn key_material_round_trips_through_the_wire_encoding() {
        let key = crate::crypto::hash_secret("hunter2").into_bytes();
        let sent = serde_json::to_value(StoreArgs {
            material: &encode(&key),
        })
        .unwrap();
        // What Kotlin sends back is the same shape it was sent.
        let retrieved: Retrieved = serde_json::from_value(sent).unwrap();
        assert_eq!(&*decode(&retrieved.material).unwrap(), &key);
    }

    #[test]
    fn the_wire_names_are_the_ones_kotlin_uses() {
        assert_eq!(
            serde_json::to_value(StoreArgs { material: "a2V5" }).unwrap(),
            json!({ "material": "a2V5" })
        );
        let status: Status = serde_json::from_value(json!({
            "available": true,
            "fingerprint": true,
            "face": false,
            "iris": false,
        }))
        .unwrap();
        assert!(status.available && status.fingerprint && !status.face && !status.iris);
    }

    #[test]
    fn material_that_is_not_base64_is_an_error() {
        assert!(decode("not base64!").is_err());
    }
}
