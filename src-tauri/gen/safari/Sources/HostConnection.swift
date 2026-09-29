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
/// in its `dev` subdirectory, as the app's is (`browser::group_socket_dir`).
/// It is the app's second listener, there for Safari alone: Chrome and
/// Firefox reach the one in the app's data directory, which this sandboxed
/// process cannot. The app binds this one best-effort, so an app that could
/// not (a home path too long for a socket address, say) looks from here like
/// an app that is not running.
///
/// Ordering holds by construction. A reader thread takes every frame the app
/// sends and files it, under `state`, in the order it arrived: a lock signal
/// the app pushes unsolicited is numbered and queued, anything else is the
/// reply the request on the wire waits on. Requests and polls are served one
/// at a time (`inFlight`), and each response is composed under `state` in one
/// step — the reply, with every signal queued so far placed before or after it
/// as the app sent them — so nothing another caller does can take a signal
/// out from between them. The JS side (`safari-native.js`) keeps one message
/// outstanding at a time, so it hears everything in that same order.
final class HostConnection: @unchecked Sendable {
    static let shared = HostConnection()

    static let appGroup = "UFBL3F444A.app.rowel.desktop"
    static let socketFile = "browser.sock"
    /// Longer than the app's own wait on the user (`CONSENT_TIMEOUT`, 60 s),
    /// so an `associate` or a passkey dialog is answered by the app, not by
    /// this giving up first.
    static let replyTimeout: TimeInterval = 90
    /// Signals are never dropped from the queue to make room: that would
    /// change the sequence the extension hears. They are rare — one per lock
    /// or unlock — so a queue this long means nobody is fetching them, and
    /// the connection is dropped instead (logged), which the extension hears
    /// as a disconnect and recovers from by reconnecting and asking afresh.
    static let signalLimit = 256

    /// Why a request got no reply. The JS side (`safari-native.js`) maps
    /// every one of them to KeePassXC-Browser's "not connected" state.
    enum Failure: Error, Equatable {
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

    struct Signal: Equatable {
        let seq: Int
        let message: String
    }

    /// What the extension hears from one exchange, in the order the app sent
    /// it.
    enum Event: Equatable {
        case signal(Signal)
        case reply(Data)
    }

    /// One exchange's answer: its events, the last signal the app sent before
    /// the reply (a consistency check for the JS side; nil without a reply),
    /// whether the connection is up, and why there is no reply, if there is
    /// none.
    struct Outcome: Equatable {
        var events: [Event]
        var after: Int?
        var connected: Bool
        var failure: Failure?
    }

    private struct Reply {
        let generation: Int
        let body: Data
        let after: Int
    }

    private let log = Logger(subsystem: "app.rowel.desktop.safari", category: "host")
    private let resolveSocketPath: () -> String?
    private let push: ((Signal) -> Void)?

    // Held for a whole exchange, request or poll: one at a time.
    private let inFlight = NSLock()
    // Guards everything below; the reader thread signals it.
    private let state = NSCondition()
    private var fd: Int32 = -1
    private var generation = 0
    private var replies: [Reply] = []
    private var signals: [Signal] = []
    private var nextSeq = 1

    /// `socketPath` says where the app listens; `push` is offered each
    /// signal as it arrives (the extension's `dispatchMessage`), on top of its
    /// being queued. Tests give both their own.
    init(
        socketPath: @escaping () -> String? = HostConnection.socketPath,
        push: ((Signal) -> Void)? = HostConnection.dispatch
    ) {
        self.resolveSocketPath = socketPath
        self.push = push
    }

