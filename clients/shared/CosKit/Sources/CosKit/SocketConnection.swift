import Darwin
import Foundation
import os

/// A stream connection to the daemon over plain POSIX sockets: its Unix
/// socket, or its tailnet listener over TCP. URLSession can't reach a Unix
/// socket. NWConnection can, but when the daemon writes a response and
/// closes at once (every response here), it sometimes drops the data and
/// reports ECANCELED instead, so cos uses sockets directly, the same way for
/// both. Blocking calls run on the connection's own queue; cancelling shuts
/// the socket down, which wakes a waiting read.
final class SocketConnection: ByteReader, @unchecked Sendable {
    private let endpoint: CosEndpoint
    private let connectTimeout: Duration
    private let queue = DispatchQueue(label: "cos.socket")
    /// The descriptor, and whether we shut it down ourselves. It is only
    /// closed in `deinit`, so a read still running on the queue never sees
    /// a reused descriptor.
    private let state = OSAllocatedUnfairLock(initialState: (fd: Int32(-1), shutDown: false))

    /// Over TCP, a read that waits this long fails, so a stream whose peer
    /// vanished (the phone slept, the network changed) is noticed and
    /// reopened. The event stream's keep-alives come every 15 s.
    static let tcpReadTimeout = 45

    init(endpoint: CosEndpoint, connectTimeout: Duration = .seconds(6)) {
        self.endpoint = endpoint
        self.connectTimeout = connectTimeout
    }

    convenience init(unixPath: String) {
        self.init(endpoint: .unix(unixPath))
    }

    deinit {
        let fd = state.withLock { $0.fd }
        if fd >= 0 { Darwin.close(fd) }
    }

    /// Connects. On the socket, "no socket" and "nobody listening" are
    /// `.notRunning`; over TCP, a name that doesn't resolve, a refusal or a
    /// timeout is `.unreachable`.
    func open() async throws {
        switch endpoint {
        case .unix(let path):
            try await run { [state] in try Self.openUnix(path: path, state: state) }
        case .tcp(let host, let port):
            let timeout = connectTimeout
            try await run { [state] in try Self.openTCP(host: host, port: port, timeout: timeout, state: state) }
        }
    }

    func send(_ bytes: [UInt8]) async throws {
        try await run { [endpoint, state] in
            let fd = state.withLock { $0.fd }
            var offset = 0
            while offset < bytes.count {
                let written = bytes[offset...].withUnsafeBytes { Darwin.write(fd, $0.baseAddress, $0.count) }
                if written < 0 {
                    if errno == EINTR { continue }
                    throw Self.error(errno, endpoint: endpoint)
                }
                offset += written
            }
        }
    }

