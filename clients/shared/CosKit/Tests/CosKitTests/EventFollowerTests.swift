import Foundation
import os
import Testing
@testable import CosKit

/// A daemon that behaves according to a script, one step per connection
/// attempt. Once the script runs out it is "not running".
final class FakeSource: EventSource {
    enum Step {
        case notRunning
        /// Answer the status, stream these events, then drop the connection.
        case serve(lastSeq: Int, events: [Int])
    }

    private struct State {
        var steps: [Step]
        var current: [Int] = []
        var streamedAfter: [Int] = []
    }

    private let state: OSAllocatedUnfairLock<State>

    init(_ steps: [Step]) {
        state = OSAllocatedUnfairLock(initialState: State(steps: steps))
    }

    var streamedAfter: [Int] { state.withLock { $0.streamedAfter } }

    func status() async throws -> Status {
        let step = state.withLock { $0.steps.isEmpty ? Step.notRunning : $0.steps.removeFirst() }
        switch step {
        case .notRunning:
            throw CosError.notRunning(socket: "/fake.sock")
        case .serve(let lastSeq, let events):
            state.withLock { $0.current = events }
            return status(lastSeq: lastSeq)
        }
    }

    func stream(after: Int) -> AsyncThrowingStream<LoggedEvent, Error> {
        let events = state.withLock { state in
            state.streamedAfter.append(after)
            return state.current
        }
        return AsyncThrowingStream { continuation in
            for seq in events {
                continuation.yield(event(seq))
            }
            continuation.finish(throwing: CosError.connectionClosed)
        }
    }

    func status(lastSeq: Int) -> Status {
        Status(version: "0.1.0", pid: 1, startedAt: Date(timeIntervalSince1970: 0), home: "/fake", events: lastSeq, lastSeq: lastSeq)
    }

    func event(_ seq: Int) -> LoggedEvent {
        LoggedEvent(seq: seq, ts: Date(timeIntervalSince1970: Double(seq)), kind: "test", data: .object(["n": .number(Double(seq))]))
    }
}

/// Collects updates until `done` says so (or a safety limit is hit).
private func collect(
    _ follower: EventFollower,
    after: Int? = nil,
    until done: ([FollowUpdate]) -> Bool
) async -> [FollowUpdate] {
    var updates: [FollowUpdate] = []
    for await update in follower.updates(after: after) {
        updates.append(update)
        if done(updates) || updates.count > 200 { break }
    }
    return updates
}

private func seqs(_ updates: [FollowUpdate]) -> [Int] {
    updates.compactMap { if case .event(let event) = $0 { event.seq } else { nil } }
}

private let instant = Backoff(initial: .zero, maximum: .zero)

