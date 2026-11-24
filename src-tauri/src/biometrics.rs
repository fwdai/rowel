//! Biometric-unlock seam. In-session re-unlock only (parity with legacy Touch
//! ID): prompt the OS biometric dialog; the master key never leaves memory.

use crate::error::{Error, Result};

// LocalAuthentication is the same framework on both Apple platforms: Touch ID /
// Face ID come out of the same `LAContext` policy evaluation.
#[cfg(target_vendor = "apple")]
mod imp {
    use super::*;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LABiometryType, LAContext, LAError, LAPolicy};
    use std::sync::mpsc;

    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

    // `biometryType` is only populated once the context has been asked whether
    // it can evaluate the policy, so the evaluation answers both halves at once
    // — which is why they are asked for together.
    pub fn probe() -> super::Probe {
        let ctx = unsafe { LAContext::new() };
        if unsafe { ctx.canEvaluatePolicy_error(POLICY) }.is_err() {
            return super::Probe::UNAVAILABLE;
        }
        let biometry = unsafe { ctx.biometryType() };
        let kind = if biometry == LABiometryType::FaceID {
            super::FACE
        } else if biometry == LABiometryType::TouchID {
            super::TOUCH
        } else {
            // Optic ID and anything Apple adds later: we have no name for it, so
            // say nothing rather than name the wrong gate.
            super::NONE
        };
        super::Probe {
            available: true,
            kind,
        }
    }

    pub fn authenticate() -> Result<()> {
        let ctx = unsafe { LAContext::new() };
        let reason = NSString::from_str("Confirm your identity");
        // evaluatePolicy fires its reply on an internal queue; block on a channel.
        // The error code rides along: it is what tells a prompt the user
        // dismissed from one that failed.
        let (tx, rx) = mpsc::channel();
        let reply = RcBlock::new(move |success: Bool, err: *mut NSError| {
            // The block owns nothing about `err` beyond this call, so the code
            // is read out now rather than the pointer sent across.
            let code = (!err.is_null()).then(|| LAError(unsafe { (*err).code() }));
            let _ = tx.send((success.as_bool(), code));
        });
        unsafe { ctx.evaluatePolicy_localizedReason_reply(POLICY, &reason, &reply) };
        match rx.recv() {
            Ok((true, _)) => Ok(()),
            Ok((false, Some(code))) if is_cancel(code) => Err(Error::Cancelled),
            _ => Err(Error::Other("biometric authentication failed".into())),
        }
    }

    // The ways a prompt ends without an answer rather than with a wrong one:
    // the user pressed Cancel or "Enter password", the system took the screen
    // (a call, a lock), or the app itself withdrew the prompt. None of them
    // says anything about the user's finger or face.
    fn is_cancel(code: LAError) -> bool {
        matches!(
            code,
            LAError::UserCancel
                | LAError::UserFallback
                | LAError::SystemCancel
                | LAError::AppCancel
        )
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::*;
    use windows::core::HSTRING;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };

    // `join` is windows-rs 0.62's name for what used to be `get`: block the
    // calling thread until the WinRT async operation completes.
    pub fn probe() -> super::Probe {
        let available = UserConsentVerifier::CheckAvailabilityAsync()
            .and_then(|op| op.join())
            .map(|a| a == UserConsentVerifierAvailability::Available)
            .unwrap_or(false);
        if available {
            super::Probe {
                available,
                kind: super::TOUCH,
            }
        } else {
            super::Probe::UNAVAILABLE
        }
    }

    pub fn authenticate() -> Result<()> {
        let message = HSTRING::from("Confirm your identity");
        let result =
            UserConsentVerifier::RequestVerificationAsync(&message).and_then(|op| op.join());
        match result {
            Ok(UserConsentVerificationResult::Verified) => Ok(()),
            // Dismissed, not refused: the user closed the Hello prompt. The
            // same distinction `secure_store` draws for the key credential.
            Ok(UserConsentVerificationResult::Canceled) => Err(Error::Cancelled),
            _ => Err(Error::Other("biometric authentication failed".into())),
        }
    }
}

// Android asks the Kotlin side (`biometric_key`), which asks `BiometricManager`
// whether a strong biometric can authenticate — the only class the Keystore
// accepts for the key the vault key is sealed under.
#[cfg(target_os = "android")]
mod imp {
    use super::*;

