import Foundation
import Testing
@testable import CosKit

/// Runs end to end against the real daemon, gated like `LiveProjectTests`.
/// The plan is proposed through the raw route, standing in for the chief.
/// Routes another piece hasn't merged yet (questions, messages, reports)
/// must answer 404, which the app shows as "not supported yet".
@MainActor
@Suite(.enabled(if: liveBinary != nil && liveHome != nil, "set COS_E2E_BINARY and COS_E2E_HOME"), .serialized)
struct LiveRunTests {
    @Test func runsFollowTheContract() async throws {
        let id = String(UUID().uuidString.prefix(6)).lowercased()
        let home = "\(liveHome!)/r-\(id)"
        let repo = "\(liveHome!)/repo-\(id)"
        defer {
            try? FileManager.default.removeItem(atPath: repo)
            try? FileManager.default.removeItem(atPath: home)
        }
        try FileManager.default.createDirectory(atPath: repo, withIntermediateDirectories: true)
        try git(repo, "init", "-b", "main")
        try "hi".write(toFile: "\(repo)/README.md", atomically: true, encoding: .utf8)
        try git(repo, "add", "README.md")
        try git(repo, "commit", "-m", "init")

        // A fake claude, in case this daemon starts a chief for new runs.
        var environment = ["GIT_CEILING_DIRECTORIES": liveHome!]
        let fake = URL(fileURLWithPath: liveBinary!).deletingLastPathComponent().appendingPathComponent("cos-fake-claude").path
        if FileManager.default.fileExists(atPath: fake) { environment["COS_CLAUDE_BIN"] = fake }
        let daemon = try startLiveDaemon(home: home, environment: environment)
        defer { daemon.terminate(); daemon.waitUntilExit() }
        let client = CosClient(socketPath: "\(home)/cos.sock")
        _ = try await retrying { try await client.status() }

        let monitor = DaemonMonitor(client: client, runsDebounce: .milliseconds(20))
        monitor.start()
        defer { monitor.stop() }

        let project = try await client.addProject(path: repo)
        let workspace = try await client.createWorkspace(projectId: project.id, base: .head)

        let run = try await monitor.createRun(NewRun(workspaceId: workspace.id, goal: "add a greeting\nand test it", preset: .trusted, approvalCap: 7))
        #expect(run.state == .planning)
        #expect(run.goal == "add a greeting\nand test it")
        #expect(run.preset == .trusted && run.approvalCap == 7)
        await #expect(throws: CosError.self) { _ = try await client.createRun(NewRun(workspaceId: workspace.id, goal: "another")) }
        try await waitUntil { monitor.currentRun(inWorkspace: workspace.id)?.id == run.id }

        monitor.watchRun(run.id)
        defer { monitor.unwatchRun(run.id) }

        // The chief's part: propose a plan.
        struct Plan: Encodable {
            struct Task: Encodable {
                var title: String
                var instructions: String
                var files: [String]
                var depends_on: [String] = []
                var test_command: String?
            }
            var tasks: [Task]
        }
        let proposed: Board = try await client.sendJSON("POST", "/v1/runs/\(run.id)/plan", body: Plan(tasks: [
            .init(title: "Greeting", instructions: "Add greet()", files: ["src/greet.rs"], test_command: "cargo test greet"),
            .init(title: "Docs", instructions: "Document greet()", files: ["docs/greet.md"], depends_on: ["Greeting"]),
        ]))
        #expect(proposed.run.state == .awaitingPlanApproval)
        try await waitUntil { monitor.boards[run.id]?.run.state == .awaitingPlanApproval && monitor.plansAwaitingApproval.map(\.id) == [run.id] }
        let board = monitor.boards[run.id]!
        #expect(board.tasks.map(\.title) == ["Greeting", "Docs"])
        #expect(board.tasks[0].testCommand == "cargo test greet")
        #expect(board.tasks[1].dependsOn == [board.tasks[0].id])
        #expect(board.workspace?.id == workspace.id)
        #expect(monitor.waitingCount >= 1)

        let approved = try await monitor.approvePlan(runId: run.id)
        #expect(approved.run.state == .running)
        #expect(approved.task(id: board.tasks[0].id)?.state == .ready)
        #expect(approved.task(id: board.tasks[1].id)?.state == .proposed)
        await #expect(throws: CosError.self) { try await monitor.approvePlan(runId: run.id) }

        // What the app offers on a blocked task: retry once (with a reason), then fail.
        let first = approved.tasks[0]
        await #expect(throws: CosError.self) { try await monitor.updateTask(first, to: .failed, reason: "not ready yet") }
        let blocked = try await monitor.updateTask(first, to: .blocked, reason: "stuck")
        let stuck = try #require(blocked.task(id: first.id))
        #expect(stuck.state == .blocked && stuck.canRetry && stuck.canFail)
        await #expect(throws: CosError.self) { try await monitor.updateTask(stuck, to: .ready) }
        let retried = try await monitor.updateTask(stuck, to: .ready, reason: "try again")
        #expect(retried.task(id: first.id)?.state == .ready)
        #expect(retried.task(id: first.id)?.retries == 1)
        let again = try #require(try await monitor.updateTask(stuck, to: .blocked, reason: "stuck again").task(id: first.id))
        #expect(!again.canRetry && again.canFail)
        await #expect(throws: CosError.self) { try await monitor.updateTask(again, to: .ready, reason: "once more") }
        let failed = try await monitor.updateTask(again, to: .failed, reason: "not needed")
        #expect(failed.task(id: board.tasks[0].id)?.state == .failed)
        #expect(failed.task(id: board.tasks[0].id)?.lastFailure == "not needed")
        #expect(failed.task(id: board.tasks[1].id)?.state == .blocked)

        // Limits, if this daemon has them.
        do {
            let limits = try await client.limits(runId: run.id)
            #expect(limits.preset == .trusted && limits.approvalCap == 7 && limits.chiefApprovals == 0)
            let changed = try await monitor.setLimits(runId: run.id, preset: .strict, approvalCap: 3)
            #expect(changed.preset == .strict && changed.approvalCap == 3)
        } catch CosError.notFound {}

        // Routes of pieces that may not be merged yet: 404 or the real thing.
        do { _ = try await client.questions() } catch CosError.notFound {}
        do { try await client.messageChief(runId: run.id, text: "hi") } catch CosError.notFound {}
        do { _ = try await client.report(runId: run.id) } catch CosError.notFound {}

        let cancelled = try await monitor.cancelRun(id: run.id, reason: "done testing")
        #expect(cancelled.run.state == .cancelled)
        #expect(cancelled.run.reason == "done testing")
        #expect(monitor.currentRun(inWorkspace: workspace.id) == nil)
        await #expect(throws: CosError.self) { try await monitor.cancelRun(id: run.id) }
        try await waitUntil { monitor.runs.first { $0.id == run.id }?.state == .cancelled && monitor.plansAwaitingApproval.isEmpty }

        // The run list carries counts.
        let listed = try await client.runs().first { $0.id == run.id }
        #expect(listed?.counts?.total == 2)
        #expect(listed?.counts?.failed == 1)
    }

    @discardableResult
    private func git(_ directory: String, _ arguments: String...) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-c", "user.name=cos e2e", "-c", "user.email=e2e@cos.invalid", "-c", "commit.gpgsign=false"] + arguments
        process.currentDirectoryURL = URL(fileURLWithPath: directory)
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        process.waitUntilExit()
        let text = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        guard process.terminationStatus == 0 else { throw CosError.transport("git \(arguments.joined(separator: " ")): \(text)") }
        return text
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            if ContinuousClock.now > deadline { Issue.record("timed out"); return }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}