@Suite struct EventFollowerTests {
    @Test func resumesAfterTheLastEventAcrossRestarts() async {
        let source = FakeSource([
            .notRunning,                         // not started yet
            .serve(lastSeq: 2, events: [1, 2]),  // runs, then stops
            .notRunning,                         // restarting
            .serve(lastSeq: 4, events: [2, 3, 4]), // 2 again must be skipped
        ])
        let follower = EventFollower(source: source, backlog: 100, backoff: instant)

        let updates = await collect(follower) { seqs($0).last == 4 }

        #expect(seqs(updates) == [1, 2, 3, 4])
        #expect(source.streamedAfter == [0, 2])
        let notRunning = CosError.notRunning(socket: "/fake.sock")
        #expect(updates == [
            .disconnected(notRunning, retryIn: .zero),
            .connected(source.status(lastSeq: 2)),
            .event(source.event(1)),
            .event(source.event(2)),
            .disconnected(.connectionClosed, retryIn: .zero),
            .disconnected(notRunning, retryIn: .zero),
            .connected(source.status(lastSeq: 4)),
            .event(source.event(3)),
            .event(source.event(4)),
        ])
    }

    @Test func startsFromAGivenSequenceNumber() async {
        let source = FakeSource([.serve(lastSeq: 9, events: [8, 9])])
        let follower = EventFollower(source: source, backoff: instant)

        let updates = await collect(follower, after: 7) { seqs($0).last == 9 }

        #expect(source.streamedAfter == [7])
        #expect(seqs(updates) == [8, 9])
    }

    @Test func firstConnectStartsABacklogBeforeTheNewest() async {
        let source = FakeSource([.serve(lastSeq: 500, events: [499, 500])])
        let follower = EventFollower(source: source, backlog: 200, backoff: instant)

        _ = await collect(follower) { seqs($0).last == 500 }

        #expect(source.streamedAfter == [300])
    }

    @Test func freshDatabaseResetsTheCursor() async {
        let source = FakeSource([
            .serve(lastSeq: 5, events: [5]),
            .serve(lastSeq: 1, events: [1]), // a new database: seq 1 again
        ])
        let follower = EventFollower(source: source, backlog: 1, backoff: instant)

        let updates = await collect(follower) { seqs($0) == [5, 1] }

        #expect(updates.contains(.logReset))
        #expect(source.streamedAfter == [4, 0])
    }

    @Test func backsOffAndResetsAfterConnecting() async {
        let source = FakeSource([.notRunning, .notRunning, .notRunning, .serve(lastSeq: 0, events: [])])
        // Tiny real delays, so the test stays fast but the values are distinct.
        let backoff = Backoff(initial: .milliseconds(1), maximum: .milliseconds(3))
        let follower = EventFollower(source: source, backoff: backoff)

        let updates = await collect(follower) { $0.count == 6 }

        let delays = updates.compactMap { if case .disconnected(_, let delay) = $0 { delay } else { nil } }
        #expect(delays == [.milliseconds(1), .milliseconds(2), .milliseconds(3), .milliseconds(1), .milliseconds(2)])
    }

    @Test func backoffDelays() {
        let backoff = Backoff()
        #expect((0..<6).map(backoff.delay(attempt:)) == [.milliseconds(500), .seconds(1), .seconds(2), .seconds(4), .seconds(5), .seconds(5)])
        #expect(backoff.delay(attempt: 10_000) == .seconds(5))
    }

    @Test func stopsWhenTheConsumerStops() async throws {
        let source = FakeSource([])
        let follower = EventFollower(source: source, backoff: Backoff(initial: .seconds(60), maximum: .seconds(60)))
        let updates = await collect(follower) { _ in true }
        #expect(updates == [.disconnected(.notRunning(socket: "/fake.sock"), retryIn: .seconds(60))])
        // Breaking out cancels the follower's sleep; nothing else is asked of the source.
        try await Task.sleep(for: .milliseconds(50))
        #expect(source.streamedAfter.isEmpty)
    }
}

@MainActor
@Suite struct DaemonMonitorTests {
    @Test func tracksConnectionStatusAndEvents() {
        let source = FakeSource([])
        let monitor = DaemonMonitor(client: CosClient(socketPath: "/fake.sock"), keep: 2)
        #expect(monitor.connection == .connecting)

        monitor.apply(.connected(source.status(lastSeq: 1)))
        #expect(monitor.connection == .running)
        for seq in 1...3 { monitor.apply(.event(source.event(seq))) }

        #expect(monitor.events.map(\.seq) == [2, 3])
        #expect(monitor.status?.lastSeq == 3)
        #expect(monitor.status?.events == 3)

        monitor.apply(.disconnected(.connectionClosed, retryIn: .zero))
        #expect(monitor.connection == .connecting)
        monitor.apply(.disconnected(.notRunning(socket: "/fake.sock"), retryIn: .zero))
        #expect(monitor.connection == .notRunning)
        #expect(monitor.status == nil)
        // History stays visible while the daemon is away.
        #expect(monitor.events.map(\.seq) == [2, 3])

        monitor.apply(.disconnected(.http(status: 500, message: "boom"), retryIn: .zero))
        #expect(monitor.connection == .failed("boom"))
    }
}
