import Darwin
import Foundation
import os

/// A stream connection to the daemon's Unix socket over plain POSIX
/// sockets. URLSession can't reach a Unix socket. NWConnection can, but when
/// the daemon writes a response and closes at once (every response here),
/// it sometimes drops the data and reports ECANCELED instead, so cos uses
/// the socket directly. Blocking calls run on the connection's own queue;
/// cancelling shuts the socket down, which wakes a waiting read.
final class SocketConnection: ByteReader, @unchecked Sendable {
    private let socketPath: String
    private let queue = DispatchQueue(label: "cos.socket")
    /// The descriptor, and whether we shut it down ourselves. It is only
    /// closed in `deinit`, so a read still running on the queue never sees
    /// a reused descriptor.
    private let state = OSAllocatedUnfairLock(initialState: (fd: Int32(-1), shutDown: false))

    init(unixPath: String) {
        socketPath = unixPath
    }

    deinit {
        let fd = state.withLock { $0.fd }
        if fd >= 0 { Darwin.close(fd) }
    }

    /// Connects, mapping "no socket" and "nobody listening" to `.notRunning`.
    func open() async throws {
        try await run { [socketPath, state] in
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            let path = Array(socketPath.utf8)
            let capacity = MemoryLayout.size(ofValue: address.sun_path)
            guard path.count < capacity else {
                throw CosError.transport("The socket path is too long: \(socketPath)")
            }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: path) }

            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else { throw Self.error(errno, socketPath: socketPath) }
            var on: Int32 = 1
            // A write to a closed socket returns EPIPE instead of killing the app.
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
            let alreadyShutDown = state.withLock { state in
                state.fd = fd
                return state.shutDown
            }
            if alreadyShutDown { throw CancellationError() }
            let connected = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard connected == 0 else { throw Self.error(errno, socketPath: socketPath) }
        }
    }

    func send(_ bytes: [UInt8]) async throws {
        try await run { [socketPath, state] in
            let fd = state.withLock { $0.fd }
            var offset = 0
            while offset < bytes.count {
                let written = bytes[offset...].withUnsafeBytes { Darwin.write(fd, $0.baseAddress, $0.count) }
                if written < 0 {
                    if errno == EINTR { continue }
                    throw Self.error(errno, socketPath: socketPath)
                }
                offset += written
            }
        }
    }

    /// The next bytes, or `nil` once the daemon has closed its end.
    func read() async throws -> [UInt8]? {
        try await run { [socketPath, state] in
            let fd = state.withLock { $0.fd }
            var buffer = [UInt8](repeating: 0, count: 64 * 1024)
            while true {
                let count = buffer.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, $0.count) }
                if count > 0 { return Array(buffer[..<count]) }
                if count < 0, errno == EINTR { continue }
                // Our own shutdown also ends the read; that's a cancellation.
                if state.withLock({ $0.shutDown }) { throw CancellationError() }
                if count == 0 { return nil }
                throw Self.error(errno, socketPath: socketPath)
            }
        }
    }

    func close() {
        let fd = state.withLock { state in
            state.shutDown = true
            return state.fd
        }
        if fd >= 0 { shutdown(fd, SHUT_RDWR) }
    }

    /// Runs blocking socket calls on the connection's queue. Cancelling the
    /// task shuts the socket down, which ends whatever call is waiting.
    private func run<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T, Error>) in
                queue.async {
                    continuation.resume(with: Result { try work() })
                }
            }
        } onCancel: {
            close()
        }
    }

    private static func error(_ code: Int32, socketPath: String) -> Error {
        switch code {
        case ENOENT, ECONNREFUSED:
            return CosError.notRunning(socket: socketPath)
        case ECONNRESET, EPIPE, ENOTCONN:
            return CosError.connectionClosed
        case ECANCELED:
            return CancellationError()
        default:
            return CosError.transport(String(cString: strerror(code)))
        }
    }
}
