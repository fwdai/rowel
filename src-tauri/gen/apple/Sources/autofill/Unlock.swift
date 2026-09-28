import Foundation
import LocalAuthentication
import Security

/// Opens the active vault the way the app's own Face ID unlock does: one
/// biometric check, then the app key read from the keychain item the app
/// enrolled, in the App Group's access group. The key is handed to Rust as
/// bytes; where the vault is and how the key opens it is `rowel-autofill`'s.
enum Unlock {
    enum Failure: LocalizedError {
        /// iOS handed over no App Group container: the extension was built
        /// without the entitlement.
        case noContainer
        /// No key in the keychain group: Face ID unlock was turned off, the
        /// enrolled faces or fingers changed (the item goes with them), or the
        /// app has not unlocked since it moved the item into the group.
        case noKey
        case keychain(OSStatus)

        var errorDescription: String? {
            switch self {
            case .noContainer: "Rowel's shared storage is not available."
            case .noKey: "Unlock Rowel with Face ID or Touch ID once, then try again."
            case let .keychain(status):
                SecCopyErrorMessageString(status, nil) as String? ?? "Keychain error \(status)."
            }
        }
    }

    /// The open vault, after a biometric prompt. Not isolated to the main
    /// actor, so the keychain read and the database open run off it.
    static func vault() async throws -> Vault {
        let group = appGroup()
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
        else { throw Failure.noContainer }
        // Before any prompt: with no vault, or none Face ID can open, there
        // is nothing to ask the user for.
        let location = try vaultLocation(container: container.path)

        // The item's access control (biometry, current set) accepts this
        // context once it has been evaluated, so the read below puts up no
        // second sheet; and the same check is the whole gate for an item the
        // app stored without one (the verify-then-read enrolment).
        let context = LAContext()
        try await context.evaluatePolicy(
            .deviceOwnerAuthenticationWithBiometrics,
            localizedReason: "Unlock Rowel to fill a password."
        )
        let key = try readKey(location, group: group, context: context)
        return try openVault(location: location, key: key)
    }

    private static func readKey(_ location: VaultLocation, group: String, context: LAContext) throws -> Data {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: location.keychainService,
            kSecAttrAccount: location.keychainAccount,
            kSecAttrAccessGroup: group,
            kSecUseAuthenticationContext: context,
            kSecReturnData: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let key = item as? Data else { throw Failure.keychain(errSecInternalError) }
            return key
        case errSecItemNotFound: throw Failure.noKey
        default: throw Failure.keychain(status)
        }
    }

    /// What the sheet says when the vault did not open.
    static func message(for error: Error) -> String {
        switch error {
        case AutofillError.NoVault:
            "There is no Rowel vault on this device yet."
        case AutofillError.Locked:
            "Turn on unlock with Face ID or Touch ID in Rowel to fill passwords here."
        case AutofillError.WrongKey:
            "Rowel's Face ID key no longer opens this vault. Unlock Rowel with your password and turn Face ID on again."
        case let AutofillError.Io(message):
            message
        case let error as LAError where error.code == .userCancel || error.code == .systemCancel:
            "Rowel is locked."
        default:
            error.localizedDescription
        }
    }
}
