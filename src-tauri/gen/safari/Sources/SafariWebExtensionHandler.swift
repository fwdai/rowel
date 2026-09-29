import Foundation
import SafariServices
import os.log

/// Where Safari hands the extension's native messages: each one a request for
/// the Rowel app, relayed over the app's socket (`HostConnection`), and its
/// reply handed back. This is what the `rowel` proxy binary is for Chrome and
/// Firefox — which Safari cannot launch — with one difference: Safari asks and
/// waits for one reply at a time, so what the app pushes unsolicited rides on
/// the replies as well.
///
/// The exchange with the extension's JS (`background/safari-native.js`):
///
///     in:  { "request": "<the KeePassXC-Browser request, as JSON text>" }
///          { "poll": true }                      the heartbeat: signals only
///     out: { "reply": "<the app's reply, as JSON text>",
///            "after": 1,      the last signal the app sent before the reply
///            "signals": [{ "seq": 1, "message": "<JSON text>" }, …],
///            "connected": true }
///          { "error": "not-running" | "disconnected" | "bad-request",
///            "detail": "…", "signals": […], "connected": false }
///
/// Requests and replies travel as text so nothing is lost converting between
/// JSON and property-list types (a JSON `null`, say, has no plist form).
final class SafariWebExtensionHandler: NSObject, NSExtensionRequestHandling {
    private static let log = Logger(subsystem: "app.rowel.desktop.safari", category: "handler")

    func beginRequest(with context: NSExtensionContext) {
        let item = context.inputItems.first as? NSExtensionItem
        let message = item?.userInfo?[SFExtensionMessageKey]
        // A request can wait on the user for up to a minute (an `associate`
        // dialog): off the thread Safari called in on.
        DispatchQueue.global(qos: .userInitiated).async {
            let response = Self.answer(message)
            let reply = NSExtensionItem()
            reply.userInfo = [SFExtensionMessageKey: response]
            context.completeRequest(returningItems: [reply], completionHandler: nil)
        }
    }

    static func answer(_ message: Any?) -> [String: Any] {
        let host = HostConnection.shared
        guard let envelope = message as? [String: Any] else {
            return failure("bad-request", "not an object", host)
        }
        if envelope["poll"] != nil {
            return [
                "signals": signals(host),
                "connected": host.isConnected,
            ]
        }
        guard let text = envelope["request"] as? String,
            let request = text.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: request) as? [String: Any]
        else {
            return failure("bad-request", "no request in the message", host)
        }
        let action = object["action"] as? String ?? ""
        switch host.send(request, action: action) {
        case .success(let reply):
            return [
                "reply": String(decoding: reply.body, as: UTF8.self),
                "after": reply.after,
                "signals": signals(host),
                "connected": true,
            ]
        case .failure(let error):
            log.info("\(action, privacy: .public): \(error.code, privacy: .public) (\(error.detail, privacy: .public))")
            return failure(error.code, error.detail, host)
        }
    }

    private static func failure(_ code: String, _ detail: String, _ host: HostConnection) -> [String: Any] {
        [
            "error": code,
            "detail": detail,
            "signals": signals(host),
            "connected": host.isConnected,
        ]
    }

    private static func signals(_ host: HostConnection) -> [[String: Any]] {
        host.takeSignals().map { ["seq": $0.seq, "message": $0.message] }
    }
}