    pub fn probe() -> super::Probe {
        match crate::biometric_key::status() {
            Ok(status) if status.available => super::Probe {
                available: true,
                kind: super::android_kind(&status),
            },
            Ok(_) => super::Probe::UNAVAILABLE,
            Err(e) => {
                log::warn!("could not ask Android about biometrics: {e}");
                super::Probe::UNAVAILABLE
            }
        }
    }

    // Nothing on Android verifies and then reads: the prompt belongs to the
    // Keystore key itself (`GateMode::KeystoreKey`), so there is no bare check
    // to run.
    #[allow(dead_code)]
    pub fn authenticate() -> Result<()> {
        Err(Error::Other(
            "a bare biometric check is not supported on this platform".into(),
        ))
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "windows", target_os = "android")))]
mod imp {
    use super::*;

    pub fn probe() -> super::Probe {
        super::Probe::UNAVAILABLE
    }

    #[allow(dead_code)]
    pub fn authenticate() -> Result<()> {
        Err(Error::Other(
            "biometrics not supported on this platform".into(),
        ))
    }
}

// What the UI calls the gate. Kept as the three wire strings the frontend's
// `BiometryType` union already names, so neither side has a mapping table.

// Only Apple and Android can report a face — on Windows the gate has always
// been a fingerprint (Windows Hello's).
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "android")),
    allow(dead_code)
)]
const FACE: &str = "face";
#[cfg_attr(
    not(any(target_vendor = "apple", target_os = "windows", target_os = "android")),
    allow(dead_code)
)]
const TOUCH: &str = "touch";
const NONE: &str = "none";

// What to call the gate on an Android device. Android does not say which
// biometric its prompt will ask for, or which ones the user enrolled — only
// which sensors the device declares. So the gate is named only when there is
// exactly one it can be; a device with several, or with one this has no name
// for, gets none, and the UI says "biometrics".
#[cfg(any(target_os = "android", test))]
fn android_kind(status: &crate::biometric_key::Status) -> &'static str {
    match (status.fingerprint, status.face, status.iris) {
        (true, false, false) => TOUCH,
        (false, true, false) => FACE,
        _ => NONE,
    }
}

/// What this device's biometric gate is, and whether it can be used at all.
///
/// One value rather than two calls: on Apple both answers fall out of a single
/// `LAContext` policy evaluation, which is the expensive part — and the launch
/// probe wants both.
pub struct Probe {
    /// The hardware is there and the OS will evaluate the policy. Not the
    /// opt-in: whether a key is enrolled is `storage::biometric_enrolled`.
    pub available: bool,
    /// `"face"`, `"touch"` or `"none"`, so the UI can name the gate rather than
    /// guess it from the platform (a Touch ID iPad is not Face ID).
    pub kind: &'static str,
}

impl Probe {
    const UNAVAILABLE: Self = Self {
        available: false,
        kind: NONE,
    };
}

pub fn probe() -> Probe {
    imp::probe()
}

pub fn is_available() -> bool {
    probe().available
}

// The verify-then-read gate, used by macOS/iOS in `GateMode::Prompt` only
// (unentitled builds, where the OS cannot enforce biometry on keychain read).
// Windows no longer calls it: there the Hello prompt is the key-credential
// signature itself (`GateMode::HelloKey`), not a check that merely precedes a
// read. Android never did, for the same reason (`GateMode::KeystoreKey`).
// Linux never calls it — the secure store is unsupported outright.
#[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
pub fn authenticate() -> Result<()> {
    imp::authenticate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biometric_key::Status;

    fn sensors(fingerprint: bool, face: bool, iris: bool) -> Status {
        Status {
            available: true,
            fingerprint,
            face,
            iris,
        }
    }

    #[test]
    fn an_android_device_with_one_sensor_names_it() {
        assert_eq!(android_kind(&sensors(true, false, false)), TOUCH);
        assert_eq!(android_kind(&sensors(false, true, false)), FACE);
    }

    // The prompt picks, and does not say which: naming either would be a guess.
    #[test]
    fn an_android_device_with_several_sensors_names_none() {
        assert_eq!(android_kind(&sensors(true, true, false)), NONE);
        assert_eq!(android_kind(&sensors(true, false, true)), NONE);
    }

    // An iris scanner alone, or a sensor Android has no feature flag for.
    #[test]
    fn an_android_sensor_without_a_name_names_none() {
        assert_eq!(android_kind(&sensors(false, false, true)), NONE);
        assert_eq!(android_kind(&sensors(false, false, false)), NONE);
    }
}
