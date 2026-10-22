import Foundation
import Testing
@testable import CosKit

/// Projects and workspaces end to end against the real daemon. Gated like
/// `LiveDaemonTests`. Each run uses a fresh cos home under `COS_E2E_HOME`
/// and throwaway git repos beside it, and removes what it creates.
@MainActor
@Suite(.enabled(if: liveBinary != nil && liveHome != nil, "set COS_E2E_BINARY and COS_E2E_HOME"), .serialized)
struct LiveProjectTests {
    @Test func projectsAndWorkspacesFollowTheContract() async throws {
        let run = String(UUID().uuidString.prefix(6)).lowercased()
        let home = "\(liveHome!)/p-\(run)"
        let repos = "\(liveHome!)/repos-\(run)"
        // Runs last, after the daemon has stopped.
        defer {
            try? FileManager.default.removeItem(atPath: repos)
            try? FileManager.default.removeItem(atPath: home)
        }

        // A repo with an origin (a local bare repo) and one without.
        let origin = "\(repos)/origin.git"
        let work = "\(repos)/work"
        let local = "\(repos)/local"
        let plain = "\(repos)/not-a-repo"
        try FileManager.default.createDirectory(atPath: "\(work)/src", withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: local, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: plain, withIntermediateDirectories: true)
        try git(repos, "init", "--bare", "-b", "main", origin)
        try git(work, "init", "-b", "main")
        try commit(work, "README.md", "hello")
        try git(work, "remote", "add", "origin", origin)
        try git(work, "push", "-u", "origin", "main")
        try git(work, "remote", "set-head", "origin", "main")
        try git(local, "init", "-b", "trunk")
        try commit(local, "a.txt", "a")

        // The repos sit inside the cos checkout; stop git looking above them,
        // so "not a repo" really isn't one.
        let daemon = try startLiveDaemon(home: home, environment: ["GIT_CEILING_DIRECTORIES": repos])
        defer { daemon.terminate(); daemon.waitUntilExit() }
        let client = CosClient(socketPath: "\(home)/cos.sock")
        _ = try await retrying { try await client.status() }

        let monitor = DaemonMonitor(client: client)
        monitor.start()
        defer { monitor.stop() }

        // Registering a subfolder registers the main checkout, once.
        let project = try await client.addProject(path: "\(work)/src")
        #expect(resolved(project.root) == resolved(work))
        #expect(project.hasOrigin)
        #expect(project.baseBranch == "main")
        #expect(project.workspaces.isEmpty)
        #expect(try await client.addProject(path: work).id == project.id)

        await #expect { try await client.addProject(path: plain) } throws: { error in
            guard case CosError.http(400, let message) = error else { return false }
            return !message.isEmpty
        }

        // From origin: exactly origin/main.
        let fromOrigin = try await client.createWorkspace(projectId: project.id, base: .origin, title: "E2E origin")
        #expect(fromOrigin.title == "E2E origin")
        #expect(fromOrigin.branch == "cos/\(fromOrigin.name)")
        #expect(fromOrigin.base == .origin)
        #expect(fromOrigin.baseRef == "origin/main")
        #expect(fromOrigin.baseCommit == (try git(work, "rev-parse", "origin/main")))
        #expect(FileManager.default.fileExists(atPath: "\(fromOrigin.path)/README.md"))
        #expect(fromOrigin.path.hasPrefix("\(home)/worktrees/"))

        // From HEAD: sees an unpushed commit.
        try commit(work, "unpushed.txt", "local only")
        let fromHead = try await client.createWorkspace(projectId: project.id, base: .head)
        #expect(fromHead.base == .head)
        #expect(fromHead.title == nil)
        #expect(fromHead.displayName == fromHead.city)
        #expect(fromHead.baseCommit == (try git(work, "rev-parse", "HEAD")))
        #expect(fromHead.name != fromOrigin.name)

        // The monitor refreshes from the events alone.
        try await waitUntil { monitor.project(id: project.id)?.workspaces.count == 2 }
        #expect(monitor.events.contains { $0.kind == EventKind.workspaceCreated })

