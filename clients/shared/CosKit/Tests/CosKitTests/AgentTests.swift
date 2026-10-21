import Foundation
import os
import Testing
@testable import CosKit

/// Payloads in the shape of the contract in docs/agents-api.md.
enum AgentSamples {
    static let agent = #"{"id":7,"workspace_id":3,"run_id":null,"role":"user","title":"fix login bug","session_id":"5504b543-4897-4751-add5-f81f24a6316c","pid":25615,"state":"working","model":"sonnet","permission_mode":"default","rows":40,"cols":120,"started_ms":1792611174811,"last_activity_ms":1792611180000,"exited_ms":null,"exit_code":null,"last_message":"Done. I created hello.txt …","pending_approval_id":null}"#
    static let waiting = #"{"id":8,"workspace_id":3,"run_id":null,"role":"user","title":"add left-pad","session_id":"e9ad2a0b-ef73-410c-9f41-d208b12d4a5b","pid":25700,"state":"waiting_approval","model":null,"permission_mode":"default","rows":30,"cols":100,"started_ms":1792611100000,"last_activity_ms":1792611190000,"exited_ms":null,"exit_code":null,"last_message":null,"pending_approval_id":12}"#
    static let exited = #"{"id":5,"workspace_id":4,"run_id":null,"role":"worker","title":"Agent 5","session_id":"0b6f","pid":null,"state":"exited","model":"haiku","permission_mode":"default","rows":40,"cols":120,"started_ms":1792611000000,"last_activity_ms":1792611050000,"exited_ms":1792611060000,"exit_code":0,"last_message":"All tests pass.","pending_approval_id":null}"#
    static let page = #"{"agents":[\#(agent),\#(waiting),\#(exited)]}"#
    static let approval = #"{"id":12,"agent_id":8,"workspace_id":3,"tool_name":"Bash","tool_input":{"command":"npm install left-pad"},"tool_use_id":"toolu_01ABC","summary":"Bash npm install left-pad","state":"pending","decided_by":null,"message":null,"created_ms":1792611174811,"decided_ms":null}"#
    static let approvals = #"{"approvals":[\#(approval)]}"#

    static func snapshot(_ text: String, rows: Int = 40, cols: Int = 120) -> String {
        "event: snapshot\ndata: {\"rows\":\(rows),\"cols\":\(cols),\"data\":\"\(Data(text.utf8).base64EncodedString())\"}\n\n"
    }

    static func output(_ text: String) -> String {
        "event: output\ndata: {\"data\":\"\(Data(text.utf8).base64EncodedString())\"}\n\n"
    }
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.cos().decode(type, from: Data(json.utf8))
}

@Suite struct AgentModelTests {
    @Test func decodesAgentsPage() throws {
        let agents = try decode(AgentsPage.self, AgentSamples.page).agents
        #expect(agents.map(\.id) == [7, 8, 5])

        let agent = agents[0]
        #expect(agent.workspaceId == 3)
        #expect(agent.runId == nil)
        #expect(agent.role == .user)
        #expect(agent.title == "fix login bug")
        #expect(agent.sessionId == "5504b543-4897-4751-add5-f81f24a6316c")
        #expect(agent.pid == 25615)
        #expect(agent.state == .working)
        #expect(agent.state.isAlive && !agent.state.canResume)
        #expect(agent.model == "sonnet")
        #expect(agent.permissionMode == "default")
        #expect(agent.rows == 40 && agent.cols == 120)
        #expect(agent.startedAt == Date(timeIntervalSince1970: 1_792_611_174.811))
        #expect(agent.lastMessage == "Done. I created hello.txt …")
        #expect(agent.exitCode == nil && agent.exitedMs == nil)

        #expect(agents[1].state == .waitingApproval)
        #expect(agents[1].state.needsYou)
        #expect(agents[1].pendingApprovalId == 12)
        #expect(agents[1].model == nil)

        let exited = agents[2]
        #expect(exited.state == .exited && exited.state.canResume && !exited.state.isAlive)
        #expect(exited.role == .worker)
        #expect(exited.exitCode == 0)
        #expect(exited.pid == nil)
    }

