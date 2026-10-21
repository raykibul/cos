import Foundation

/// What `EventFollower` needs from the daemon. `CosClient` is the real one;
/// tests use a scripted fake.
public protocol EventSource: Sendable {
    func status() async throws -> Status
    func stream(after: Int) -> AsyncThrowingStream<LoggedEvent, Error>
}

extension CosClient: EventSource {}

public enum FollowUpdate: Equatable, Sendable {
    /// Connected; this is the daemon's status at that moment.
    case connected(Status)
    case event(LoggedEvent)
    /// The daemon's log is older than what we've seen (a fresh database),
    /// so events already shown no longer belong to it.
    case logReset
    /// Lost the daemon or couldn't reach it; trying again after `retryIn`.
    case disconnected(CosError, retryIn: Duration)
}

/// Exponential backoff for reconnecting: 0.5 s, 1 s, 2 s, then every 5 s.
/// The cap stays low so the app notices quickly when the daemon starts.
public struct Backoff: Equatable, Sendable {
    public var initial: Duration
    public var maximum: Duration

    public init(initial: Duration = .milliseconds(500), maximum: Duration = .seconds(5)) {
        self.initial = initial
        self.maximum = maximum
    }

    /// Delay before retry number `attempt`, counting from 0.
    public func delay(attempt: Int) -> Duration {
        var delay = initial
        for _ in 0..<min(attempt, 30) where delay < maximum {
            delay *= 2
        }
        return min(delay, maximum)
    }
}

/// Follows the event log for as long as it is iterated: connects, reads
/// events, and after any disconnect (the daemon restarts, or isn't running
/// yet) reconnects with backoff and resumes after the last event it saw.
/// Each event is delivered once, in order.
public struct EventFollower: Sendable {
    public var source: EventSource
    /// On the first connect, start this many events back rather than
    /// replaying the whole log.
    public var backlog: Int
    public var backoff: Backoff

    public init(source: EventSource, backlog: Int = 200, backoff: Backoff = Backoff()) {
        self.source = source
        self.backlog = backlog
        self.backoff = backoff
    }

    /// `after`: resume after this sequence number; `nil` starts `backlog`
    /// events before the newest.
    public func updates(after: Int? = nil) -> AsyncStream<FollowUpdate> {
        AsyncStream { continuation in
            let task = Task {
                await run(after: after) { continuation.yield($0) }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func run(after: Int?, yield: (FollowUpdate) -> Void) async {
        var last = after
        var attempt = 0
        while !Task.isCancelled {
            let reason: CosError
            do {
                let status = try await source.status()
                if let seen = last, status.lastSeq < seen {
                    last = nil
                    yield(.logReset)
                }
                attempt = 0
                yield(.connected(status))
                let from = last ?? max(0, status.lastSeq - backlog)
                for try await event in source.stream(after: from) {
                    // A resumed stream should start after `from`; skip anything older anyway.
                    if event.seq <= (last ?? from) { continue }
                    last = event.seq
                    yield(.event(event))
                }
                reason = .connectionClosed
            } catch is CancellationError {
                return
            } catch let error as CosError {
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
