import Darwin
import Foundation

/// Native messaging framing, as `src-tauri/src/browser/frame.rs` speaks it: a
/// four-byte length in the host's byte order, then that many bytes of JSON.
/// The cap is the same both ways; a length past it is never read.
enum Frame {
    static let maxLength = 1024 * 1024

    enum Failure: Error {
        case tooLarge(Int)
        case truncated
        case io(Int32)
    }

    /// One frame from `fd`, or nil at a clean end of stream — nothing after
    /// the last frame. A stream that ends inside a frame is `truncated`.
    static func read(from fd: Int32) throws -> Data? {
        guard let header = try readExactly(4, from: fd, atStart: true) else { return nil }
        let length = Int(header.withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) })
        guard length <= maxLength else { throw Failure.tooLarge(length) }
        guard let body = try readExactly(length, from: fd, atStart: false) else {
            throw Failure.truncated
        }
        return body
    }

    /// One frame to `fd`, whole: the reader on the other side waits on all of it.
    static func write(_ body: Data, to fd: Int32) throws {
        guard body.count <= maxLength else { throw Failure.tooLarge(body.count) }
        var length = UInt32(body.count)
        var frame = Data(bytes: &length, count: 4)
        frame.append(body)
        try frame.withUnsafeBytes { raw in
            var offset = 0
            while offset < raw.count {
                let written = Darwin.write(fd, raw.baseAddress! + offset, raw.count - offset)
                if written < 0 {
                    if errno == EINTR { continue }
                    throw Failure.io(errno)
                }
                offset += written
            }
        }
    }

    // `count` bytes, or nil when the stream ends before the first of them and
    // that is allowed (`atStart`, between frames).
    private static func readExactly(_ count: Int, from fd: Int32, atStart: Bool) throws -> Data? {
        var data = Data(count: count)
        var offset = 0
        while offset < count {
            let got = data.withUnsafeMutableBytes { raw in
                Darwin.read(fd, raw.baseAddress! + offset, count - offset)
            }
            if got < 0 {
                if errno == EINTR { continue }
                throw Failure.io(errno)
            }
            if got == 0 {
                if offset == 0 && atStart { return nil }
                throw Failure.truncated
            }
            offset += got
        }
        return data
    }
}
