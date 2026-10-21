import Foundation
import Network
import os

/// A stream connection through Network.framework. URLSession can't reach a
/// Unix socket; NWConnection can, and the same code will later carry TCP to
/// the daemon over the tailnet.
final class SocketConnection: ByteReader {
    private let connection: NWConnection
    private let queue = DispatchQueue(label: "cos.socket")
    private let socketPath: String

    init(unixPath: String) {
        socketPath = unixPath
        connection = NWConnection(to: .unix(path: unixPath), using: .tcp)
    }

    /// Connects, mapping "no socket" and "nobody listening" to `.notRunning`.
    func open() async throws {
        // State callbacks keep coming after we've resumed; only the first counts.
        let resumed = OSAllocatedUnfairLock(initialState: false)
        let socketPath = socketPath
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                connection.stateUpdateHandler = { [connection] state in
                    let result: Result<Void, Error>?
                    switch state {
                    case .ready:
                        result = .success(())
                    // A Unix socket that can't connect reports "waiting" rather
                    // than "failed", since Network would retry it forever.
                    case .waiting(let error), .failed(let error):
                        connection.cancel()
                        result = .failure(Self.map(error, socketPath: socketPath))
                    case .cancelled:
                        result = .failure(CancellationError())
                    default:
                        result = nil
                    }
                    guard let result else { return }
                    let first = resumed.withLock { done in
                        defer { done = true }
                        return !done
                    }
                    if first { continuation.resume(with: result) }
                }
                connection.start(queue: queue)
            }
        } onCancel: {
            connection.cancel()
        }
    }

    func send(_ bytes: [UInt8]) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            connection.send(content: Data(bytes), completion: .contentProcessed { [socketPath] error in
                if let error {
                    continuation.resume(throwing: Self.map(error, socketPath: socketPath))
                } else {
                    continuation.resume()
                }
            })
        }
    }

    func read() async throws -> [UInt8]? {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<[UInt8]?, Error>) in
                connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [socketPath] data, _, isComplete, error in
                    if let data, !data.isEmpty {
                        continuation.resume(returning: Array(data))
                    } else if let error {
                        continuation.resume(throwing: Self.map(error, socketPath: socketPath))
                    } else if isComplete {
                        continuation.resume(returning: nil)
                    } else {
                        continuation.resume(returning: [])
                    }
                }
            }
        } onCancel: {
            // Ends a read that is waiting for a quiet stream.
            connection.cancel()
        }
    }

    func close() {
        connection.cancel()
    }

    private static func map(_ error: NWError, socketPath: String) -> Error {
        if case .posix(let code) = error {
            switch code {
            case .ENOENT, .ECONNREFUSED:
                return CosError.notRunning(socket: socketPath)
            case .ECANCELED:
                return CancellationError()
            case .ECONNRESET, .EPIPE:
                return CosError.connectionClosed
            default:
                break
            }
        }
        return CosError.transport(error.localizedDescription)
    }
}
