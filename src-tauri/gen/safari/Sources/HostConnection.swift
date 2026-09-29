import Darwin
import Foundation
import SafariServices
import os.log

/// The one connection to the Rowel app's browser host, shared by every
/// handler instance: Safari may make a new `SafariWebExtensionHandler` for
/// each message, but the app keeps a key exchange per connection
/// (`browser::actions::Connection`), so the socket has to outlive them.
///
/// The socket is `browser.sock` in the App Group container the app and this
/// extension share (`rowel_core::app::DESKTOP_APP_GROUP`); a debug build's is
/// in its `dev` subdirectory, as the app's is (`browser::socket_dir`).
///
/// One request is on the wire at a time. A reader thread takes every frame
/// the app sends: the lock signals it pushes unsolicited go to the extension
/// (`deliver`), anything else is the reply the request in flight waits on.
final class HostConnection: @unchecked Sendable {
    static let shared = HostConnection()

    static let appGroup = "UFBL3F444A.app.rowel.desktop"
    static let socketFile = "browser.sock"
    /// Longer than the app's own wait on the user (`CONSENT_TIMEOUT`, 60 s),
    /// so an `associate` or a passkey dialog is answered by the app, not by
    /// this giving up first.
    static let replyTimeout: TimeInterval = 90
    /// The signals kept for a reply to carry. Only the latest few say
    /// anything: the extension acts on the state they leave it in.
    static let signalBacklog = 8

    /// Why a request got no reply. The JS side (`safari-native.js`) maps
    /// every one of them to KeePassXC-Browser's "not connected" state.
    enum Failure: Error {
        /// Nothing is listening: the app is not running, or has the browser
        /// integration turned off.
        case notRunning(String)
        /// The connection ended, or was never made for this request: a
        /// request other than `change-public-keys` on a fresh connection
        /// would reach an app holding no keys for it. The extension has to
        /// connect again, and exchange keys first.
        case disconnected(String)

        var code: String {
            switch self {
            case .notRunning: return "not-running"
            case .disconnected: return "disconnected"
            }
        }

        var detail: String {
            switch self {
            case .notRunning(let detail), .disconnected(let detail): return detail
            }
        }
    }

    struct Signal {
        let seq: Int
        let message: String
    }

    /// A reply, and the last signal the app sent before it (0 for none):
    /// the extension hears the signals up to it before the reply, and the
    /// rest after, in the order the app sent them.
    struct Reply {
        let body: Data
        let after: Int
    }

    private let log = Logger(subsystem: "app.rowel.desktop.safari", category: "host")

    // Held for a whole request: write, then wait for the reply.
    private let inFlight = NSLock()
    // Guards everything below; the reader thread signals it.
    private let state = NSCondition()
    private var fd: Int32 = -1
    private var generation = 0
    private var replies: [Reply] = []
    private var signals: [Signal] = []
    private var nextSeq = 1
    // Whether pushing a signal from here reached Safari. Unknown until the
    // first try; the signals are queued for the next reply either way.
    private var dispatchFailed = false

    var isConnected: Bool {
        state.lock()
        defer { state.unlock() }
        return fd >= 0
    }

