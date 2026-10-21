import Foundation
import Testing
@testable import CosKit

/// Payloads in the shape of docs/daemon.md ("Runs, tasks and the board")
/// and docs/orchestration.md.
enum RunSamples {
    static let counts = #"{"proposed":0,"ready":1,"running":1,"checking":0,"review":0,"merged":2,"blocked":0,"failed":0,"cancelled":0,"total":4}"#
    static let run = #"{"id":3,"workspace_id":3,"goal":"add rate limiting","state":"running","preset":"normal","approval_cap":20,"chief_agent_id":9,"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:05:00.123Z"}"#
    static let summary = #"{"id":3,"workspace_id":3,"goal":"add rate limiting","state":"running","preset":"normal","approval_cap":20,"chief_agent_id":9,"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:05:00Z","tasks":\#(counts)}"#
    static let done = #"{"id":2,"workspace_id":4,"goal":"write release notes","state":"done","preset":"strict","approval_cap":5,"report_path":"runs/2/report.md","created_at":"2026-10-21T09:00:00Z","updated_at":"2026-10-21T10:00:00Z","tasks":{"merged":1,"total":1}}"#
    static let page = #"{"runs":[\#(summary),\#(done)]}"#

    static let merged = #"{"id":6,"run_id":3,"position":0,"title":"Add limiter","instructions":"Write a token bucket.","files":["src/limit.rs"],"test_command":"cargo test limit","depends_on":[],"state":"merged","retries":0,"agent_id":11,"checks":{"scope":{"ok":true,"out_of_scope":[],"protected":[],"uncommitted":[]},"tests":{"status":"passed","exit_code":0,"output_tail":"test result: ok. 4 passed"}},"review":{"verdict":"approve","notes":null},"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:04:00Z"}"#
    static let blocked = #"{"id":7,"run_id":3,"position":1,"title":"Wire it in","instructions":"Call the limiter from the router.","files":["src/router.rs","src/limit/**"],"depends_on":[6],"state":"blocked","retries":1,"branch":"cos/run-3/wire-it-in","worktree":"/Users/me/.cos/tasks/3-7","last_failure":"tests failed twice","agent_id":12,"checks":{"scope":{"ok":false,"out_of_scope":["Cargo.lock"],"protected":[".env"],"uncommitted":[]},"tests":{"status":"failed","exit_code":101,"output_tail":"thread 'main' panicked"}},"review":{"verdict":"changes","notes":"Handle the burst case."},"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:05:00Z"}"#
    static let board = #"{"run":\#(run),"workspace":\#(ProjectSamples.lisbon),"tasks":[\#(blocked),\#(merged)],"counts":\#(counts)}"#

    static let question = #"{"id":4,"run_id":3,"agent_id":9,"question":"Use Redis or in-memory?","choices":["redis","memory"],"state":"open","answer":null,"created_ms":1792611174811,"answered_ms":null}"#
    static let questions = #"{"questions":[\#(question)]}"#
    static let limits = #"{"preset":"normal","approval_cap":20,"chief_approvals":3}"#

    static func approval(id: Int, agent: Int, state: String = "pending", route: String?, tier: String? = "chief", reason: String? = "runs a build in the worktree", decidedBy: String? = nil) -> String {
        func json(_ value: String?) -> String { value.map { "\"\($0)\"" } ?? "null" }
        return #"{"id":\#(id),"agent_id":\#(agent),"workspace_id":3,"tool_name":"Bash","tool_input":{"command":"cargo build"},"summary":"Bash cargo build","state":"\#(state)","decided_by":\#(json(decidedBy)),"message":null,"created_ms":\#(1792611174811 + Int64(id)),"decided_ms":null,"tier":\#(json(tier)),"route":\#(json(route)),"reason":\#(json(reason))}"#
    }

    static func agent(id: Int, run: Int?, role: String, state: String = "working") -> String {
        #"{"id":\#(id),"workspace_id":3,"run_id":\#(run.map(String.init) ?? "null"),"role":"\#(role)","title":"\#(role) \#(id)","session_id":"s\#(id)","pid":\#(100 + id),"state":"\#(state)","model":null,"permission_mode":"default","rows":40,"cols":120,"started_ms":1792611174811,"last_activity_ms":null,"exited_ms":null,"exit_code":null,"last_message":null,"pending_approval_id":null}"#
    }
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder.cos().decode(type, from: Data(json.utf8))
}