    @Test func toleratesNewStatesRolesAndRunIds() throws {
        let future = try decode(Agent.self, AgentSamples.agent
            .replacingOccurrences(of: #""state":"working""#, with: #""state":"compacting""#)
            .replacingOccurrences(of: #""role":"user""#, with: #""role":"planner""#)
            .replacingOccurrences(of: #""run_id":null"#, with: #""run_id":42"#))
        #expect(future.state.rawValue == "compacting")
        #expect(future.state.isAlive)
        #expect(future.role.rawValue == "planner")
        #expect(future.runId == "42")

        let named = try decode(Agent.self, AgentSamples.agent.replacingOccurrences(of: #""run_id":null"#, with: #""run_id":"r-1""#))
        #expect(named.runId == "r-1")
    }

    @Test func generationChangesWithANewProcess() throws {
        let before = try decode(Agent.self, AgentSamples.exited)
        var after = before
        after.pid = 30000
        after.state = .starting
        #expect(before.generation != after.generation)
    }

    @Test func decodesApprovals() throws {
        let approvals = try decode(ApprovalsPage.self, AgentSamples.approvals).approvals
        #expect(approvals.count == 1)
        let approval = approvals[0]
        #expect(approval.id == 12)
        #expect(approval.agentId == 8)
        #expect(approval.workspaceId == 3)
        #expect(approval.toolName == "Bash")
        #expect(approval.toolInput["command"] == .string("npm install left-pad"))
        #expect(approval.toolUseId == "toolu_01ABC")
        #expect(approval.summary == "Bash npm install left-pad")
        #expect(approval.state == .pending)
        #expect(approval.decidedBy == nil)
        #expect(approval.detail == "npm install left-pad")

        let edit = try decode(Approval.self, AgentSamples.approval
            .replacingOccurrences(of: #""tool_name":"Bash""#, with: #""tool_name":"Write""#)
            .replacingOccurrences(of: #"{"command":"npm install left-pad"}"#, with: #"{"file_path":"/x/hello.txt","content":"hi"}"#)
            .replacingOccurrences(of: #""state":"pending""#, with: #""state":"settled""#))
        #expect(edit.detail == "/x/hello.txt")
        #expect(edit.state == .settled)

        let other = try decode(Approval.self, AgentSamples.approval.replacingOccurrences(of: #"{"command":"npm install left-pad"}"#, with: #"{"todos":[]}"#))
        #expect(other.detail == "Bash npm install left-pad")
    }

    @Test func decodesScreen() throws {
        let screen = try decode(AgentScreen.self, #"{"rows":2,"cols":10,"text":"> hi\n","cursor":[0,4]}"#)
        #expect(screen.rows == 2 && screen.cols == 10 && screen.cursor == [0, 4])
    }

    @Test func eventsTriggerTheRightRefreshes() {
        func event(_ kind: String) -> LoggedEvent { LoggedEvent(seq: 1, ts: .now, kind: kind, data: .null) }
        #expect(event(EventKind.agentStarted).changesAgents)
        #expect(event(EventKind.agentStateChanged).changesAgents)
        #expect(event(EventKind.approvalRequested).changesAgents)
        #expect(event(EventKind.workspaceRemoved).changesAgents)
        #expect(event(EventKind.approvalDecided).changesApprovals)
        #expect(event(EventKind.agentExited).changesApprovals)
        #expect(!event(EventKind.agentStateChanged).changesApprovals)
    }

    @Test func newAgentLeavesOutEmptyFields() throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        func json(_ request: NewAgent) throws -> String { String(decoding: try encoder.encode(request), as: UTF8.self) }
        #expect(try json(NewAgent()) == "{}")
        #expect(try json(NewAgent(prompt: "  ", title: "", model: "")) == "{}")
        #expect(try json(NewAgent(prompt: "fix it\nplease\n", title: " Login ", model: "opus", permissionMode: "acceptEdits", rows: 40, cols: 120))
            == #"{"cols":120,"model":"opus","permission_mode":"acceptEdits","prompt":"fix it\nplease","rows":40,"title":"Login"}"#)
    }
}

@Suite struct TerminalEventTests {
    private func message(_ event: String, _ data: String) -> SSEMessage {
        SSEMessage(id: nil, event: event, data: data)
    }

    @Test func decodesEachEvent() {
        let bytes: [UInt8] = [0x1B, 0x5B, 0x48, 0xE2, 0x9C, 0xBB, 0x00, 0xFF]
        let base64 = Data(bytes).base64EncodedString()
        #expect(TerminalEvent(message("snapshot", #"{"rows":40,"cols":120,"data":"\#(base64)"}"#)) == .snapshot(rows: 40, cols: 120, data: bytes))
        #expect(TerminalEvent(message("output", #"{"data":"\#(base64)"}"#)) == .output(bytes))
        #expect(TerminalEvent(message("resize", #"{"rows":50,"cols":160}"#)) == .resize(rows: 50, cols: 160))
        #expect(TerminalEvent(message("exit", #"{"exit_code":0}"#)) == .exit(code: 0))
        #expect(TerminalEvent(message("exit", #"{"exit_code":null}"#)) == .exit(code: nil))
        #expect(TerminalEvent(message("exit", "{}")) == .exit(code: nil))
        #expect(TerminalEvent(message("output", #"{"data":""}"#)) == .output([]))
    }

    @Test func skipsWhatItDoesNotKnow() {
        #expect(TerminalEvent(message("title", #"{"title":"x"}"#)) == nil)
        #expect(TerminalEvent(SSEMessage(id: nil, event: nil, data: "{}")) == nil)
        #expect(TerminalEvent(message("output", "not json")) == nil)
        #expect(TerminalEvent(message("output", #"{"data":"%%%"}"#)) == nil)
        #expect(TerminalEvent(message("snapshot", #"{"rows":40,"data":""}"#)) == nil)
    }

    /// The stream arrives in arbitrary pieces, chunk sizes included; the
    /// HTTP, SSE and base64 layers must put the events back together.
    @Test func survivesArbitrarySplits() async throws {
        let body = AgentSamples.snapshot("\u{1B}[2J\u{1B}[1;1H✻ Welcome", rows: 24, cols: 80)
            + ": keep-alive\n\n"
            + AgentSamples.output("\u{1B}[38;2;215;119;87m> \u{1B}[0m")
            + "event: resize\ndata: {\"rows\":30,\"cols\":100}\n\n"
            + "event: exit\ndata: {\"exit_code\":1}\n\n"
        let chunk = String(body.utf8.count, radix: 16) + "\r\n" + body + "\r\n0\r\n\r\n"
        let raw = Array(("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n" + chunk).utf8)

        for pieceSize in [1, 3, 7, 64, raw.count] {
            let response = try await HTTPResponse(reader: ScriptedReader(bytes: raw, pieceSize: pieceSize))
            var parser = SSEParser()
            var events: [TerminalEvent] = []
            while let chunk = try await response.nextBodyChunk() {
                events += parser.feed(chunk).compactMap(TerminalEvent.init)
            }
            #expect(events == [
                .snapshot(rows: 24, cols: 80, data: Array("\u{1B}[2J\u{1B}[1;1H✻ Welcome".utf8)),
                .output(Array("\u{1B}[38;2;215;119;87m> \u{1B}[0m".utf8)),
                .resize(rows: 30, cols: 100),
                .exit(code: 1),
            ], "piece size \(pieceSize)")
        }
    }
}

@Suite(.serialized) struct AgentClientTests {
    @Test func sendsEveryRouteAsTheContractSays() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("GET", "/v1/agents"), ("GET", "/v1/agents?workspace_id=3"), ("GET", "/v1/agents?workspace_id=3&state=waiting_approval"):
                .json(200, AgentSamples.page)
            case ("GET", "/v1/agents/7"): .json(200, AgentSamples.agent)
            case ("POST", "/v1/workspaces/3/agents"): .json(201, AgentSamples.agent)
            case ("POST", "/v1/agents/7/input"), ("POST", "/v1/agents/7/keys"), ("POST", "/v1/agents/7/resize"), ("POST", "/v1/agents/7/trust"):
                .empty(204)
            case ("POST", "/v1/agents/7/stop"): .empty(202)
            case ("POST", "/v1/agents/5/resume"): .json(201, AgentSamples.exited.replacingOccurrences(of: #""state":"exited""#, with: #""state":"starting""#))
            case ("DELETE", "/v1/agents/5"): .empty(204)
            case ("GET", "/v1/agents/7/screen"): .json(200, #"{"rows":40,"cols":120,"text":"> ","cursor":[0,2]}"#)
            case ("GET", "/v1/approvals?state=pending"), ("GET", "/v1/approvals"): .json(200, AgentSamples.approvals)
            case ("POST", "/v1/approvals/12"):
                .json(200, AgentSamples.approval.replacingOccurrences(of: #""state":"pending""#, with: #""state":"denied""#))
            default: .json(404, #"{"error":"unexpected \#(request.method) \#(request.path)"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.agents().count == 3)
        _ = try await client.agents(workspaceId: 3)
        _ = try await client.agents(workspaceId: 3, state: .waitingApproval)
        #expect(try await client.agent(id: 7).title == "fix login bug")
        #expect(try await client.createAgent(workspaceId: 3, NewAgent(prompt: "fix the login bug\nand add a test", title: "Login", model: "opus")).id == 7)
        _ = try await client.createAgent(workspaceId: 3)
        try await client.sendInput(agentId: 7, text: "yes please")
        try await client.sendKeys(agentId: 7, [0x1B, 0x5B, 0x41, 0x03])
        try await client.resizeAgent(id: 7, rows: 50, cols: 160)
        try await client.trustAgent(id: 7)
        try await client.stopAgent(id: 7)
        #expect(try await client.resumeAgent(id: 5).state == .starting)
        try await client.removeAgent(id: 5)
        #expect(try await client.screen(agentId: 7).cursor == [0, 2])
        #expect(try await client.approvals().map(\.id) == [12])
        _ = try await client.approvals(state: nil)
        #expect(try await client.decide(approvalId: 12, .deny, message: "not now").state == .denied)
        _ = try await client.decide(approvalId: 12, .allow)

        let requests = server.requests
        #expect(requests.map { "\($0.method) \($0.path)" } == [
            "GET /v1/agents",
            "GET /v1/agents?workspace_id=3",
            "GET /v1/agents?workspace_id=3&state=waiting_approval",
            "GET /v1/agents/7",
            "POST /v1/workspaces/3/agents",
            "POST /v1/workspaces/3/agents",
            "POST /v1/agents/7/input",
            "POST /v1/agents/7/keys",
            "POST /v1/agents/7/resize",
            "POST /v1/agents/7/trust",
            "POST /v1/agents/7/stop",
            "POST /v1/agents/5/resume",
            "DELETE /v1/agents/5",
            "GET /v1/agents/7/screen",
            "GET /v1/approvals?state=pending",
            "GET /v1/approvals",
            "POST /v1/approvals/12",
            "POST /v1/approvals/12",
        ])
        #expect(requests[4].json == ["prompt": "fix the login bug\nand add a test", "title": "Login", "model": "opus"])
        #expect(requests[4].headers["content-type"] == "application/json")
        // Nothing given: the daemon picks every default.
        #expect(String(decoding: requests[5].body, as: UTF8.self) == "{}")
        #expect(requests[6].json == ["text": "yes please"])
        #expect(requests[7].json == ["data": "G1tBAw=="])
        #expect(requests[8].value == .object(["rows": .number(50), "cols": .number(160)]))
        #expect(requests[16].json == ["decision": "deny", "message": "not now"])
        #expect(requests[17].json == ["decision": "allow"])
        // Body-less routes send no body at all.
        for index in [0, 3, 9, 10, 11, 12, 13, 14, 15] {
            #expect(requests[index].body.isEmpty, "\(requests[index].path)")
            #expect(requests[index].headers["content-length"] == nil, "\(requests[index].path)")
        }
    }

    @Test func surfacesTheDaemonsMessages() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/agents/5/input"): .json(409, #"{"error":"agent 5 has exited"}"#)
            case ("POST", "/v1/agents/7/trust"): .json(409, #"{"error":"agent 7 isn't asking for trust"}"#)
            case ("DELETE", "/v1/agents/7"): .json(409, #"{"error":"agent 7 is still running; stop it first"}"#)
            case ("POST", "/v1/approvals/12"): .json(409, #"{"error":"approval 12 was already answered in the terminal"}"#)
            case ("GET", "/v1/agents/99"): .json(404, #"{"error":"no agent with id 99"}"#)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        await #expect(throws: CosError.conflict("agent 5 has exited")) { try await client.sendInput(agentId: 5, text: "hi") }
        await #expect(throws: CosError.conflict("agent 7 isn't asking for trust")) { try await client.trustAgent(id: 7) }
        await #expect(throws: CosError.conflict("agent 7 is still running; stop it first")) { try await client.removeAgent(id: 7) }
        await #expect(throws: CosError.conflict("approval 12 was already answered in the terminal")) { _ = try await client.decide(approvalId: 12, .allow) }
        await #expect(throws: CosError.notFound("no agent with id 99")) { _ = try await client.agent(id: 99) }
        // An older daemon without the agents API.
        await #expect(throws: CosError.notFound("The cos daemon answered 404 for /v1/agents.")) { _ = try await client.agents() }
    }
}

/// Collects a terminal stream's updates until it ends or `limit` is reached.
private func collect(_ stream: TerminalStream, limit: Int = 50) async -> [TerminalUpdate] {
    var updates: [TerminalUpdate] = []
    for await update in stream.updates() {
        updates.append(update)
        if updates.count >= limit { break }
    }
    return updates
}

private let instant = Backoff(initial: .zero, maximum: .zero)

@Suite(.serialized) struct TerminalStreamTests {
    @Test func streamsUntilExitThenEnds() async throws {
        let server = try FakeDaemon { request in
            guard request.path == "/v1/agents/7/terminal" else { return .empty(404) }
            return .stream(
                AgentSamples.snapshot("screen")
                    + AgentSamples.output("a") + AgentSamples.output("b")
                    + "event: resize\ndata: {\"rows\":50,\"cols\":160}\n\n"
                    + "event: exit\ndata: {\"exit_code\":0}\n\n"
                    // Anything after exit is ignored: the client stops reading.
                    + AgentSamples.output("late")
            )
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        let updates = await collect(TerminalStream(source: client, agentId: 7, backoff: instant))
        #expect(updates == [
            .event(.snapshot(rows: 40, cols: 120, data: Array("screen".utf8))),
            .event(.output(Array("a".utf8))),
            .event(.output(Array("b".utf8))),
            .event(.resize(rows: 50, cols: 160)),
            .event(.exit(code: 0)),
        ])
        #expect(server.requests.count == 1)
        #expect(server.requests[0].method == "GET")
        #expect(server.requests[0].headers["accept"] == "text/event-stream")
    }

    @Test func reconnectsWithAFreshSnapshotAfterADrop() async throws {
        let connections = OSAllocatedUnfairLock(initialState: 0)
        let server = try FakeDaemon { request in
            guard request.path == "/v1/agents/7/terminal" else { return .empty(404) }
            let attempt = connections.withLock { count in
                count += 1
                return count
            }
            switch attempt {
            case 1: return .streamThenDrop(AgentSamples.snapshot("first") + AgentSamples.output("x"))
            // The daemon is restarting: a cleanly closed stream without exit.
            case 2: return .streamThenEnd("")
            case 3: return .json(503, #"{"error":"starting up"}"#)
            default: return .streamThenEnd(AgentSamples.snapshot("second", rows: 30, cols: 100) + "event: exit\ndata: {\"exit_code\":null}\n\n")
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        let updates = await collect(TerminalStream(source: client, agentId: 7, backoff: instant))
        #expect(updates == [
            .event(.snapshot(rows: 40, cols: 120, data: Array("first".utf8))),
            .event(.output(Array("x".utf8))),
            .disconnected(.connectionClosed, retryIn: .zero),
            .disconnected(.connectionClosed, retryIn: .zero),
            .disconnected(.http(status: 503, message: "starting up"), retryIn: .zero),
            .event(.snapshot(rows: 30, cols: 100, data: Array("second".utf8))),
            .event(.exit(code: nil)),
        ])
        #expect(server.requests.count == 4)
    }

    @Test func backsOffWhileTheDaemonIsAway() async throws {
        // Nothing listens on this path.
        let client = CosClient(socketPath: NSTemporaryDirectory() + "cos-nobody-\(UUID().uuidString.prefix(6)).sock")
        let backoff = Backoff(initial: .milliseconds(1), maximum: .milliseconds(4))
        let updates = await collect(TerminalStream(source: client, agentId: 7, backoff: backoff), limit: 4)
        let delays: [Duration] = updates.compactMap {
            if case .disconnected(.notRunning, let delay) = $0 { delay } else { nil }
        }
        #expect(delays == [.milliseconds(1), .milliseconds(2), .milliseconds(4), .milliseconds(4)])
    }

    @Test func stopsWhenTheAgentIsGone() async throws {
        let server = try FakeDaemon { _ in .json(404, #"{"error":"no agent with id 7"}"#) }
        defer { server.stop() }
        let updates = await collect(TerminalStream(source: CosClient(socketPath: server.socketPath), agentId: 7, backoff: instant))
        #expect(updates == [.gone(.notFound("no agent with id 7"))])
    }
}

// MARK: TerminalSession

/// A terminal source that plays one script per connection.
final class ScriptedTerminal: TerminalSource {
    enum Step: Sendable {
        case events([TerminalEvent], then: CosError?)
        /// Yield these, then stay open until cancelled.
        case hold([TerminalEvent])
    }

    private let steps: OSAllocatedUnfairLock<[Step]>

    init(_ steps: [Step]) {
        self.steps = OSAllocatedUnfairLock(initialState: steps)
    }

    func terminal(agentId: Int) -> AsyncThrowingStream<TerminalEvent, Error> {
        let step = steps.withLock { $0.isEmpty ? Step.hold([]) : $0.removeFirst() }
        return AsyncThrowingStream { continuation in
            switch step {
            case .events(let events, let error):
                for event in events { continuation.yield(event) }
                continuation.finish(throwing: error)
            case .hold(let events):
                for event in events { continuation.yield(event) }
            }
        }
    }
}

/// Records keystrokes and resizes; each keystroke request takes `delay`.
actor RecordingControl: TerminalControl {
    private(set) var keys: [[UInt8]] = []
    private(set) var resizes: [TerminalSize] = []
    var failKeysWith: CosError?
    let delay: Duration

    init(delay: Duration = .zero) {
        self.delay = delay
    }

    func failKeys(with error: CosError?) {
        failKeysWith = error
    }

    func sendKeys(agentId: Int, _ bytes: [UInt8]) async throws {
        try await Task.sleep(for: delay)
        if let failKeysWith { throw failKeysWith }
        keys.append(bytes)
    }

    func resizeAgent(id: Int, rows: Int, cols: Int) async throws {
        resizes.append(TerminalSize(rows: rows, cols: cols))
    }
}

@MainActor
final class RecordingSink: TerminalSink {
    enum Call: Equatable {
        case reset(TerminalSize)
        case feed(String)
        case resize(TerminalSize)
    }

    var calls: [Call] = []

    func terminalReset(size: TerminalSize) { calls.append(.reset(size)) }
    func terminalFeed(_ bytes: [UInt8]) { calls.append(.feed(String(decoding: bytes, as: UTF8.self))) }
    func terminalResize(size: TerminalSize) { calls.append(.resize(size)) }
}

@MainActor
private func waitFor(_ what: String = "condition", _ condition: () async -> Bool) async throws {
    let deadline = ContinuousClock.now + .seconds(5)
    while !(await condition()) {
        if ContinuousClock.now > deadline { Issue.record("timed out waiting for \(what)"); return }
        try await Task.sleep(for: .milliseconds(5))
    }
}

@MainActor
@Suite(.serialized) struct TerminalSessionTests {
    private func bytes(_ text: String) -> [UInt8] { Array(text.utf8) }

    @Test func feedsSnapshotThenOutputAndFollowsResizes() async throws {
        let source = ScriptedTerminal([.hold([
            .snapshot(rows: 40, cols: 120, data: bytes("screen")),
            .output(bytes("a")),
            .resize(rows: 50, cols: 160),
            // Same size again (an echo): nothing to do.
            .resize(rows: 50, cols: 160),
            .output(bytes("b")),
        ])])
        let sink = RecordingSink()
        let session = TerminalSession(agentId: 7, source: source, control: RecordingControl(), backoff: instant)
        session.sink = sink
        session.start()
        defer { session.stop() }

        try await waitFor { sink.calls.count == 5 }
        #expect(sink.calls == [
            .reset(TerminalSize(rows: 40, cols: 120)),
            .feed("screen"),
            .feed("a"),
            .resize(TerminalSize(rows: 50, cols: 160)),
            .feed("b"),
        ])
        #expect(session.phase == .live)
        #expect(session.ptySize == TerminalSize(rows: 50, cols: 160))
    }

    @Test func reconnectsAndRedrawsFromTheNewSnapshot() async throws {
        let source = ScriptedTerminal([
            .events([.snapshot(rows: 40, cols: 120, data: bytes("one")), .output(bytes("x"))], then: .connectionClosed),
            .events([.snapshot(rows: 40, cols: 120, data: bytes("two")), .exit(code: 3)], then: nil),
        ])
        let sink = RecordingSink()
        let session = TerminalSession(agentId: 7, source: source, control: RecordingControl(), backoff: instant)
        session.sink = sink
        session.start()

        try await waitFor { session.phase == .exited(code: 3) }
        #expect(sink.calls == [
            .reset(TerminalSize(rows: 40, cols: 120)), .feed("one"), .feed("x"),
            .reset(TerminalSize(rows: 40, cols: 120)), .feed("two"),
        ])
        #expect(session.snapshots == 2)
        try await waitFor { !session.isRunning }
    }

    @Test func restartOpensAFreshStream() async throws {
        let source = ScriptedTerminal([
            .events([.snapshot(rows: 4, cols: 10, data: bytes("old")), .exit(code: 0)], then: nil),
            .hold([.snapshot(rows: 4, cols: 10, data: bytes("resumed"))]),
        ])
        let sink = RecordingSink()
        let session = TerminalSession(agentId: 7, source: source, control: RecordingControl(), backoff: instant)
        session.sink = sink
        session.start()
        try await waitFor { session.phase == .exited(code: 0) }

        session.restart()
        try await waitFor { session.phase == .live }
        #expect(sink.calls.last == .feed("resumed"))
        #expect(session.isRunning)
        session.stop()
    }

    @Test func endsWhenTheAgentIsGone() async throws {
        let source = ScriptedTerminal([.events([], then: .notFound("no agent with id 7"))])
        let session = TerminalSession(agentId: 7, source: source, control: RecordingControl(), backoff: instant)
        session.start()
        try await waitFor { session.phase == .gone("no agent with id 7") }
    }

    @Test func keystrokesKeepTheirOrderAndBatchWhileInFlight() async throws {
        let control = RecordingControl(delay: .milliseconds(50))
        let source = ScriptedTerminal([.hold([.snapshot(rows: 4, cols: 10, data: [])])])
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant)
        session.start()
        defer { session.stop() }
        try await waitFor { session.phase == .live }

        session.send(bytes("h"))
        // Let "h" leave (its request takes 50 ms).
        try await Task.sleep(for: .milliseconds(15))
        // Typed while "h" is on its way: these go together, in order.
        session.send(bytes("e"))
        session.send(bytes("ll"))
        session.send(bytes("o\r"))
        try await waitFor { await control.keys.count == 2 }
        #expect(await control.keys == [bytes("h"), bytes("ello\r")])

        session.send([0x1B])
        try await waitFor { await control.keys.count == 3 }
        #expect(await control.keys.last == [0x1B])
    }

    @Test func aBigPasteGoesInSeveralOrderedRequests() async throws {
        let control = RecordingControl()
        let source = ScriptedTerminal([.hold([.snapshot(rows: 4, cols: 10, data: [])])])
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant)
        session.start()
        defer { session.stop() }
        try await waitFor { session.phase == .live }

        let paste = (0..<(TerminalSession.maxKeyBatch * 2 + 10)).map { UInt8(truncatingIfNeeded: $0) }
        session.send(paste)
        try await waitFor { await control.keys.count == 3 }
        let sent = await control.keys
        #expect(sent.map(\.count) == [TerminalSession.maxKeyBatch, TerminalSession.maxKeyBatch, 10])
        #expect(sent.flatMap { $0 } == paste)
    }

    @Test func dropsQueuedKeysOnceTheAgentHasExited() async throws {
        let control = RecordingControl()
        await control.failKeys(with: .conflict("agent 7 has exited"))
        let source = ScriptedTerminal([.hold([.snapshot(rows: 4, cols: 10, data: [])])])
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant)
        session.start()
        defer { session.stop() }
        try await waitFor { session.phase == .live }

        session.send(bytes("x"))
        try await waitFor { session.lastError != nil }
        #expect(session.lastError == "agent 7 has exited")
    }

    @Test func ignoresKeysBeforeTheTerminalIsLiveAndAfterExit() async throws {
        let control = RecordingControl()
        let source = ScriptedTerminal([.events([.snapshot(rows: 4, cols: 10, data: []), .exit(code: 0)], then: nil)])
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant)
        session.send(bytes("early"))
        session.start()
        try await waitFor { session.phase == .exited(code: 0) }
        session.send(bytes("late"))
        try await Task.sleep(for: .milliseconds(30))
        #expect(await control.keys.isEmpty)
    }

    @Test func resizesTheAgentToFitTheViewDebounced() async throws {
        let control = RecordingControl()
        let source = ScriptedTerminal([.hold([.snapshot(rows: 40, cols: 120, data: bytes("s"))])])
        let sink = RecordingSink()
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant, resizeDelay: .milliseconds(40))
        session.sink = sink
        // The view knows its size before the snapshot arrives.
        session.viewSizeChanged(TerminalSize(rows: 30, cols: 90))
        session.start()
        defer { session.stop() }

        // After the snapshot it claims the PTY for its size.
        try await waitFor { await control.resizes.count == 1 }
        #expect(await control.resizes == [TerminalSize(rows: 30, cols: 90)])
        #expect(session.ptySize == TerminalSize(rows: 30, cols: 90))
        #expect(sink.calls.last == .resize(TerminalSize(rows: 30, cols: 90)))

        // A live window resize: many sizes quickly, one request at the end.
        for cols in 91...110 {
            session.viewSizeChanged(TerminalSize(rows: 31, cols: cols))
        }
        try await waitFor { await control.resizes.count == 2 }
        try await Task.sleep(for: .milliseconds(80))
        #expect(await control.resizes == [TerminalSize(rows: 30, cols: 90), TerminalSize(rows: 31, cols: 110)])

        // Back to the size the PTY already has: nothing to send.
        session.viewSizeChanged(TerminalSize(rows: 31, cols: 110))
        try await Task.sleep(for: .milliseconds(80))
        #expect(await control.resizes.count == 2)
    }

    @Test func reclaimsTheSizeWhenYouTypeAfterSomeoneElseResized() async throws {
        let control = RecordingControl()
        let source = ScriptedTerminal([.hold([
            .snapshot(rows: 30, cols: 90, data: []),
            // `cos agent attach` in an 80×24 terminal.
            .resize(rows: 24, cols: 80),
        ])])
        let sink = RecordingSink()
        let session = TerminalSession(agentId: 7, source: source, control: control, backoff: instant, resizeDelay: .milliseconds(10))
        session.sink = sink
        session.viewSizeChanged(TerminalSize(rows: 30, cols: 90))
        session.start()
        defer { session.stop() }

        try await waitFor { session.ptySize == TerminalSize(rows: 24, cols: 80) }
        #expect(sink.calls.last == .resize(TerminalSize(rows: 24, cols: 80)))
        try await Task.sleep(for: .milliseconds(40))
        #expect(await control.resizes.isEmpty)

        session.send(bytes("x"))
        try await waitFor { await control.resizes.count == 1 }
        #expect(await control.resizes == [TerminalSize(rows: 30, cols: 90)])
    }
}

// MARK: Monitor

@MainActor
@Suite(.serialized) struct MonitorAgentTests {
    @Test func refreshesOnConnectAndOnAgentEvents() async throws {
        let events = [
            #"{"seq":2,"ts":"2026-10-22T09:00:00Z","kind":"agent.started","data":{"agent_id":7,"workspace_id":3,"role":"user","session_id":"5504"}}"#,
            #"{"seq":3,"ts":"2026-10-22T09:00:01Z","kind":"approval.requested","data":{"approval_id":12,"agent_id":8,"summary":"Bash npm install left-pad"}}"#,
        ]
        let server = try FakeDaemon { request in
            switch request.path {
            case "/v1/status": .json(200, Samples.status.replacingOccurrences(of: #""last_seq":1"#, with: #""last_seq":4"#))
            case let path where path.hasPrefix("/v1/events/stream"):
                .stream(events.enumerated().map { "id: \($0.offset + 2)\ndata: \($0.element)\n\n" }.joined())
            case "/v1/projects": .json(200, ProjectSamples.page)
            case "/v1/agents": .json(200, AgentSamples.page)
            case "/v1/approvals?state=pending": .json(200, AgentSamples.approvals)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))
        monitor.start()
        defer { monitor.stop() }

        func fetches(_ path: String) -> Int { server.requests.filter { $0.path == path }.count }
        try await waitFor { monitor.events.count == 2 && monitor.projects.count == 2 && monitor.agents.count == 3 && monitor.approvals.count == 1 }
        try await Task.sleep(for: .milliseconds(200))
        // On connect, plus one for the events unless they arrived before the
        // connect fetch began (then that one fetch already covers them).
        #expect((1...3).contains(fetches("/v1/agents")))
        #expect((1...3).contains(fetches("/v1/approvals?state=pending")))
        // agent.* events don't touch projects.
        #expect(fetches("/v1/projects") == 1)

        // Once settled, each kind of event refreshes exactly what it changes.
        func settle(_ kind: String, agents: Int, approvals: Int) async throws {
            let before = (fetches("/v1/agents"), fetches("/v1/approvals?state=pending"))
            monitor.apply(.event(LoggedEvent(seq: monitor.events.last!.seq + 1, ts: .now, kind: kind, data: .object([:]))))
            try await waitFor {
                fetches("/v1/agents") == before.0 + agents
                    && fetches("/v1/approvals?state=pending") == before.1 + approvals
            }
            try await Task.sleep(for: .milliseconds(50))
            #expect(fetches("/v1/agents") == before.0 + agents, "\(kind)")
            #expect(fetches("/v1/approvals?state=pending") == before.1 + approvals, "\(kind)")
        }
        try await settle(EventKind.agentStateChanged, agents: 1, approvals: 0)
        try await settle(EventKind.approvalDecided, agents: 1, approvals: 1)
        try await settle(EventKind.agentExited, agents: 1, approvals: 1)
        try await settle(EventKind.daemonStarted, agents: 0, approvals: 0)

        #expect(monitor.agentsSupported)
        #expect(monitor.agents(inWorkspace: 3).map(\.id) == [7, 8])
        #expect(monitor.pendingApproval(forAgent: 8)?.id == 12)
        #expect(monitor.pendingApproval(forAgent: 7) == nil)
        #expect(monitor.waitingCount == 1)
        #expect(monitor.agentsWaitingOnYou.map(\.id) == [8])
    }

    @Test func anOlderDaemonWithoutAgentsIsNotAnError() async throws {
        let server = try FakeDaemon { request in
            switch request.path {
            case "/v1/status": .json(200, Samples.status)
            case let path where path.hasPrefix("/v1/events/stream"): .stream("")
            case "/v1/projects": .json(200, ProjectSamples.page)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))
        monitor.start()
        defer { monitor.stop() }

        try await waitFor { monitor.projects.count == 2 && !monitor.agentsSupported }
        #expect(monitor.agentsError == nil)
        #expect(monitor.agents.isEmpty && monitor.approvals.isEmpty)
    }

    @Test func actionsUpdateTheListsAtOnce() async throws {
        let resumed = AgentSamples.exited.replacingOccurrences(of: #""state":"exited""#, with: #""state":"starting""#).replacingOccurrences(of: #""pid":null"#, with: #""pid":4242"#)
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/workspaces/3/agents"): .json(201, AgentSamples.agent)
            case ("POST", "/v1/agents/5/resume"): .json(201, resumed)
            case ("DELETE", "/v1/agents/7"): .empty(204)
            case ("POST", "/v1/agents/7/stop"): .empty(202)
            case ("POST", "/v1/agents/7/trust"): .empty(204)
            case ("POST", "/v1/approvals/12"): .json(409, #"{"error":"approval 12 was already answered in the terminal"}"#)
            case ("POST", "/v1/approvals/13"): .json(200, AgentSamples.approval.replacingOccurrences(of: #""id":12"#, with: #""id":13"#).replacingOccurrences(of: #""state":"pending""#, with: #""state":"allowed""#))
            // The lists fail, so whatever changes comes from the actions.
            default: .json(503, #"{"error":"busy"}"#)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))

        let agent = try await monitor.createAgent(workspaceId: 3, NewAgent(prompt: "fix the login bug"))
        #expect(monitor.agents.map(\.id) == [7])
        #expect(agent.title == "fix login bug")
        try await monitor.stopAgent(id: 7)
        try await monitor.trustAgent(id: 7)

        let back = try await monitor.resumeAgent(id: 5)
        #expect(back.state == .starting)
        #expect(monitor.agents.map(\.id) == [5, 7])
        #expect(monitor.agent(id: 5)?.pid == 4242)

        try await monitor.removeAgent(id: 7)
        #expect(monitor.agents.map(\.id) == [5])

        // Answered in the terminal first: it leaves the list, and the app hears why.
        let approval = try JSONDecoder.cos().decode(Approval.self, from: Data(AgentSamples.approval.utf8))
        await #expect(throws: CosError.conflict("approval 12 was already answered in the terminal")) {
            try await monitor.decide(approval, .allow)
        }
        var other = approval
        other.id = 13
        #expect(try await monitor.decide(other, .allow).state == .allowed)

        try await waitFor { monitor.agentsError != nil }
        #expect(monitor.agentsError == "busy")
    }

    private func waitFor(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            if ContinuousClock.now > deadline { Issue.record("timed out"); return }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}