        let renamed = try await client.renameWorkspace(id: fromHead.id, title: "Renamed by e2e")
        #expect(renamed.title == "Renamed by e2e")
        try await waitUntil { monitor.workspace(id: fromHead.id)?.title == "Renamed by e2e" }

        // Without an origin: "origin" is refused with a message, "head" works.
        let localProject = try await client.addProject(path: local)
        #expect(!localProject.hasOrigin)
        #expect(localProject.baseBranch == "trunk")
        await #expect(throws: CosError.self) { try await client.createWorkspace(projectId: localProject.id, base: .origin) }
        let localWorkspace = try await client.createWorkspace(projectId: localProject.id, base: .head)

        // Conflicts carry the daemon's message.
        await #expect { try await client.removeProject(id: project.id) } throws: { error in
            guard case CosError.conflict(let message) = error else { return false }
            return !message.isEmpty
        }
        try "dirty".write(toFile: "\(fromOrigin.path)/dirty.txt", atomically: true, encoding: .utf8)
        await #expect { try await client.removeWorkspace(id: fromOrigin.id) } throws: { error in
            guard case CosError.conflict(let message) = error else { return false }
            return !message.isEmpty
        }
        try FileManager.default.removeItem(atPath: "\(fromOrigin.path)/dirty.txt")

        await #expect { try await client.renameWorkspace(id: 999_999, title: "x") } throws: { error in
            guard case CosError.notFound(let message) = error else { return false }
            return !message.isEmpty
        }

        // Removing keeps the branch and deletes the folder.
        for workspace in [fromOrigin, fromHead] {
            try await client.removeWorkspace(id: workspace.id)
            #expect(!FileManager.default.fileExists(atPath: workspace.path))
            #expect(try git(work, "branch", "--list", workspace.branch).contains(workspace.branch))
        }
        try await client.removeWorkspace(id: localWorkspace.id)
        try await client.removeProject(id: project.id)
        try await client.removeProject(id: localProject.id)
        #expect(try await client.projects().isEmpty)
        try await waitUntil { monitor.projects.isEmpty }

        let kinds = Set(try await client.events(after: 0, limit: 1000).map(\.kind))
        #expect(kinds.isSuperset(of: [
            EventKind.projectAdded, EventKind.projectRemoved,
            EventKind.workspaceCreated, EventKind.workspaceRenamed, EventKind.workspaceRemoved,
        ]))
    }

    @discardableResult
    private func git(_ directory: String, _ arguments: String...) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        // Independent of the user's git config (identity, signing).
        process.arguments = ["-c", "user.name=cos e2e", "-c", "user.email=e2e@cos.invalid", "-c", "commit.gpgsign=false"] + arguments
        process.currentDirectoryURL = URL(fileURLWithPath: directory)
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw CosError.transport("git \(arguments.joined(separator: " ")) failed") }
        return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func commit(_ repo: String, _ file: String, _ text: String) throws {
        try text.write(toFile: "\(repo)/\(file)", atomically: true, encoding: .utf8)
        try git(repo, "add", file)
        try git(repo, "commit", "-m", "add \(file)")
    }

    private func resolved(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out")
                return
            }
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}

let liveBinary = ProcessInfo.processInfo.environment["COS_E2E_BINARY"]
let liveHome = ProcessInfo.processInfo.environment["COS_E2E_HOME"]

func startLiveDaemon(home: String, environment extra: [String: String] = [:]) throws -> Process {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: liveBinary!)
    process.arguments = ["daemon"]
    process.environment = ProcessInfo.processInfo.environment.merging(extra.merging(["COS_HOME": home]) { $1 }) { $1 }
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    try process.run()
    return process
}

@MainActor func retrying<T>(_ body: () async throws -> T) async throws -> T {
    let deadline = ContinuousClock.now + .seconds(10)
    while true {
        do {
            return try await body()
        } catch {
            if ContinuousClock.now > deadline { throw error }
            try await Task.sleep(for: .milliseconds(100))
        }
    }
}