@Suite struct RunModelTests {
    @Test func decodesTheRunList() throws {
        let runs = try decode(RunsPage.self, RunSamples.page).runs
        #expect(runs.map(\.id) == [3, 2])
        let run = runs[0]
        #expect(run.workspaceId == 3)
        #expect(run.goal == "add rate limiting")
        #expect(run.state == .running && !run.state.isFinished)
        #expect(run.preset == .normal)
        #expect(run.approvalCap == 20)
        #expect(run.chiefAgentId == 9)
        #expect(run.createdAt == Date(timeIntervalSince1970: 1_792_659_600))
        #expect(run.counts?.total == 4)
        #expect(run.counts?.finished == 2)
        #expect(run.counts?.active == 1)
        #expect(run.counts?.progress == 0.5)

        let done = runs[1]
        #expect(done.state == .done && done.state.isFinished)
        #expect(done.preset == .strict)
        #expect(done.reportPath == "runs/2/report.md")
        #expect(done.chiefAgentId == nil)
        // Counts the daemon left out are zero.
        #expect(done.counts?.merged == 1 && done.counts?.ready == 0 && done.counts?.total == 1)
        #expect(done.reportURL(home: "/Users/me/.cos")?.path == "/Users/me/.cos/runs/2/report.md")
        var absolute = done
        absolute.reportPath = "/tmp/r.md"
        #expect(absolute.reportURL(home: nil)?.path == "/tmp/r.md")
        #expect(done.reportURL(home: nil) == nil)
    }

    @Test func decodesTheBoardWithTheContractsFields() throws {
        let board = try decode(Board.self, RunSamples.board)
        #expect(board.run.id == 3)
        #expect(board.run.counts == nil)
        #expect(board.workspace?.name == "lisbon")
        // In plan order, whatever order they came in.
        #expect(board.tasks.map(\.id) == [6, 7])
        #expect(board.counts.total == 4)

        let merged = board.tasks[0]
        #expect(merged.title == "Add limiter")
        #expect(merged.instructions == "Write a token bucket.")
        #expect(merged.files == ["src/limit.rs"])
        #expect(merged.testCommand == "cargo test limit")
        #expect(merged.state == .merged && merged.state.isFinished)
        #expect(merged.agentId == 11)
        #expect(merged.checks?.scope?.ok == true)
        #expect(merged.checks?.tests?.passed == true)
        #expect(merged.checks?.tests?.outputTail == "test result: ok. 4 passed")
        #expect(merged.review?.approved == true)
        #expect(merged.review?.notes == nil)
        #expect(!merged.canCancel && !merged.canRetry)

        let blocked = board.tasks[1]
        #expect(blocked.dependsOn == [6])
        #expect(blocked.testCommand == nil)
        #expect(blocked.retries == 1)
        #expect(blocked.branch == "cos/run-3/wire-it-in")
        #expect(blocked.worktree == "/Users/me/.cos/tasks/3-7")
        #expect(blocked.lastFailure == "tests failed twice")
        #expect(blocked.state.needsDecision)
        // Its one retry is used up; fail or cancel it.
        #expect(!blocked.canRetry && blocked.canFail && blocked.canCancel)
        #expect(blocked.checks?.scope?.ok == false)
        #expect(blocked.checks?.scope?.problems == ["outside its files: Cargo.lock", "protected paths: .env"])
        #expect(blocked.checks?.tests?.exitCode == 101)
        #expect(blocked.checks?.tests?.passed == false)
        #expect(blocked.review?.verdict == "changes")
        #expect(blocked.review?.notes == "Handle the burst case.")
        #expect(board.task(id: 7)?.title == "Wire it in")
    }

