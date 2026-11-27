import AuthenticationServices
import Foundation
import UIKit

/// A probe, not a feature: answers whether Apple's Passwords app lists Rowel
/// under "Export Data to Another App" and what it hands over when it does.
///
/// It imports nothing. When the system launches the app with a credential
/// exchange, the payload is fetched and reduced to its shape — counts and the
/// names of the fields present, never a value — and that report is shown in an
/// alert, with a button that copies it, and written to the log.
enum CredentialExchangeProbe {
    /// The activity types this app declares in `NSUserActivityTypes`. Apple's
    /// documentation spells the exchange's type both ways; the SDK constant
    /// is the first.
    private static let exchangeTypes = ["ASCredentialExchangeActivity", "ASCredentialExchangeActivityType"]

    private static var lines: [String] = []

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
            note("hooks: \(UserActivityTap.installed.sorted { $0.key < $1.key }.map { "\($0.key) \($0.value)" })")
            if #available(iOS 26.0, *) {
                note("iOS \(UIDevice.current.systemVersion), exchange activity type \(ASCredentialExchangeActivity)")
            } else {
                note("iOS \(UIDevice.current.systemVersion): no credential exchange before 26")
            }
        }
    }

    private static func received(_ activity: NSUserActivity, via: String) {
        let keys = (activity.userInfo ?? [:]).keys.map { "\($0)" }.sorted()
        note("activity \(activity.activityType) via \(via), userInfo keys \(keys)")
        guard exchangeTypes.contains(activity.activityType) || keys.contains("ASCredentialImportToken") else {
            return
        }
        guard #available(iOS 26.0, *) else {
            return finish("Credential exchange needs iOS 26; this is \(UIDevice.current.systemVersion).")
        }
        guard let token = activity.userInfo?[ASCredentialImportToken] as? UUID else {
            return finish("The activity carried no import token.")
        }
        Task { @MainActor in
            do {
                let payload = try await ASCredentialImportManager().importCredentials(token: token)
                finish(try describe(payload))
            } catch {
                let e = error as NSError
                finish("importCredentials failed: \(e.domain) \(e.code)\n\(e.localizedDescription)")
            }
        }
    }

    /// The payload as the CXF JSON the Rust importer reads, reduced to what
    /// is safe to show: how many of each thing, and which fields are present.
    @available(iOS 26.0, *)
    private static func describe(_ payload: ASExportedCredentialData) throws -> String {
        let encoder = JSONEncoder()
        // CXF dates are Unix seconds; the encoder's default counts from 2001.
        encoder.dateEncodingStrategy = .secondsSince1970
        let json = try encoder.encode(payload)
        let document = try JSONSerialization.jsonObject(with: json)

        let items = payload.accounts.reduce(0) { $0 + $1.items.count }
        var credentials: [String: Int] = [:]
        var paths: [String: Int] = [:]
        walk(document, "", &paths, &credentials)

        var out = [
            "exporter: \(payload.exporterDisplayName) (\(payload.exporterRelyingPartyIdentifier))",
            "format: \(payload.formatVersion.rawValue), \(json.count) bytes of JSON",
            "accounts: \(payload.accounts.count), items: \(items)",
            "credentials by type:",
        ]
        out += credentials.sorted { $0.key < $1.key }.map { "  \($0.key): \($0.value)" }
        out.append("fields present (count):")
        out += paths.sorted { $0.key < $1.key }.map { "  \($0.key): \($0.value)" }
        return out.joined(separator: "\n")
    }

    /// Count every field path in the document. A value is kept only where it
    /// is one of the format's own enumerations, never user data.
    private static func walk(
        _ value: Any, _ path: String, _ paths: inout [String: Int], _ credentials: inout [String: Int]
    ) {
        switch value {
        case let object as [String: Any]:
            if path.hasSuffix("credentials[]"), let type = object["type"] as? String {
                credentials[type, default: 0] += 1
            }
            for (key, child) in object {
                let next = path.isEmpty ? key : "\(path).\(key)"
                if ["type", "fieldType", "algorithm"].contains(key), let name = child as? String {
                    paths["\(next)=\(name)", default: 0] += 1
                } else {
                    walk(child, next, &paths, &credentials)
                }
            }
        case let array as [Any]:
            for child in array {
                walk(child, "\(path)[]", &paths, &credentials)
            }
        default:
            paths[path, default: 0] += 1
        }
    }

    private static func note(_ line: String) {
        lines.append(line)
        NSLog("[credential-exchange-probe] %@", line)
    }

    private static func finish(_ result: String) {
        let report = (lines + ["", result]).joined(separator: "\n")
        NSLog("[credential-exchange-probe] report:\n%@", report)
        DispatchQueue.main.async { present(report, attempt: 0) }
    }

    // A launch delivers the activity before the window has a view controller,
    // so the alert waits for one.
    private static func present(_ report: String, attempt: Int) {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
        var top = (windows.first(where: \.isKeyWindow) ?? windows.first)?.rootViewController
        while let presented = top?.presentedViewController {
            top = presented
        }
        guard let top else {
            if attempt < 40 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { present(report, attempt: attempt + 1) }
            }
            return
        }
        let alert = UIAlertController(title: "Credential exchange probe", message: report, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Copy report", style: .default) { _ in
            UIPasteboard.general.string = report
        })
        alert.addAction(UIAlertAction(title: "Close", style: .cancel))
        top.present(alert, animated: true)
    }
}

/// Called from `main`, before the app starts.
@_cdecl("rowel_credential_exchange_probe_install")
public func rowelCredentialExchangeProbeInstall() {
    CredentialExchangeProbe.install()
}
