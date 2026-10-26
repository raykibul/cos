import Foundation
import Testing
@testable import CosKit

/// "Shells" in docs/agents-api.md.
@Suite struct ShellTests {
    static let shell = #"{"id":3,"project_id":1,"cwd":"/w/atlas-1/lisbon","pid":4242,"started_ms":1761330000000,"exited":false,"exit_code":null}"#

    @Test func decodesShellsLeniently() throws {
        let page = try JSONDecoder.cos().decode(ShellsPage.self, from: Data(#"{"shells":[\#(Self.shell),{"cwd":"/no/id"},{"id":"4","cwd":"/w","exited":true,"exit_code":1}]}"#.utf8))
        #expect(page.shells.map(\.id) == [3, 4])
        #expect(page.shells[0].projectId == 1)
        #expect(page.shells[0].folderName == "lisbon")
        #expect(page.shells[1].exited)
        #expect(page.shells[1].exitCode == 1)
    }

    @Test func sendsEveryRouteAsTheContractSays() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("GET", "/v1/shells"): .json(200, #"{"shells":[\#(Self.shell)]}"#)
            case ("POST", "/v1/shells"): .json(201, Self.shell)
            case ("POST", "/v1/shells/3/keys"), ("POST", "/v1/shells/3/resize"), ("DELETE", "/v1/shells/3"): .empty(204)
            case ("DELETE", "/v1/shells/9"): .json(404, #"{"error":"there is no such terminal"}"#)
            default: .json(404, #"{"error":"unknown route"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.shells().map(\.id) == [3])
        #expect(try await client.startShell(cwd: "/w/atlas-1/lisbon", projectId: 1).id == 3)
        _ = try await client.startShell(cwd: "/w")
        let terminal = ShellTerminal(shells: client)
        try await terminal.sendKeys(agentId: 3, Array("ls\r".utf8))
        try await terminal.resizeAgent(id: 3, rows: 30, cols: 90)
        try await client.closeShell(id: 3)
        await #expect(throws: CosError.notFound("there is no such terminal")) { try await client.closeShell(id: 9) }

        let posts = server.requests.filter { $0.method == "POST" && $0.path == "/v1/shells" }
        #expect(posts.map(\.value) == [
            .object(["cwd": .string("/w/atlas-1/lisbon"), "project_id": .number(1)]),
            .object(["cwd": .string("/w")]),
        ])
        #expect(server.requests.first { $0.path == "/v1/shells/3/keys" }?.json == ["data": Data("ls\r".utf8).base64EncodedString()])
        #expect(server.requests.first { $0.path == "/v1/shells/3/resize" }?.value == .object(["rows": .number(30), "cols": .number(90)]))
    }

    @Test func aShellsTerminalStreamsThroughTheAgentPlumbing() async throws {
        let server = try FakeDaemon { request in
            guard request.path == "/v1/shells/3/terminal" else { return .empty(404) }
            return .stream(
                "event: snapshot\ndata: {\"rows\":24,\"cols\":80,\"data\":\"\(Data("$ ".utf8).base64EncodedString())\"}\n\n"
                    + "event: exit\ndata: {\"exit_code\":0}\n\n"
            )
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        var updates: [TerminalUpdate] = []
        for await update in TerminalStream(source: ShellTerminal(shells: client), agentId: 3, backoff: Backoff(initial: .zero, maximum: .zero)).updates() {
            updates.append(update)
        }
        #expect(updates == [.event(.snapshot(rows: 24, cols: 80, data: Array("$ ".utf8))), .event(.exit(code: 0))])
    }
}