    @Test func toleratesOlderAndNewerDaemons() throws {
        // Today's daemon: no agent_id, checks, review or chief_agent_id.
        let old = try decode(Board.self, #"{"run":{"id":1,"workspace_id":2,"goal":"g","state":"awaiting_plan_approval","preset":"normal","approval_cap":20,"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:00:00Z"},"workspace":null,"tasks":[{"id":1,"run_id":1,"position":0,"title":"A","instructions":"do a","files":["a"],"depends_on":[],"state":"proposed","retries":0,"created_at":"2026-10-22T09:00:00Z","updated_at":"2026-10-22T09:00:00Z"}],"counts":{"proposed":1,"total":1}}"#)
        #expect(old.run.state == .awaitingPlanApproval)
        #expect(old.workspace == nil)
        #expect(old.tasks[0].agentId == nil && old.tasks[0].checks == nil && old.tasks[0].review == nil)
        #expect(old.counts.proposed == 1)

        // A newer one: unknown states, presets and verdicts, extra fields,
        // a plan's `description`, and checks in a shape not seen before.
        let new = try decode(Board.self, #"{"run":{"id":1,"workspace_id":2,"goal":"g","state":"pausing","preset":"paranoid","approval_cap":"lots","created_at":"2026-10-22T09:00:00Z","extra":{"x":1}},"tasks":[{"id":1,"title":"A","description":"from the plan","state":"rebasing","checks":{"scope":{"out_of_scope":["x"],"uncommitted":true},"tests":{"status":"skipped"},"lint":{}},"review":{"verdict":"maybe"},"agent_id":"not a number"}]}"#)
        #expect(new.run.state.rawValue == "pausing" && !new.run.state.isFinished)
        #expect(new.run.preset.rawValue == "paranoid")
        #expect(new.run.approvalCap == 20)
        #expect(new.run.updatedAt == new.run.createdAt)
        #expect(new.tasks[0].instructions == "from the plan")
        #expect(new.tasks[0].state.rawValue == "rebasing" && !new.tasks[0].state.isFinished)
        #expect(new.tasks[0].checks?.scope?.ok == false)
        #expect(new.tasks[0].checks?.scope?.uncommitted == ["(yes)"])
        #expect(new.tasks[0].checks?.tests?.status == "skipped")
        #expect(new.tasks[0].review?.approved == false)
        #expect(new.tasks[0].agentId == nil)
        // Counts the daemon didn't send are worked out from the tasks.
        #expect(new.counts.total == 1)
    }

    @Test func decodesQuestionsAndLimits() throws {
        let questions = try decode(QuestionsPage.self, RunSamples.questions).questions
        #expect(questions.count == 1)
        let question = questions[0]
        #expect(question.id == 4 && question.runId == 3 && question.agentId == 9)
        #expect(question.question == "Use Redis or in-memory?")
        #expect(question.choices == ["redis", "memory"])
        #expect(question.state == .open)
        #expect(question.answer == nil)
        #expect(question.createdAt == Date(timeIntervalSince1970: 1_792_611_174.811))

        let bare = try decode(Question.self, #"{"id":5,"question":"Which one?"}"#)
        #expect(bare.choices.isEmpty && bare.state == .open && bare.runId == nil)

        let limits = try decode(RunLimits.self, RunSamples.limits)
        #expect(limits == RunLimits(preset: .normal, approvalCap: 20, chiefApprovals: 3))
        #expect(!limits.capReached)
        #expect(RunLimits(preset: .trusted, approvalCap: 3, chiefApprovals: 3).capReached)
    }

    @Test func decodesApprovalRoutes() throws {
        let routed = try decode(Approval.self, RunSamples.approval(id: 1, agent: 11, route: "chief"))
        #expect(routed.tier == .chief)
        #expect(routed.route == .chief)
        #expect(routed.reason == "runs a build in the worktree")
        #expect(!routed.isForYou && routed.isWithChief)

        let yours = try decode(Approval.self, RunSamples.approval(id: 2, agent: 11, route: "user", tier: "never_delegated", reason: "git push is never delegated"))
        #expect(yours.tier == .neverDelegated && yours.isForYou)

        let auto = try decode(Approval.self, RunSamples.approval(id: 3, agent: 11, state: "allowed", route: "auto", tier: "allowed", decidedBy: "rule"))
        #expect(auto.route == .auto && auto.decidedBy == "rule" && !auto.isForYou)

        // A daemon without routes: every pending approval is yours.
        let plain = try decode(Approval.self, AgentSamples.approval)
        #expect(plain.tier == nil && plain.route == nil && plain.reason == nil)
        #expect(plain.isForYou)
    }

    @Test func newRunLeavesOutWhatIsntSet() throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        func json(_ request: NewRun) throws -> String { String(decoding: try encoder.encode(request), as: UTF8.self) }
        #expect(try json(NewRun(workspaceId: 3, goal: "  add rate limiting\n")) == #"{"goal":"add rate limiting","workspace_id":3}"#)
        #expect(try json(NewRun(workspaceId: 3, goal: "g", preset: .trusted, approvalCap: 5)) == #"{"approval_cap":5,"goal":"g","preset":"trusted","workspace_id":3}"#)
    }

    @Test func eventsTriggerTheRightRunRefreshes() {
        func event(_ kind: String, runId: String? = nil, data: JSONValue = .null) -> LoggedEvent {
            LoggedEvent(seq: 1, ts: .now, kind: kind, runId: runId, data: data)
        }
        for kind in [EventKind.runCreated, EventKind.runStateChanged, EventKind.planProposed, EventKind.planApproved,
                     EventKind.taskStateChanged, EventKind.taskChecked, EventKind.taskReviewed, EventKind.taskMerged,
                     EventKind.runReportWritten, EventKind.runLimitsChanged, "task.something_new", EventKind.workspaceRemoved] {
            #expect(event(kind).changesRuns, "\(kind)")
        }
        #expect(!event(EventKind.agentStarted).changesRuns)
        #expect(!event(EventKind.approvalRouted).changesRuns)
        #expect(event(EventKind.questionAsked).changesQuestions)
        #expect(event(EventKind.questionAnswered).changesQuestions)
        #expect(event(EventKind.runStateChanged).changesQuestions)
        #expect(!event(EventKind.taskStateChanged).changesQuestions)
        #expect(event(EventKind.approvalRouted).changesApprovals)

        #expect(event(EventKind.taskStateChanged, runId: "3").runNumber == 3)
        #expect(event(EventKind.questionAsked, data: .object(["run_id": .number(4)])).runNumber == 4)
        #expect(event(EventKind.questionAsked, data: .object(["run_id": .string("5")])).runNumber == 5)
        #expect(event(EventKind.approvalRouted).runNumber == nil)
    }
}

@Suite(.serialized) struct RunClientTests {
    @Test func sendsEveryRouteAsTheContractSays() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("GET", "/v1/runs"): .json(200, RunSamples.page)
            case ("POST", "/v1/runs"): .json(201, RunSamples.run)
            case ("GET", "/v1/runs/3"), ("POST", "/v1/runs/3/approve-plan"), ("POST", "/v1/runs/3/cancel"), ("PATCH", "/v1/tasks/7"):
                .json(200, RunSamples.board)
            case ("GET", "/v1/questions?state=open"), ("GET", "/v1/questions"): .json(200, RunSamples.questions)
            case ("POST", "/v1/questions/4/answer"):
                .json(200, RunSamples.question.replacingOccurrences(of: #""state":"open","answer":null"#, with: #""state":"answered","answer":"redis""#))
            case ("POST", "/v1/runs/3/message"): .empty(202)
            case ("GET", "/v1/runs/3/report"): .json(200, ##"{"markdown":"# Report\n\nAll done."}"##)
            case ("GET", "/v1/runs/3/limits"): .json(200, RunSamples.limits)
            case ("PUT", "/v1/runs/3/limits"): .json(200, #"{"preset":"trusted","approval_cap":30,"chief_approvals":3}"#)
            default: .json(404, #"{"error":"unexpected \#(request.method) \#(request.path)"}"#)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        #expect(try await client.runs().map(\.id) == [3, 2])
        #expect(try await client.createRun(NewRun(workspaceId: 3, goal: "add rate limiting\nand tests", preset: .strict, approvalCap: 10)).id == 3)
        _ = try await client.createRun(NewRun(workspaceId: 3, goal: "g"))
        #expect(try await client.board(runId: 3).tasks.count == 2)
        _ = try await client.approvePlan(runId: 3)
        _ = try await client.cancelRun(id: 3)
        _ = try await client.cancelRun(id: 3, reason: "wrong goal")
        _ = try await client.updateTask(id: 7, state: .ready, reason: "try once more")
        _ = try await client.updateTask(id: 7, state: .cancelled)
        #expect(try await client.questions().map(\.id) == [4])
        _ = try await client.questions(state: nil)
        #expect(try await client.answer(questionId: 4, "redis").answer == "redis")
        try await client.messageChief(runId: 3, text: "skip the docs")
        #expect(try await client.report(runId: 3) == "# Report\n\nAll done.")
        #expect(try await client.limits(runId: 3).chiefApprovals == 3)
        #expect(try await client.setLimits(runId: 3, preset: .trusted, approvalCap: 30).preset == .trusted)

        let requests = server.requests
        #expect(requests.map { "\($0.method) \($0.path)" } == [
            "GET /v1/runs",
            "POST /v1/runs",
            "POST /v1/runs",
            "GET /v1/runs/3",
            "POST /v1/runs/3/approve-plan",
            "POST /v1/runs/3/cancel",
            "POST /v1/runs/3/cancel",
            "PATCH /v1/tasks/7",
            "PATCH /v1/tasks/7",
            "GET /v1/questions?state=open",
            "GET /v1/questions",
            "POST /v1/questions/4/answer",
            "POST /v1/runs/3/message",
            "GET /v1/runs/3/report",
            "GET /v1/runs/3/limits",
            "PUT /v1/runs/3/limits",
        ])
        #expect(String(decoding: requests[1].body, as: UTF8.self) == #"{"approval_cap":10,"goal":"add rate limiting\nand tests","preset":"strict","workspace_id":3}"#)
        #expect(requests[1].headers["content-type"] == "application/json")
        // No preset or cap: the daemon's defaults.
        #expect(String(decoding: requests[2].body, as: UTF8.self) == #"{"goal":"g","workspace_id":3}"#)
        #expect(requests[6].json == ["reason": "wrong goal"])
        #expect(requests[7].json == ["state": "ready", "reason": "try once more"])
        #expect(requests[8].json == ["state": "cancelled"])
        #expect(requests[11].json == ["answer": "redis"])
        #expect(requests[12].json == ["text": "skip the docs"])
        #expect(requests[15].value == .object(["preset": .string("trusted"), "approval_cap": .number(30)]))
        // Body-less routes send no body at all.
        for index in [0, 3, 4, 5, 9, 10, 13, 14] {
            #expect(requests[index].body.isEmpty, "\(requests[index].path)")
            #expect(requests[index].headers["content-length"] == nil, "\(requests[index].path)")
        }
    }

    @Test func surfacesTheDaemonsMessages() async throws {
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/runs"): .json(409, #"{"error":"workspace 3 already has an unfinished run (run 3)"}"#)
            case ("POST", "/v1/runs/3/approve-plan"): .json(409, #"{"error":"run 3 has no plan waiting for approval (it is running)"}"#)
            case ("PATCH", "/v1/tasks/7"): .json(400, #"{"error":"failing a task needs a reason"}"#)
            case ("POST", "/v1/questions/4/answer"): .json(409, #"{"error":"question 4 is already answered"}"#)
            case ("GET", "/v1/runs/3/report"): .json(404, #"{"error":"run 3 has no report yet"}"#)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let client = CosClient(socketPath: server.socketPath)

        await #expect(throws: CosError.conflict("workspace 3 already has an unfinished run (run 3)")) { _ = try await client.createRun(NewRun(workspaceId: 3, goal: "g")) }
        await #expect(throws: CosError.conflict("run 3 has no plan waiting for approval (it is running)")) { _ = try await client.approvePlan(runId: 3) }
        await #expect(throws: CosError.http(status: 400, message: "failing a task needs a reason")) { _ = try await client.updateTask(id: 7, state: .failed) }
        await #expect(throws: CosError.conflict("question 4 is already answered")) { _ = try await client.answer(questionId: 4, "x") }
        await #expect(throws: CosError.notFound("run 3 has no report yet")) { _ = try await client.report(runId: 3) }
        // A daemon without the chief's routes.
        await #expect(throws: CosError.notFound("The cos daemon answered 404 for /v1/questions?state=open.")) { _ = try await client.questions() }
        await #expect(throws: CosError.notFound("The cos daemon answered 404 for /v1/runs/3/message.")) { try await client.messageChief(runId: 3, text: "hi") }
    }
}

@MainActor
@Suite(.serialized) struct MonitorRunTests {
    /// A fake daemon with one run in workspace 3, its chief (9) and worker (12),
    /// an approval routed to the chief, one to you and one a rule allowed.
    private func makeServer(
        runs: String = RunSamples.page,
        board: @escaping @Sendable () -> String = { RunSamples.board },
        extra: @escaping @Sendable (FakeDaemon.Request) -> FakeDaemon.Reply? = { _ in nil }
    ) throws -> FakeDaemon {
        let agents = #"{"agents":[\#(RunSamples.agent(id: 12, run: 3, role: "worker")),\#(RunSamples.agent(id: 9, run: 3, role: "chief", state: "idle")),\#(RunSamples.agent(id: 7, run: nil, role: "user"))]}"#
        let pending = #"{"approvals":[\#(RunSamples.approval(id: 20, agent: 12, route: "chief")),\#(RunSamples.approval(id: 21, agent: 12, route: "user", tier: "ask_user", reason: "an unknown program")),\#(RunSamples.approval(id: 22, agent: 7, route: nil, tier: nil, reason: nil))]}"#
        let all = #"{"approvals":[\#(RunSamples.approval(id: 20, agent: 12, route: "chief")),\#(RunSamples.approval(id: 21, agent: 12, route: "user", tier: "ask_user", reason: "an unknown program")),\#(RunSamples.approval(id: 19, agent: 12, state: "allowed", route: "auto", tier: "allowed", reason: "reads inside the worktree", decidedBy: "rule")),\#(RunSamples.approval(id: 22, agent: 7, route: nil, tier: nil, reason: nil))]}"#
        return try FakeDaemon { request in
            if let reply = extra(request) { return reply }
            switch request.path {
            case "/v1/status": return .json(200, Samples.status)
            case let path where path.hasPrefix("/v1/events/stream"): return .stream(": open\n\n")
            case "/v1/projects": return .json(200, ProjectSamples.page)
            case "/v1/agents": return .json(200, agents)
            case "/v1/approvals?state=pending": return .json(200, pending)
            case "/v1/approvals": return .json(200, all)
            case "/v1/runs": return .json(200, runs)
            case "/v1/runs/3": return .json(200, board())
            case "/v1/runs/3/limits": return .json(200, RunSamples.limits)
            case "/v1/questions?state=open": return .json(200, RunSamples.questions)
            default: return .json(404, #"{"error":"no \#(request.path)"}"#)
            }
        }
    }

    @Test func loadsRunsAndQuestionsAndCountsWhatWaitsOnYou() async throws {
        let server = try makeServer()
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath), runsDebounce: .milliseconds(20))
        monitor.start()
        defer { monitor.stop() }

        try await waitFor { monitor.runs.count == 2 && monitor.questions.count == 1 && monitor.approvals.count == 3 && monitor.agents.count == 3 }
        #expect(monitor.runsSupported && monitor.questionsSupported)
        #expect(monitor.currentRun(inWorkspace: 3)?.id == 3)
        #expect(monitor.currentRun(inWorkspace: 4) == nil)
        #expect(monitor.runs(inWorkspace: 4).map(\.id) == [2])
        #expect(monitor.activeRuns.map(\.id) == [3])
        #expect(monitor.plansAwaitingApproval.isEmpty)
        #expect(monitor.questions(forRun: 3).map(\.id) == [4])
        #expect(monitor.agents(forRun: 3).map(\.id) == [12, 9])
        // The one routed to the chief isn't yours; the one without a route is.
        #expect(monitor.approvalsForYou.map(\.id) == [21, 22])
        // The worker's request is with the chief: it isn't waiting on you.
        #expect(monitor.agentsWaitingOnYou.map(\.id) == [7])
        // Two approvals and a question.
        #expect(monitor.waitingCount == 3)
        #expect(monitor.waitingCount(forRun: 3) == 2)
    }

    @Test func refreshesOnRunEventsAndFoldsBursts() async throws {
        let server = try makeServer()
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath), runsDebounce: .milliseconds(60))
        monitor.start()
        defer { monitor.stop() }
        func fetches(_ path: String) -> Int { server.requests.filter { $0.path == path }.count }

        try await waitFor { monitor.runs.count == 2 && monitor.questions.count == 1 }
        try await Task.sleep(for: .milliseconds(150))
        #expect(fetches("/v1/runs") == 1)
        #expect(fetches("/v1/questions?state=open") == 1)

        func send(_ kind: String, runId: String? = nil) {
            monitor.apply(.event(LoggedEvent(seq: (monitor.events.last?.seq ?? 1) + 1, ts: .now, kind: kind, runId: runId, data: .object([:]))))
        }
        // Approving a plan logs plan.approved, the run's move and one event
        // per ready task: one refresh.
        send(EventKind.planApproved, runId: "3")
        send(EventKind.runStateChanged, runId: "3")
        for _ in 0..<5 { send(EventKind.taskStateChanged, runId: "3") }
        try await waitFor { fetches("/v1/runs") == 2 }
        try await Task.sleep(for: .milliseconds(150))
        #expect(fetches("/v1/runs") == 2)
        // run.state_changed may close questions.
        #expect(fetches("/v1/questions?state=open") == 2)

        send(EventKind.questionAsked, runId: "3")
        try await waitFor { fetches("/v1/questions?state=open") == 3 }
        send(EventKind.agentStateChanged)
        send(EventKind.approvalRouted)
        try await Task.sleep(for: .milliseconds(200))
        #expect(fetches("/v1/runs") == 2, "\(server.requests.map(\.path))")
        #expect(fetches("/v1/questions?state=open") == 3)
        // Nothing watched: no boards fetched.
        #expect(fetches("/v1/runs/3") == 0)
    }

    @Test func aWatchedRunKeepsItsBoardLimitsReportAndActivityCurrent() async throws {
        let state = Locked("running")
        let server = try makeServer(board: {
            RunSamples.board.replacingOccurrences(of: #""state":"running","preset""#, with: #""state":"\#(state.value)","preset""#)
        }, extra: { request in
            request.path == "/v1/runs/3/report" ? .json(200, ##"{"markdown":"# Done"}"##) : nil
        })
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath), runsDebounce: .milliseconds(20))
        monitor.start()
        defer { monitor.stop() }
        func fetches(_ path: String) -> Int { server.requests.filter { $0.path == path }.count }
        try await waitFor { monitor.runs.count == 2 && monitor.agents.count == 3 }

        monitor.watchRun(3)
        monitor.watchRun(3)
        try await waitFor { monitor.boards[3] != nil && monitor.limits[3] != nil && !monitor.allApprovals.isEmpty }
        #expect(monitor.boards[3]?.tasks.count == 2)
        #expect(monitor.limits[3]?.chiefApprovals == 3)
        #expect(monitor.limitsSupported)
        // Running: no report to fetch yet.
        #expect(monitor.reports[3] == nil)
        #expect(fetches("/v1/runs/3/report") == 0)
        // The run's activity: its agents' approvals, auto-allowed ones too.
        #expect(monitor.approvals(forRun: 3).map(\.id) == [21, 20, 19])
        // The board's counts reach the run list.
        #expect(monitor.run(id: 3)?.counts?.total == 4)

        // Another run's events leave this board alone; its own refresh it.
        let boards = fetches("/v1/runs/3")
        monitor.apply(.event(LoggedEvent(seq: 10, ts: .now, kind: EventKind.taskStateChanged, runId: "8", data: .object([:]))))
        try await Task.sleep(for: .milliseconds(120))
        #expect(fetches("/v1/runs/3") == boards)

        state.value = "done"
        monitor.apply(.event(LoggedEvent(seq: 11, ts: .now, kind: EventKind.runStateChanged, runId: "3", data: .object([:]))))
        try await waitFor { monitor.boards[3]?.run.state == .done && monitor.reports[3] == .markdown("# Done") }
        #expect(monitor.run(id: 3)?.state == .done)

        // Approvals change the chief's count.
        let limits = fetches("/v1/runs/3/limits")
        monitor.apply(.event(LoggedEvent(seq: 12, ts: .now, kind: EventKind.approvalDecided, data: .object([:]))))
        try await waitFor { fetches("/v1/runs/3/limits") == limits + 1 }

        // Counted: one unwatch keeps it, the second drops it.
        monitor.unwatchRun(3)
        #expect(monitor.watches[3] != nil)
        monitor.unwatchRun(3)
        #expect(monitor.watches[3] == nil)
        let after = fetches("/v1/runs/3")
        monitor.apply(.event(LoggedEvent(seq: 13, ts: .now, kind: EventKind.taskStateChanged, runId: "3", data: .object([:]))))
        try await Task.sleep(for: .milliseconds(120))
        #expect(fetches("/v1/runs/3") == after)
        // The last board stays for the next visit.
        #expect(monitor.boards[3] != nil)
    }

    @Test func anOlderDaemonWithoutRunsIsNotAnError() async throws {
        let server = try FakeDaemon { request in
            switch request.path {
            case "/v1/status": .json(200, Samples.status)
            case let path where path.hasPrefix("/v1/events/stream"): .stream(": open\n\n")
            case "/v1/projects": .json(200, ProjectSamples.page)
            case "/v1/runs/3": .json(200, RunSamples.board)
            default: .empty(404)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath), runsDebounce: .milliseconds(10))
        monitor.start()
        defer { monitor.stop() }

        try await waitFor { monitor.projects.count == 2 && !monitor.runsSupported && !monitor.questionsSupported }
        #expect(monitor.runsError == nil)
        #expect(monitor.runs.isEmpty && monitor.questions.isEmpty)

        // Runs, but not the chief's or approvals' routes yet.
        monitor.watchRun(3)
        try await waitFor { monitor.boards[3] != nil && !monitor.limitsSupported }
        #expect(monitor.boardErrors[3] == nil)
    }

    @Test func actionsUpdateAtOnce() async throws {
        let awaiting = RunSamples.board.replacingOccurrences(of: #""state":"running","preset""#, with: #""state":"awaiting_plan_approval","preset""#)
        let server = try FakeDaemon { request in
            switch (request.method, request.path) {
            case ("POST", "/v1/runs"): .json(201, RunSamples.run.replacingOccurrences(of: #""id":3"#, with: #""id":5"#).replacingOccurrences(of: #""state":"running""#, with: #""state":"planning""#))
            case ("POST", "/v1/runs/3/approve-plan"): .json(200, RunSamples.board)
            case ("POST", "/v1/runs/3/cancel"): .json(200, RunSamples.board.replacingOccurrences(of: #""state":"running","preset""#, with: #""state":"cancelled","reason":"wrong goal","preset""#))
            case ("PATCH", "/v1/tasks/7"): .json(200, RunSamples.board.replacingOccurrences(of: #""state":"blocked""#, with: #""state":"failed""#))
            case ("POST", "/v1/questions/4/answer"): .json(409, #"{"error":"question 4 is already answered"}"#)
            case ("POST", "/v1/questions/5/answer"): .json(200, RunSamples.question.replacingOccurrences(of: #""id":4"#, with: #""id":5"#))
            case ("POST", "/v1/runs/3/message"): .empty(202)
            case ("PUT", "/v1/runs/3/limits"): .json(200, #"{"preset":"strict","approval_cap":4,"chief_approvals":0}"#)
            // The lists fail, so whatever changes comes from the actions.
            default: .json(503, #"{"error":"busy"}"#)
            }
        }
        defer { server.stop() }
        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath), runsDebounce: .milliseconds(10))

        let run = try await monitor.createRun(NewRun(workspaceId: 3, goal: "add rate limiting"))
        #expect(run.id == 5 && run.state == .planning)
        #expect(monitor.runs.map(\.id) == [5])

        monitor.store(try JSONDecoder.cos().decode(Board.self, from: Data(awaiting.utf8)))
        #expect(monitor.plansAwaitingApproval.map(\.id) == [3])
        #expect(monitor.waitingCount == 1)

        try await monitor.approvePlan(runId: 3)
        #expect(monitor.run(id: 3)?.state == .running)
        #expect(monitor.plansAwaitingApproval.isEmpty)

        let task = monitor.boards[3]!.task(id: 7)!
        try await monitor.updateTask(task, to: .failed, reason: "not worth it")
        #expect(monitor.boards[3]?.task(id: 7)?.state == .failed)

        try await monitor.cancelRun(id: 3, reason: "wrong goal")
        #expect(monitor.run(id: 3)?.state == .cancelled)
        #expect(monitor.run(id: 3)?.reason == "wrong goal")
        #expect(monitor.currentRun(inWorkspace: 3)?.id == 5)

        try await monitor.messageChief(runId: 3, text: "hi")
        #expect(try await monitor.setLimits(runId: 3, preset: .strict, approvalCap: 4).approvalCap == 4)
        #expect(monitor.limits[3]?.preset == .strict)

        // Answered elsewhere first: it leaves the list, and the app hears why.
        let question = try JSONDecoder.cos().decode(Question.self, from: Data(RunSamples.question.utf8))
        monitor.questions = [question]
        await #expect(throws: CosError.conflict("question 4 is already answered")) { try await monitor.answer(question, "redis") }
        #expect(monitor.questions.isEmpty)
        var other = question
        other.id = 5
        monitor.questions = [other]
        _ = try await monitor.answer(other, "memory")
        #expect(monitor.questions.isEmpty)

        try await waitFor { monitor.runsError != nil }
        #expect(monitor.runsError == "busy")
    }

    private func waitFor(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            if ContinuousClock.now > deadline { Issue.record("timed out"); return }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}

/// A value the fake daemon's handler reads while the test changes it.
final class Locked<Value: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Value
    init(_ value: Value) { stored = value }
    var value: Value {
        get { lock.withLock { stored } }
        set { lock.withLock { stored = newValue } }
    }
}
