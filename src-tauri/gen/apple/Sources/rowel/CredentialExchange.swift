import AuthenticationServices
import Foundation
import UIKit

/// The receiving end of iOS 26 credential exchange: another password manager
/// (Passwords calls it "Export Data to Another App") hands its credentials to
/// this app through the system.
///
/// The system launches the app with a user activity that carries a token. The
/// token is traded for the credentials, which encode to the FIDO Credential
/// Exchange Format document the Rust importer already reads from a file, and
/// that document is handed to the core (`src-tauri/src/exchange.rs`). Nothing
/// is kept here and nothing is written to disk.
enum CredentialExchange {
    static func install() {
        // Posted once tao's delegate has returned from launching: its class
        // and the scene delegate's exist by then, and no scene has connected.
        NotificationCenter.default.addObserver(
            forName: UIApplication.didFinishLaunchingNotification, object: nil, queue: nil
        ) { _ in
            UserActivityTap.install(
                appDelegate: UIApplication.shared.delegate.flatMap { object_getClass($0) },
                sceneDelegate: NSClassFromString("TaoSceneDelegate"),
                handler: received
            )
        }
    }

    /// True for a credential exchange, whatever becomes of it: the activity
    /// is this app's to answer for from the moment it is recognised.
    private static func received(_ activity: NSUserActivity, via _: String) -> Bool {
        guard #available(iOS 26.0, *), activity.activityType == ASCredentialExchangeActivity else {
            return false
        }
        guard let token = activity.userInfo?[ASCredentialImportToken] as? UUID else {
            rowel_credential_exchange_failed("The other app sent nothing to import.")
            return true
        }
        Task { @MainActor in
            do {
                let credentials = try await ASCredentialImportManager().importCredentials(token: token)
                let encoder = JSONEncoder()
                // CXF dates are Unix seconds; the encoder's default counts from 2001.
                encoder.dateEncodingStrategy = .secondsSince1970
                var document = try encoder.encode(credentials)
                // The core copies what it is given, so this copy is wiped.
                defer { document.resetBytes(in: 0..<document.count) }
                document.withUnsafeBytes { bytes in
                    rowel_credential_exchange_received(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
                }
            } catch {
                rowel_credential_exchange_failed("The transfer did not complete. \(error.localizedDescription)")
            }
        }
        return true
    }
}

/// Called from `main`, before the app starts.
@_cdecl("rowel_credential_exchange_install")
public func rowelCredentialExchangeInstall() {
    CredentialExchange.install()
}

/// Asked by the core: whether this device can be handed another app's
/// credentials at all.
@_cdecl("rowel_credential_exchange_supported")
public func rowelCredentialExchangeSupported() -> Bool {
    if #available(iOS 26.0, *) {
        return true
    }
    return false
}