    /// The socket: in the group container, or its `dev` subdirectory in a
    /// debug build. A debug build honours `ROWEL_DB_DIR` as the app's does
    /// (`browser::socket_dir`) — Safari passes the extension no environment,
    /// so that is for running this code outside Safari against a test app.
    static func socketPath() -> String? {
        #if DEBUG
        if let dir = ProcessInfo.processInfo.environment["ROWEL_DB_DIR"] {
            return URL(fileURLWithPath: dir).appendingPathComponent(socketFile).path
        }
        #endif
        guard var dir = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroup
        ) else { return nil }
        #if DEBUG
        dir.appendPathComponent("dev", isDirectory: true)
        #endif
        return dir.appendingPathComponent(socketFile).path
    }

    /// Send one request and wait for its reply. `action` is the request's,
    /// read by the caller: it decides whether a fresh connection may be made.
    func send(_ request: Data, action: String) -> Result<Reply, Failure> {
        inFlight.lock()
        defer { inFlight.unlock() }

        state.lock()
        if fd < 0 {
            guard action == "change-public-keys" else {
                state.unlock()
                return .failure(.disconnected("no connection to the app; exchange keys first"))
            }
            do {
                try connect()
            } catch let failure as Failure {
                state.unlock()
                return .failure(failure)
            } catch {
                state.unlock()
                return .failure(.notRunning("\(error)"))
            }
        }
        let socket = fd
        let current = generation
        replies.removeAll()
        state.unlock()

        do {
            try Frame.write(request, to: socket)
        } catch {
            drop(current, because: "write failed: \(error)")
            return .failure(.disconnected("the app went away"))
        }

        state.lock()
        defer { state.unlock() }
        let deadline = Date(timeIntervalSinceNow: Self.replyTimeout)
        while replies.isEmpty && generation == current && fd >= 0 {
            if !state.wait(until: deadline) { break }
        }
        if !replies.isEmpty && generation == current {
            return .success(replies.removeFirst())
        }
        if generation == current && fd >= 0 {
            // Timed out. The reply may still come, and would then be taken for
            // the next request's: the connection cannot be trusted any more.
            dropLocked(because: "no reply in \(Int(Self.replyTimeout)) s")
        }
        return .failure(.disconnected("the connection to the app ended"))
    }

    /// The signals queued since the last call, oldest first.
    func takeSignals() -> [Signal] {
        state.lock()
        defer { state.unlock() }
        let taken = signals
        signals.removeAll()
        return taken
    }

    // MARK: - The socket

    // Called with `state` held.
    private func connect() throws {
        guard let path = Self.socketPath() else {
            throw Failure.notRunning("no App Group container for \(Self.appGroup)")
        }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8) + [0]
        guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            throw Failure.notRunning("socket path too long: \(path)")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
        }
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)

        let socket = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard socket >= 0 else { throw Failure.notRunning("socket: \(errno)") }
        var on: Int32 = 1
        setsockopt(socket, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        let connected = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(socket, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            let error = errno
            close(socket)
            throw Failure.notRunning("connect \(path): \(String(cString: strerror(error)))")
        }

        fd = socket
        generation += 1
        let reading = generation
        log.info("connected to the app")
        let thread = Thread { [weak self] in self?.read(socket, generation: reading) }
        thread.name = "rowel-host-reader"
        thread.start()
    }

    private func read(_ socket: Int32, generation reading: Int) {
        defer { close(socket) }
        while true {
            let body: Data
            do {
                guard let frame = try Frame.read(from: socket) else {
                    drop(reading, because: "the app closed the connection")
                    return
                }
                body = frame
            } catch {
                // A length past the cap included: the frame is never read,
                // and nothing after it can be told apart, so the connection
                // is done.
                drop(reading, because: "read failed: \(error)")
                return
            }
            if Self.isSignal(body) {
                deliver(body, generation: reading)
            } else {
                state.lock()
                if generation == reading {
                    replies.append(Reply(body: body, after: nextSeq - 1))
                    state.broadcast()
                }
                state.unlock()
            }
        }
    }

    /// What `browser::server::signal` sends, in the clear: `database-locked`
    /// and `database-unlocked`. No reply carries either action.
    static func isSignal(_ body: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
            let action = object["action"] as? String
        else { return false }
        return action == "database-locked" || action == "database-unlocked"
    }

    private func drop(_ reading: Int, because reason: String) {
        state.lock()
        defer { state.unlock() }
        if generation == reading && fd >= 0 {
            dropLocked(because: reason)
        }
    }

    private func dropLocked(because reason: String) {
        log.info("connection dropped: \(reason, privacy: .public)")
        // Shut down, not closed: the reader blocked in read() returns, and
        // the reader alone closes the descriptor, on its way out. Closed here,
        // its number could go to the next connect while the old reader still
        // reads from it.
        shutdown(fd, SHUT_RDWR)
        fd = -1
        generation += 1
        replies.removeAll()
        state.broadcast()
    }

    // MARK: - Signals

    /// A signal from the app: queued for the next reply to carry, and pushed
    /// to the extension's native port straight away. Whether the push works
    /// from inside the extension process is undocumented — Apple documents
    /// `dispatchMessage` for the containing app — so the queue is what the
    /// extension relies on (with a heartbeat to fetch it, `safari-native.js`),
    /// and the push is a shortcut when it lands. Both carry the sequence
    /// number the JS side drops duplicates by.
    private func deliver(_ body: Data, generation reading: Int) {
        let message = String(decoding: body, as: UTF8.self)
        state.lock()
        guard generation == reading else {
            state.unlock()
            return
        }
        let signal = Signal(seq: nextSeq, message: message)
        nextSeq += 1
        signals.append(signal)
        if signals.count > Self.signalBacklog {
            signals.removeFirst(signals.count - Self.signalBacklog)
        }
        let tryPush = !dispatchFailed
        state.unlock()

        guard tryPush, let extensionID = Bundle.main.bundleIdentifier else { return }
        SFSafariApplication.dispatchMessage(
            withName: "signal",
            toExtensionWithIdentifier: extensionID,
            userInfo: ["seq": signal.seq, "message": signal.message]
        ) { [weak self] error in
            guard let self, let error else { return }
            self.log.info("pushing a signal failed: \(error.localizedDescription, privacy: .public); signals ride on replies")
            self.state.lock()
            self.dispatchFailed = true
            self.state.unlock()
        }
    }
}