    /// The socket: in the group container, or its `dev` subdirectory in a
    /// debug build. A debug build honours `ROWEL_DB_DIR` — as `<dir>/browser.sock`,
    /// the socket a debug app run with it listens on (`browser::root_dir`) —
    /// so this code can be run outside Safari against a test app; Safari passes
    /// the extension no environment.
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
    func request(_ body: Data, action: String) -> Outcome {
        inFlight.lock()
        defer { inFlight.unlock() }

        state.lock()
        if fd < 0 {
            guard action == "change-public-keys" else {
                return finishLocked(failure: .disconnected("no connection to the app; exchange keys first"))
            }
            do {
                try connect()
            } catch let failure as Failure {
                return finishLocked(failure: failure)
            } catch {
                return finishLocked(failure: .notRunning("\(error)"))
            }
        }
        let socket = fd
        let current = generation
        replies.removeAll()
        state.unlock()

        do {
            try Frame.write(body, to: socket)
        } catch {
            drop(current, because: "write failed: \(error)")
            state.lock()
            return finishLocked(failure: .disconnected("the app went away"))
        }

        state.lock()
        let deadline = Date(timeIntervalSinceNow: Self.replyTimeout)
        // A reply that came in before the connection ended still counts: the
        // app answered.
        while !replies.contains(where: { $0.generation == current }) && generation == current {
            if !state.wait(until: deadline) { break }
        }
        if let index = replies.firstIndex(where: { $0.generation == current }) {
            let reply = replies.remove(at: index)
            return finishLocked(reply: reply)
        }
        if generation == current {
            // Timed out. The reply may still come, and would then be taken for
            // the next request's: the connection cannot be trusted any more.
            dropLocked(because: "no reply in \(Int(Self.replyTimeout)) s")
        }
        return finishLocked(failure: .disconnected("the connection to the app ended"))
    }

    /// The heartbeat: the signals queued so far, and whether the connection
    /// is up. Waits its turn behind a request on the wire, so it can never
    /// take the signals that belong around that request's reply.
    func poll() -> Outcome {
        inFlight.lock()
        defer { inFlight.unlock() }
        state.lock()
        return finishLocked()
    }

    // Compose an exchange's outcome from the queue, and release `state`
    // (held on entry). Every queued signal goes in, the ones the app sent
    // before the reply ahead of it; the queue is left empty.
    private func finishLocked(reply: Reply? = nil, failure: Failure? = nil) -> Outcome {
        defer { state.unlock() }
        var events: [Event] = []
        let queued = signals
        signals.removeAll()
        if let reply {
            events += queued.filter { $0.seq <= reply.after }.map(Event.signal)
            events.append(.reply(reply.body))
            events += queued.filter { $0.seq > reply.after }.map(Event.signal)
        } else {
            events = queued.map(Event.signal)
        }
        return Outcome(events: events, after: reply?.after, connected: fd >= 0, failure: failure)
    }

    // MARK: - The socket

    // Called with `state` held.
    private func connect() throws {
        guard let path = resolveSocketPath() else {
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
                    replies.append(Reply(generation: reading, body: body, after: nextSeq - 1))
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

    // The queued signals stay: the app sent them before the connection ended,
    // and the extension hears them before it hears that it did.
    private func dropLocked(because reason: String) {
        log.info("connection dropped: \(reason, privacy: .public)")
        // Shut down, not closed: the reader blocked in read() returns, and
        // the reader alone closes the descriptor, on its way out. Closed here,
        // its number could go to the next connect while the old reader still
        // reads from it.
        shutdown(fd, SHUT_RDWR)
        fd = -1
        generation += 1
        state.broadcast()
    }

    // MARK: - Signals

    /// A signal from the app: numbered and queued for the next exchange to
    /// carry, and offered to `push`.
    private func deliver(_ body: Data, generation reading: Int) {
        let message = String(decoding: body, as: UTF8.self)
        state.lock()
        guard generation == reading else {
            state.unlock()
            return
        }
        guard signals.count < Self.signalLimit else {
            log.fault("\(Self.signalLimit) signals nobody fetched; dropping the connection rather than a signal")
            signals.removeAll()
            dropLocked(because: "signal queue full")
            state.unlock()
            return
        }
        let signal = Signal(seq: nextSeq, message: message)
        nextSeq += 1
        signals.append(signal)
        state.unlock()
        push?(signal)
    }

    /// Push a signal to the extension's native port. Whether this works from
    /// inside the extension process is undocumented — Apple documents
    /// `dispatchMessage` for the containing app — so the JS side takes it only
    /// as a hint to poll now (`safari-native.js`); the signal itself always
    /// comes through the queue, in order.
    static func dispatch(_ signal: Signal) {
        guard let extensionID = Bundle.main.bundleIdentifier else { return }
        SFSafariApplication.dispatchMessage(
            withName: "signal",
            toExtensionWithIdentifier: extensionID,
            userInfo: ["seq": signal.seq]
        ) { error in
            if let error {
                Logger(subsystem: "app.rowel.desktop.safari", category: "host")
                    .debug("pushing a signal failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }
}
