//! What the biometric-gated vault key is stored under in the OS secure store,
//! and how an enrolment records the gate it chose. Persisted names, read by the
//! app that writes the item and by the iOS AutoFill extension that reads it
//! back — so they are spelled once, here. The store itself is the app's
//! (`secure_store.rs`); the extension reads the item through the Security
//! framework in Swift.

/// The item's service.
pub const SERVICE: &str = "app.rowel.desktop.vault";
/// The account of the item stored behind [`GateMode::Protected`].
pub const ACCOUNT: &str = "master-key";
/// Separate account for the verify-then-read item on Apple platforms. The two
/// modes carry different access control, so they must never be able to resolve
/// each other's item: a distinct account makes a cross-mode read a clean
/// `NotFound` rather than an item read under the wrong gate.
pub const ACCOUNT_PROMPT: &str = "master-key-prompt";

/// How an enrolled key is gated. Recorded at enrollment; never re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    /// OS-enforced on read (Apple data-protection keychain + `SecAccessControl`).
    Protected,
    /// App-enforced: explicit biometric prompt, then a plain credential-store read.
    Prompt,
    /// Windows: sealed under a Windows Hello key-credential signature, so the
    /// stored blob cannot be opened without passing the Hello prompt.
    HelloKey,
}

impl GateMode {
    /// The marker value persisted next to the enrollment flag
    /// ([`crate::layout::BIOMETRIC_FILE`]). Stable on disk — changing these
    /// strings orphans existing enrollments.
    pub fn as_marker(self) -> &'static str {
        match self {
            Self::Protected => "protected",
            Self::Prompt => "prompt",
            Self::HelloKey => "hello-key",
        }
    }

    /// Read a persisted marker. Anything unrecognised — including the legacy
    /// `"1"` marker written before modes existed — reads as the mode that build
    /// would have used, so an old enrollment keeps working (or fails loudly)
    /// rather than being reinterpreted under a gate it was never stored behind.
    pub fn from_marker(marker: &str) -> Self {
        match marker.trim() {
            "prompt" => Self::Prompt,
            "protected" => Self::Protected,
            "hello-key" => Self::HelloKey,
            _ => Self::LEGACY,
        }
    }

    // Pre-mode enrollments: macOS only ever wrote the protected item, every
    // other platform only ever wrote the verify-then-read one. iOS had no
    // pre-mode build at all, but shares macOS' enrollment path.
    #[cfg(target_vendor = "apple")]
    const LEGACY: Self = Self::Protected;
    #[cfg(not(target_vendor = "apple"))]
    const LEGACY: Self = Self::Prompt;
}

#[cfg(test)]
mod tests {
    use super::GateMode;

    #[test]
    fn gate_mode_markers_round_trip() {
        for mode in [GateMode::Protected, GateMode::Prompt, GateMode::HelloKey] {
            assert_eq!(GateMode::from_marker(mode.as_marker()), mode);
        }
        // A pre-mode marker reads as whatever that build actually wrote.
        assert_eq!(GateMode::from_marker("1"), GateMode::LEGACY);
        assert_eq!(GateMode::from_marker(""), GateMode::LEGACY);
    }
}
