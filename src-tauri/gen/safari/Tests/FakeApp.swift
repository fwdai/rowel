import Darwin
import Foundation

/// A stand-in for the Rowel app's browser host: a Unix socket in a temporary
/// directory, speaking the host's framing, and answering each request with
/// whatever the test's `handle` does with it.
final class FakeApp {
    final class Peer {
        let fd: Int32
        init(fd: Int32) { self.fd = fd }

        func send(_ object: [String: Any]) {
            try? Frame.write(try! JSONSerialization.data(withJSONObject: object), to: fd)
        }

        func signal(_ action: String) { send(["action": action]) }

        func reply(to request: [String: Any], _ extra: [String: Any] = [:]) {
            var object: [String: Any] = ["action": request["action"] ?? "", "success": "true"]
            object.merge(extra) { $1 }
            send(object)
        }

        func sendRaw(_ bytes: Data) {
            _ = bytes.withUnsafeBytes { Darwin.write(fd, $0.baseAddress!, bytes.count) }
        }

        func close() {
            shutdown(fd, SHUT_RDWR)
        }
    }

    let directory: URL
    let path: String
    private let listener: Int32
    private let lock = NSLock()
    private var accepted = 0
    private let handle: (Peer, [String: Any]) -> Void

    var connections: Int {
        lock.lock()
        defer { lock.unlock() }
        return accepted
    }

    init(handle: @escaping (Peer, [String: Any]) -> Void) throws {
        self.handle = handle
        // Short, for sun_path: /tmp rather than the per-user temporary dir.
        directory = URL(fileURLWithPath: "/tmp/rowel-safari-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        path = directory.appendingPathComponent("browser.sock").path

        listener = socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8) + [0]
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0, listen(listener, 8) == 0 else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
        }
        Thread { [self] in acceptLoop() }.start()
    }

    /// Stop listening, as an app that quit would; connections already made
    /// are the test's to close.
    func stop() {
        Darwin.close(listener)
        try? FileManager.default.removeItem(at: directory)
    }

    private func acceptLoop() {
        while true {
            let fd = accept(listener, nil, nil)
            guard fd >= 0 else { return }
            var on: Int32 = 1
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
            lock.lock()
            accepted += 1
            lock.unlock()
            let peer = Peer(fd: fd)
            Thread { [self] in
                defer { Darwin.close(fd) }
                while let body = try? Frame.read(from: fd) {
                    let request = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any] ?? [:]
                    handle(peer, request)
                }
            }.start()
        }
    }
}
