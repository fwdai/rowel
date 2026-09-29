import Foundation
import SafariServices
import os.log

/// Where Safari hands the extension's native messages: each one a request for
/// the Rowel app, relayed over the app's socket (`HostConnection`), and its
/// reply handed back. This is what the `rowel` proxy binary is for Chrome and
/// Firefox — which Safari cannot launch — with one difference: Safari asks and
/// waits for one reply at a time, so what the app pushes unsolicited rides on
/// the replies, and on a heartbeat poll, instead.
///
/// The exchange with the extension's JS (`background/safari-native.js`):
///
///     in:  { "request": "<the KeePassXC-Browser request, as JSON text>" }
///          { "poll": true }                   the heartbeat: signals only
///     out: { "events": [ { "seq": 1, "signal": "<JSON text>" },
///                        { "reply": "<JSON text>" }, … ],
///            "after": 1,        the last signal the app sent before the reply
///            "connected": true }
///          plus, when there is no reply,
///            "error": "not-running" | "disconnected" | "bad-request",
///            "detail": "…"
///
/// `events` is in the order the app sent them; the JS side emits them as
/// they are, and only checks them against `after`. Requests and replies
/// travel as text so nothing is lost converting between JSON and
/// property-list types (a JSON `null`, say, has no plist form).
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

    static func answer(_ message: Any?, host: HostConnection = .shared) -> [String: Any] {
        guard let envelope = message as? [String: Any] else {
            return badRequest("not an object")
        }
        if envelope["poll"] != nil {
            return encode(host.poll())
        }
        guard let text = envelope["request"] as? String,
            let request = text.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: request) as? [String: Any]
        else {
            return badRequest("no request in the message")
        }
        let action = object["action"] as? String ?? ""
        let outcome = host.request(request, action: action)
        if let failure = outcome.failure {
            log.info("\(action, privacy: .public): \(failure.code, privacy: .public) (\(failure.detail, privacy: .public))")
        }
        return encode(outcome)
    }

    static func encode(_ outcome: HostConnection.Outcome) -> [String: Any] {
        var response: [String: Any] = [
            "events": outcome.events.map { event -> [String: Any] in
                switch event {
                case .signal(let signal): return ["seq": signal.seq, "signal": signal.message]
                case .reply(let body): return ["reply": String(decoding: body, as: UTF8.self)]
                }
            },
            "connected": outcome.connected,
        ]
        if let after = outcome.after {
            response["after"] = after
        }
        if let failure = outcome.failure {
            response["error"] = failure.code
            response["detail"] = failure.detail
        }
        return response
    }

    // Refused before it reaches the connection: no events, which stay queued
    // for the next exchange.
    private static func badRequest(_ detail: String) -> [String: Any] {
        ["events": [], "connected": false, "error": "bad-request", "detail": detail]
    }
}