    /// The next bytes, or `nil` once the daemon has closed its end.
    func read() async throws -> [UInt8]? {
        try await run { [endpoint, state] in
            let fd = state.withLock { $0.fd }
            var buffer = [UInt8](repeating: 0, count: 64 * 1024)
            while true {
                let count = buffer.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, $0.count) }
                if count > 0 { return Array(buffer[..<count]) }
                if count < 0, errno == EINTR { continue }
                // Our own shutdown also ends the read; that's a cancellation.
                if state.withLock({ $0.shutDown }) { throw CancellationError() }
                if count == 0 { return nil }
                if errno == EAGAIN || errno == EWOULDBLOCK {
                    throw CosError.unreachable(address: endpoint.description, detail: "It stopped answering.")
                }
                throw Self.error(errno, endpoint: endpoint)
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

    // MARK: Opening

    private typealias State = OSAllocatedUnfairLock<(fd: Int32, shutDown: Bool)>

    /// Records `fd` as the connection's, unless it was already shut down.
    /// A descriptor from an earlier attempt (another address of the host)
    /// is closed first.
    private static func adopt(_ fd: Int32, _ state: State) throws {
        var on: Int32 = 1
        // A write to a closed socket returns EPIPE instead of killing the app.
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        let alreadyShutDown = state.withLock { state in
            if state.fd >= 0, state.fd != fd { Darwin.close(state.fd) }
            state.fd = fd
            return state.shutDown
        }
        if alreadyShutDown { throw CancellationError() }
    }

    private static func openUnix(path socketPath: String, state: State) throws {
        let endpoint = CosEndpoint.unix(socketPath)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let path = Array(socketPath.utf8)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard path.count < capacity else {
            throw CosError.transport("The socket path is too long: \(socketPath)")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: path) }

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw error(errno, endpoint: endpoint) }
        try adopt(fd, state)
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { throw error(errno, endpoint: endpoint) }
    }

    /// Resolves `host` and tries each address in turn, each with `timeout`.
    private static func openTCP(host: String, port: Int, timeout: Duration, state: State) throws {
        let endpoint = CosEndpoint.tcp(host: host, port: port)
        var hints = addrinfo()
        hints.ai_family = AF_UNSPEC
        hints.ai_socktype = SOCK_STREAM
        hints.ai_protocol = IPPROTO_TCP
        var list: UnsafeMutablePointer<addrinfo>?
        let resolved = getaddrinfo(host, String(port), &hints, &list)
        guard resolved == 0, let first = list else {
            let why = String(cString: gai_strerror(resolved))
            throw CosError.unreachable(
                address: endpoint.description,
                detail: "Can't find \(host) (\(why)). Is Tailscale on, on this device?"
            )
        }
        defer { freeaddrinfo(list) }

        var lastError: Error = CosError.unreachable(address: endpoint.description, detail: "It has no address.")
        var info: UnsafeMutablePointer<addrinfo>? = first
        while let current = info {
            info = current.pointee.ai_next
            if state.withLock({ $0.shutDown }) { throw CancellationError() }
            let fd = socket(current.pointee.ai_family, current.pointee.ai_socktype, current.pointee.ai_protocol)
            guard fd >= 0 else { continue }
            do {
                try adopt(fd, state)
                try connect(fd, current.pointee.ai_addr, current.pointee.ai_addrlen, timeout: timeout, endpoint: endpoint, state: state)
                var on: Int32 = 1
                setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, socklen_t(MemoryLayout<Int32>.size))
                var readTimeout = timeval(tv_sec: tcpReadTimeout, tv_usec: 0)
                setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &readTimeout, socklen_t(MemoryLayout<timeval>.size))
                return
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                lastError = error
            }
        }
        throw lastError
    }

    /// A non-blocking connect, waited for in short slices so a shutdown
    /// (cancellation) is noticed, then switched back to blocking.
    private static func connect(
        _ fd: Int32, _ address: UnsafeMutablePointer<sockaddr>?, _ length: socklen_t,
        timeout: Duration, endpoint: CosEndpoint, state: State
    ) throws {
        let flags = fcntl(fd, F_GETFL, 0)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
        defer { _ = fcntl(fd, F_SETFL, flags) }
        if Darwin.connect(fd, address, length) == 0 { return }
        guard errno == EINPROGRESS else { throw error(errno, endpoint: endpoint) }
        let deadline = ContinuousClock.now + timeout
        while true {
            if state.withLock({ $0.shutDown }) { throw CancellationError() }
            var poller = pollfd(fd: fd, events: Int16(POLLOUT), revents: 0)
            let ready = poll(&poller, 1, 200)
            if ready < 0 {
                if errno == EINTR { continue }
                throw error(errno, endpoint: endpoint)
            }
            if ready > 0 { break }
            if ContinuousClock.now >= deadline { throw error(ETIMEDOUT, endpoint: endpoint) }
        }
        var problem: Int32 = 0
        var size = socklen_t(MemoryLayout<Int32>.size)
        getsockopt(fd, SOL_SOCKET, SO_ERROR, &problem, &size)
        if problem != 0 { throw error(problem, endpoint: endpoint) }
    }

    private static func error(_ code: Int32, endpoint: CosEndpoint) -> Error {
        switch (code, endpoint) {
        case (ENOENT, .unix(let path)), (ECONNREFUSED, .unix(let path)):
            return CosError.notRunning(socket: path)
        case (ECONNREFUSED, .tcp):
            return CosError.unreachable(
                address: endpoint.description,
                detail: "Nothing answers on that port. Is cos running there, with remote access on (cos remote on)?"
            )
        case (ETIMEDOUT, .tcp):
            return CosError.unreachable(address: endpoint.description, detail: "It didn't answer in time. Is it awake and on your tailnet?")
        case (EHOSTUNREACH, .tcp), (ENETUNREACH, .tcp), (EHOSTDOWN, .tcp), (ENETDOWN, .tcp), (EADDRNOTAVAIL, .tcp):
            return CosError.unreachable(address: endpoint.description, detail: "There's no route to it. Is Tailscale on, on this device?")
        case (ECONNRESET, _), (EPIPE, _), (ENOTCONN, _):
            return CosError.connectionClosed
        case (ECANCELED, _):
            return CancellationError()
        default:
            return CosError.transport(String(cString: strerror(code)))
        }
    }
}
