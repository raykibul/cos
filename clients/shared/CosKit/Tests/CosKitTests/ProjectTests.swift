import Foundation
import Testing
@testable import CosKit

/// Payloads in the shape of the contract in docs/daemon.md.
enum ProjectSamples {
    static let lisbon = #"{"id":3,"project_id":1,"name":"lisbon","title":"Fix the login flow","branch":"cos/lisbon","path":"/Users/me/.cos/worktrees/cos-1/lisbon","base":"origin","base_ref":"origin/main","base_commit":"3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f","created_at":"2026-10-22T09:01:00.123456Z"}"#
    static let porto = #"{"id":4,"project_id":1,"name":"porto","title":"porto","branch":"cos/porto","path":"/Users/me/.cos/worktrees/cos-1/porto","base":"head","base_ref":"HEAD","base_commit":"9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e","created_at":"2026-10-22T09:05:00Z"}"#
    static let cos = #"{"id":1,"name":"cos","root":"/Users/me/personal/apps/cos","base_branch":"main","has_origin":true,"created_at":"2026-10-22T09:00:00Z","workspaces":[\#(lisbon),\#(porto)]}"#
    static let scratch = #"{"id":2,"name":"scratch","root":"/Users/me/scratch","base_branch":"trunk","has_origin":false,"created_at":"2026-10-22T09:10:00.5Z","workspaces":[]}"#
    static let page = #"{"projects":[\#(cos),\#(scratch)]}"#
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.cos().decode(type, from: Data(json.utf8))
}

@Suite struct ProjectModelTests {
    @Test func decodesProjectsPage() throws {
        let page = try decode(ProjectsPage.self, ProjectSamples.page)
        #expect(page.projects.map(\.name) == ["cos", "scratch"])

        let cos = page.projects[0]
        #expect(cos.root == "/Users/me/personal/apps/cos")
        #expect(cos.baseBranch == "main")
        #expect(cos.hasOrigin)
        #expect(cos.createdAt == Date(timeIntervalSince1970: 1_792_659_600))
        #expect(cos.workspaces.map(\.name) == ["lisbon", "porto"])
        #expect(!page.projects[1].hasOrigin)
        #expect(page.projects[1].workspaces.isEmpty)
    }

    @Test func decodesWorkspace() throws {
        let lisbon = try decode(Workspace.self, ProjectSamples.lisbon)
        #expect(lisbon.id == 3)
        #expect(lisbon.projectId == 1)
        #expect(lisbon.title == "Fix the login flow")
        #expect(lisbon.branch == "cos/lisbon")
        #expect(lisbon.base == .origin)
        #expect(lisbon.baseRef == "origin/main")
        #expect(lisbon.shortCommit == "3f2c9a1")

        let porto = try decode(Workspace.self, ProjectSamples.porto)
        #expect(porto.base == .head)
    }

