import Foundation

/// One event of `GET /v1/agents/{id}/terminal`, with the base64 already
/// decoded. A terminal emulator resets and feeds `snapshot`, then feeds
/// every `output` in order.
public enum TerminalEvent: Equatable, Sendable {
    /// Always first on a stream: escape sequences that redraw the current
    /// screen at this size.
    case snapshot(rows: Int, cols: Int, data: [UInt8])
    /// Raw PTY bytes. May end mid escape sequence; the next chunk continues it.
    case output([UInt8])
    /// Someone resized the agent. The PTY has one size; the last resize wins.
    case resize(rows: Int, cols: Int)
    /// The process ended (`nil`: it was killed). The stream ends after this.
    case exit(code: Int?)

    /// Decodes one SSE message. Unknown events and malformed data give
    /// `nil` rather than an error: the stream carries on, as elsewhere in cos.
    public init?(_ message: SSEMessage) {
        let object = try? JSONDecoder().decode(JSONValue.self, from: Data(message.data.utf8))
        func int(_ key: String) -> Int? {
            if case .number(let value)? = object?[key], value.rounded() == value { return Int(value) }
            return nil
        }
        func bytes() -> [UInt8]? {
            guard case .string(let text)? = object?["data"] else { return nil }
            return Data(base64Encoded: text, options: .ignoreUnknownCharacters).map { Array($0) }
        }
        switch message.event {
        case "snapshot":
            guard let rows = int("rows"), let cols = int("cols"), let data = bytes() else { return nil }
            self = .snapshot(rows: rows, cols: cols, data: data)
        case "output":
            guard let data = bytes() else { return nil }
            self = .output(data)
        case "resize":
            guard let rows = int("rows"), let cols = int("cols") else { return nil }
            self = .resize(rows: rows, cols: cols)
        case "exit":
            self = .exit(code: int("exit_code"))
        default:
            return nil
        }
    }
}

/// Where a `TerminalStream` gets its connections. `CosClient` is the real one.
public protocol TerminalSource: Sendable {
    func terminal(agentId: Int) -> AsyncThrowingStream<TerminalEvent, Error>
}

extension CosClient: TerminalSource {
    /// One connection to an agent's terminal. It finishes normally after
    /// `exit`, and throws `.connectionClosed` if the daemon drops it before
    /// then. It does not reconnect; `TerminalStream` does.
    public func terminal(agentId: Int) -> AsyncThrowingStream<TerminalEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let (connection, response) = try await request(
                        "GET", "/v1/agents/\(agentId)/terminal", headers: [("Accept", "text/event-stream")]
                    )
                    defer { connection.close() }
                    var parser = SSEParser()
                    while let chunk = try await response.nextBodyChunk() {
                        for message in parser.feed(chunk) {
                            guard let event = TerminalEvent(message) else { continue }
                            continuation.yield(event)
                            if case .exit = event {
                                continuation.finish()
                                return
                            }
                        }
                    }
                    continuation.finish(throwing: CosError.connectionClosed)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

public enum TerminalUpdate: Equatable, Sendable {
    case event(TerminalEvent)
    /// The stream dropped before `exit`, or couldn't be opened. A new one
    /// (starting with a fresh snapshot) is tried after `retryIn`.
    case disconnected(CosError, retryIn: Duration)
    /// The daemon doesn't know this agent (404): removed, or a daemon
    /// without agents. Nothing more comes.
    case gone(CosError)
}

/// Follows an agent's terminal for as long as it is iterated. Each
/// connection starts with a snapshot; when one drops while the agent is
/// alive it reconnects with backoff and gets a fresh snapshot (there is no
/// resume by event id). It ends after `exit` or `gone`.
public struct TerminalStream: Sendable {
    public var source: TerminalSource
    public var agentId: Int
    public var backoff: Backoff

    public init(source: TerminalSource, agentId: Int, backoff: Backoff = Backoff()) {
        self.source = source
        self.agentId = agentId
        self.backoff = backoff
    }

    public func updates() -> AsyncStream<TerminalUpdate> {
        AsyncStream { continuation in
            let task = Task {
                await run { continuation.yield($0) }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func run(yield: (TerminalUpdate) -> Void) async {
        var attempt = 0
        while !Task.isCancelled {
            let reason: CosError
            do {
                for try await event in source.terminal(agentId: agentId) {
                    if case .snapshot = event { attempt = 0 }
                    yield(.event(event))
                    if case .exit = event { return }
                }
                // Finished without `exit`: a source that ended quietly.
                reason = .connectionClosed
            } catch is CancellationError {
                return
            } catch let error as CosError {
                if case .notFound = error {
                    yield(.gone(error))
                    return
                }
                reason = error
            } catch {
                reason = .transport(error.localizedDescription)
            }
            if Task.isCancelled { return }
            let delay = backoff.delay(attempt: attempt)
            attempt += 1
            yield(.disconnected(reason, retryIn: delay))
            do {
                try await Task.sleep(for: delay)
            } catch {
                return
            }
        }
    }
}
