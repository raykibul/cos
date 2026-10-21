import Foundation
import os
import Testing
@testable import CosKit

/// End to end against the real daemon. Skipped unless `COS_E2E_BINARY`
/// (path to a built `cos`) and `COS_E2E_HOME` (a scratch `COS_HOME`) are set.
/// The test starts and stops the daemon itself.
private let environment = ProcessInfo.processInfo.environment
private let binary = environment["COS_E2E_BINARY"]
private let home = environment["COS_E2E_HOME"]

@Suite(.enabled(if: binary != nil && home != nil, "set COS_E2E_BINARY and COS_E2E_HOME"), .serialized)
struct LiveDaemonTests {
    @Test func statusEventsAndResumeAcrossARestart() async throws {
        let client = CosClient(socketPath: CosClient.defaultSocketPath(environment: ["COS_HOME": home!]))

        // Nothing running yet: a clear "not running", not some other error.
        await #expect(throws: CosError.notRunning(socket: client.socketPath)) { try await client.status() }

        let updates = Updates()
        let following = Task {
            for await update in EventFollower(source: client, backlog: 1).updates() {
                updates.append(update)
            }
        }
        defer { following.cancel() }

        var daemon = try startDaemon()
        defer { if daemon.isRunning { daemon.terminate(); daemon.waitUntilExit() } }

        let status = try await eventually { try await client.status() }
        #expect(status.pid == Int(daemon.processIdentifier))
        #expect(status.home == home)
        #expect(status.events >= 1)
        let firstPid = status.pid

        // The follower connects and, with a backlog of 1, sees this daemon's daemon.started.
        try await waitFor { updates.events.contains { $0.kind == EventKind.daemonStarted && $0.data["pid"] == .number(Double(firstPid)) } }
        let page = try await client.events(after: status.lastSeq - 1, limit: 10)
        #expect(page.map(\.seq) == [status.lastSeq])

        // Stop it: the follower notices and reports "not running".
        daemon.terminate()
        daemon.waitUntilExit()
        try await waitFor { updates.all.contains { if case .disconnected(.notRunning, _) = $0 { true } else { false } } }

        // Start it again: the follower resumes without gaps or repeats.
        daemon = try startDaemon()
        let restarted = try await eventually { try await client.status() }
        #expect(restarted.pid != firstPid)
        try await waitFor { updates.events.contains { $0.kind == EventKind.daemonStarted && $0.data["pid"] == .number(Double(restarted.pid)) } }

        let seen = updates.events.map(\.seq)
        #expect(seen == Array(seen.first!...seen.last!), "events in order, no gaps or repeats: \(seen)")
        let kinds = updates.events.map(\.kind)
        #expect(kinds.suffix(3) == [EventKind.daemonStarted, EventKind.daemonStopped, EventKind.daemonStarted])
        #expect(updates.events.last(where: { $0.kind == EventKind.daemonStopped })?.data["reason"] == .string("SIGTERM"))
        let connects = updates.all.filter { if case .connected = $0 { true } else { false } }
        #expect(connects.count == 2)
    }

    private func startDaemon() throws -> Process {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary!)
        process.arguments = ["daemon"]
        process.environment = environment.merging(["COS_HOME": home!]) { $1 }
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        return process
    }
}

final class Updates: Sendable {
    private let list = OSAllocatedUnfairLock<[FollowUpdate]>(initialState: [])
    func append(_ update: FollowUpdate) { list.withLock { $0.append(update) } }
    var all: [FollowUpdate] { list.withLock { $0 } }
    var events: [LoggedEvent] { all.compactMap { if case .event(let event) = $0 { event } else { nil } } }
}

private func eventually<T>(timeout: Duration = .seconds(10), _ body: () async throws -> T) async throws -> T {
    let deadline = ContinuousClock.now + timeout
    while true {
        do {
            return try await body()
        } catch {
            if ContinuousClock.now > deadline { throw error }
            try await Task.sleep(for: .milliseconds(100))
        }
    }
}

private struct TimedOut: Error {}

private func waitFor(timeout: Duration = .seconds(15), _ condition: () -> Bool) async throws {
    let deadline = ContinuousClock.now + timeout
    while !condition() {
        if ContinuousClock.now > deadline { throw TimedOut() }
        try await Task.sleep(for: .milliseconds(50))
    }
}