    @Test func toleratesMissingWorkspacesAndNewBases() throws {
        let bare = try decode(Project.self, #"{"id":5,"name":"x","root":"/x","base_branch":"main","has_origin":true,"created_at":"2026-10-22T09:00:00Z"}"#)
        #expect(bare.workspaces.isEmpty)

        let future = try decode(Workspace.self, ProjectSamples.lisbon.replacingOccurrences(of: #""base":"origin""#, with: #""base":"branch""#))
        #expect(future.base.rawValue == "branch")
    }

    @Test func projectEventsTriggerRefresh() {
        func event(_ kind: String) -> LoggedEvent { LoggedEvent(seq: 1, ts: .now, kind: kind, data: .null) }
        #expect(event(EventKind.projectAdded).changesProjects)
        #expect(event(EventKind.workspaceRenamed).changesProjects)
        #expect(event("workspace.archived").changesProjects)
        #expect(!event(EventKind.daemonStarted).changesProjects)
        #expect(!event("projects").changesProjects)
    }
}

@Suite struct ErrorResponseTests {
    @Test func mapsDaemonErrorsByStatus() {
        func error(_ status: Int, _ body: String) -> CosError {
            CosError.fromResponse(status: status, body: Array(body.utf8), path: "/v1/projects")
        }
        let conflict = error(409, #"{"error":"cos has 2 workspaces; remove them first"}"#)
        #expect(conflict == .conflict("cos has 2 workspaces; remove them first"))
        #expect(conflict.localizedDescription == "cos has 2 workspaces; remove them first")

        #expect(error(404, #"{"error":"no workspace 9"}"#) == .notFound("no workspace 9"))
        #expect(error(400, #"{"error":"/tmp is not a git repository"}"#) == .http(status: 400, message: "/tmp is not a git repository"))
        #expect(error(400, #"{"error":"/tmp is not a git repository"}"#).localizedDescription == "/tmp is not a git repository")
    }

    @Test func fallsBackForBodiesThatAreNotDaemonErrors() {
        // An older daemon without the route: axum's bare 404.
        #expect(CosError.fromResponse(status: 404, body: [], path: "/v1/projects") == .notFound("The cos daemon answered 404 for /v1/projects."))
        // axum's own rejection of a bad JSON body is plain text.
        #expect(CosError.fromResponse(status: 422, body: Array("Failed to deserialize the JSON body\n".utf8), path: "/v1/projects")
            == .http(status: 422, message: "Failed to deserialize the JSON body"))
    }

    @Test func requestWithJSONBody() {
        let body = Array(#"{"path":"/r"}"#.utf8)
        let request = String(decoding: httpRequest(method: "POST", path: "/v1/projects", json: body), as: UTF8.self)
        #expect(request == "POST /v1/projects HTTP/1.1\r\nHost: cos\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"path\":\"/r\"}")
    }

    @Test func noContentHasNoBody() async throws {
        // The reader would report "closed" if asked for more; 204 must not ask.
        let response = try await HTTPResponse(reader: ScriptedReader(["HTTP/1.1 204 No Content\r\n\r\n"]))
        #expect(try await response.readBody().isEmpty)
    }
}

@Suite(.serialized) struct ProjectClientTests {
    @Test func sendsEachCallAsTheContractSays() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("GET", "/v1/projects"): .json(200, ProjectSamples.page)
            case ("POST", "/v1/projects"): .json(201, ProjectSamples.scratch)
            case ("DELETE", "/v1/projects/2"): .empty(204)
            case ("POST", "/v1/projects/1/workspaces"): .json(201, ProjectSamples.porto)
            case ("PATCH", "/v1/workspaces/3"): .json(200, ProjectSamples.lisbon)
            case ("DELETE", "/v1/workspaces/4"): .empty(204)
            default: .json(404, #"{"error":"unexpected"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.projects().count == 2)
        #expect(try await client.addProject(path: "/Users/me/scratch").id == 2)
        try await client.removeProject(id: 2)
        #expect(try await client.createWorkspace(projectId: 1, base: .head, title: "Try the new parser").name == "porto")
        _ = try await client.createWorkspace(projectId: 1)
        _ = try await client.createWorkspace(projectId: 1, title: "")
        #expect(try await client.renameWorkspace(id: 3, title: "Fix the login flow").title == "Fix the login flow")
        try await client.removeWorkspace(id: 4)

        let requests = server.requests
        #expect(requests.map { "\($0.method) \($0.path)" } == [
            "GET /v1/projects",
            "POST /v1/projects",
            "DELETE /v1/projects/2",
            "POST /v1/projects/1/workspaces",
            "POST /v1/projects/1/workspaces",
            "POST /v1/projects/1/workspaces",
            "PATCH /v1/workspaces/3",
            "DELETE /v1/workspaces/4",
        ])
        #expect(requests[1].json == ["path": "/Users/me/scratch"])
        #expect(requests[1].headers["content-type"] == "application/json")
        #expect(requests[3].json == ["base": "head", "title": "Try the new parser"])
        // No title: the daemon picks the name, so none is sent.
        #expect(requests[4].json == ["base": "origin"])
        #expect(requests[5].json == ["base": "origin"])
        #expect(requests[6].json == ["title": "Fix the login flow"])
        #expect(requests[0].body.isEmpty && requests[2].body.isEmpty)
    }

    @Test func surfacesTheDaemonsMessages() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("DELETE", "/v1/projects/1"): .json(409, #"{"error":"cos still has 2 workspaces. Remove them first."}"#)
            case ("DELETE", "/v1/workspaces/3"): .json(409, #"{"error":"lisbon has uncommitted changes. Commit or discard them first."}"#)
            case ("PATCH", "/v1/workspaces/99"): .json(404, #"{"error":"no workspace with id 99"}"#)
            case ("POST", "/v1/projects"): .json(400, #"{"error":"/tmp is not inside a git repository"}"#)
            default: .empty(500)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        await #expect(throws: CosError.conflict("cos still has 2 workspaces. Remove them first.")) { try await client.removeProject(id: 1) }
        await #expect(throws: CosError.conflict("lisbon has uncommitted changes. Commit or discard them first.")) { try await client.removeWorkspace(id: 3) }
        await #expect(throws: CosError.notFound("no workspace with id 99")) { try await client.renameWorkspace(id: 99, title: "x") }
        await #expect(throws: CosError.http(status: 400, message: "/tmp is not inside a git repository")) { try await client.addProject(path: "/tmp") }
        await #expect(throws: CosError.http(status: 500, message: "The cos daemon answered 500 for /v1/projects.")) { try await client.projects() }
    }
}

@MainActor
@Suite(.serialized) struct MonitorProjectTests {
    private func waitFor(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            if ContinuousClock.now > deadline { Issue.record("timed out"); return }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test func refreshesOnConnectAndOnProjectEvents() async throws {
        let event = #"{"seq":2,"ts":"2026-10-22T09:00:00Z","kind":"project.added","data":{"project_id":1,"name":"cos","root":"/Users/me/personal/apps/cos"}}"#
        let server = try FakeDaemon { request in
            switch request.path {
            case "/v1/status": .json(200, Samples.status.replacingOccurrences(of: #""last_seq":1"#, with: #""last_seq":2"#))
            case let path where path.hasPrefix("/v1/events/stream"): .stream("id: 2\ndata: \(event)\n\n")
            case "/v1/projects": .json(200, ProjectSamples.page)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))
        monitor.start()
        defer { monitor.stop() }

        try await waitFor { monitor.events.count == 1 && monitor.projects.count == 2 }
        // Give a coalesced follow-up refresh time to land.
        try await Task.sleep(for: .milliseconds(200))
        let fetches = server.requests.filter { $0.path == "/v1/projects" }.count
        // One on connect, one for the project.added event (or folded into one follow-up).
        #expect(fetches == 2)
        #expect(monitor.projects.map(\.name) == ["cos", "scratch"])
        #expect(monitor.workspace(id: 4)?.name == "porto")
        #expect(monitor.projectsError == nil)
    }

    @Test func actionsUpdateTheListAtOnce() async throws {
        // The list endpoint fails, so anything that changes comes from the actions.
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/projects"): .json(201, ProjectSamples.cos.replacingOccurrences(of: #""workspaces":[\#(ProjectSamples.lisbon),\#(ProjectSamples.porto)]"#, with: #""workspaces":[\#(ProjectSamples.lisbon)]"#))
            case ("POST", "/v1/projects/1/workspaces"): .json(201, ProjectSamples.porto)
            case ("PATCH", "/v1/workspaces/4"): .json(200, ProjectSamples.porto.replacingOccurrences(of: #""title":"porto""#, with: #""title":"Parser""#))
            case ("DELETE", "/v1/workspaces/3"): .empty(204)
            case ("DELETE", "/v1/projects/1"): .json(409, #"{"error":"cos still has 1 workspace. Remove it first."}"#)
            default: .json(503, #"{"error":"busy"}"#)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))

        try await monitor.addProject(path: "/Users/me/personal/apps/cos")
        #expect(monitor.projects.map(\.id) == [1])
        let porto = try await monitor.createWorkspace(projectId: 1, base: .head, title: nil)
        #expect(monitor.project(id: 1)?.workspaces.map(\.name) == ["lisbon", "porto"])
        try await monitor.renameWorkspace(porto, title: "Parser")
        #expect(monitor.workspace(id: 4)?.title == "Parser")
        try await monitor.removeWorkspace(monitor.workspace(id: 3)!)
        #expect(monitor.project(id: 1)?.workspaces.map(\.name) == ["porto"])

        await #expect(throws: CosError.conflict("cos still has 1 workspace. Remove it first.")) { try await monitor.removeProject(id: 1) }
        #expect(monitor.projects.count == 1)

        // The refreshes after each action failed, and say why.
        try await waitFor { monitor.projectsError != nil }
        #expect(monitor.projectsError == "busy")
    }
}
