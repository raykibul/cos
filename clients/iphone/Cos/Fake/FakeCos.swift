import CosKit
import Foundation

/// An in-memory daemon with invented data, for `-fakeData` (screenshots and
/// trying the app without a Mac) and for the view-model tests. It mirrors
/// the desktop's fake bridge (clients/desktop/src/bridge/fake):
///
/// - two projects: atlas (lisbon, porto, madrid) and harbor (kyoto,
///   nairobi); porto and kyoto have no title
/// - atlas's Chief of Staff (agent 40) has a conversation and started
///   agents 7 and 8; harbor's hasn't started
/// - an approval waiting for you (agent 8) and one with a run's chief
/// - agents needing a Claude login (11) and the trust dialog (12)
/// - runs: 3 running (tasks in several states, with a question), 4 with a
///   plan waiting, 2 done with a report, 1 cancelled
/// - a tailnet (`FakeData.peers(answering:)`): gpu-box runs cos and isn't
///   saved, mini runs no cos, the iPhone itself, an offline iPad, and a
///   tagged CI server
///
/// Actions change the data and log events, as the daemon would, so the
/// app's live updates work against it too.
actor FakeCos: CosService {
    private let now: Int64
    private var refusal: RefusalCode?
    private let unreachable: Bool
    private let replyDelay: Duration?
    /// The saved host this fake plays, which its tailnet list leaves out.
    private let hostName: String?
    /// Answers the peer list as an older daemon does over the tailnet.
    private let peersHostOnly: Bool
    /// How long the peer list takes, for timeout tests.
    private let peersDelay: Duration?
    private let data: FakeData

    private var projectList: [Project] = []
    private var agentList: [Agent] = []
    private var approvalList: [Approval] = []
    private var runList: [Run] = []
    private var tasks: [RunTask] = []
    private var questionList: [Question] = []
    private var staffs: [Int: Staff] = [:]
    private var transcripts: [Int: [TranscriptMessage]] = [:]
    private var inboxes: [Int: [StaffItem]] = [:]
    private var reports: [Int: String] = [:]
    private var log: [LoggedEvent] = []
    private var subscribers: [UUID: AsyncThrowingStream<LoggedEvent, Error>.Continuation] = [:]
    private var nextId = 100

    /// Requests in order, as `METHOD path`, for tests.
    private(set) var calls: [String] = []

    init(
        now: Date = Date(), refusal: RefusalCode? = nil, unreachable: Bool = false, replyDelay: Duration? = .milliseconds(1500),
        hostName: String? = nil, peersHostOnly: Bool = false, peersDelay: Duration? = nil
    ) {
        self.now = Int64(now.timeIntervalSince1970 * 1000)
        self.refusal = refusal
        self.unreachable = unreachable
        self.replyDelay = replyDelay
        self.hostName = hostName
        self.peersHostOnly = peersHostOnly
        self.peersDelay = peersDelay
        let data = FakeData(now: self.now)
        self.data = data
        projectList = data.projects
        agentList = data.agents
        approvalList = data.approvals
        runList = data.runs
        tasks = data.tasks
        questionList = data.questions
        staffs = data.staffs
        transcripts = data.transcripts
        inboxes = data.inboxes
        reports = data.reports
        log = [LoggedEvent(seq: 1, ts: Date(unixMilliseconds: self.now - 3_600_000), kind: "daemon.started", data: .object(["version": .string("0.1.2")]))]
    }

    // MARK: Status and events

    func status() async throws -> Status {
        try check("GET /v1/status")
        return Status(
            version: "0.1.2", pid: 4242, startedAt: Date(unixMilliseconds: now - 3_600_000), home: "/Users/me/.cos",
            events: log.count, lastSeq: log.last?.seq ?? 0
        )
    }

    func events(after: Int, limit: Int) async throws -> [LoggedEvent] {
        try check("GET /v1/events")
        return Array(log.filter { $0.seq > after }.prefix(limit))
    }

    nonisolated func stream(after: Int) -> AsyncThrowingStream<LoggedEvent, Error> {
        let id = UUID()
        return AsyncThrowingStream { continuation in
            Task { await self.subscribe(id, after: after, continuation) }
            continuation.onTermination = { _ in Task { await self.unsubscribe(id) } }
        }
    }

    private func subscribe(_ id: UUID, after: Int, _ continuation: AsyncThrowingStream<LoggedEvent, Error>.Continuation) {
        if let error = failure() {
            continuation.finish(throwing: error)
            return
        }
        for event in log where event.seq > after { continuation.yield(event) }
        subscribers[id] = continuation
    }

    private func unsubscribe(_ id: UUID) {
        subscribers[id] = nil
    }

    private func emit(_ kind: String, _ data: [String: JSONValue], runId: Int? = nil) {
        let event = LoggedEvent(
            seq: (log.last?.seq ?? 0) + 1, ts: Date(), kind: kind, runId: runId.map(String.init), data: .object(data)
        )
        log.append(event)
        for continuation in subscribers.values { continuation.yield(event) }
    }

    private func failure() -> CosError? {
        if unreachable {
            return .unreachable(address: "studio.tail0000.ts.net:7700", detail: "There's no route to it. Is Tailscale on, on this device?")
        }
        if let refusal {
            let message: String
            switch refusal {
            case .devicePending: message = "this device (iPhone) is waiting for approval. On studio, run `cos devices approve 3`"
            case .deviceDenied: message = "this device (iPhone) was denied. On studio, run `cos devices remove 3` to start over"
            case .wrongUser: message = "this device is logged in to Tailscale as someone@example.com; studio belongs to me@example.com"
            default: message = "refused: \(refusal.rawValue)"
            }
            return .refused(Refusal(status: refusal == .tailscaleUnavailable ? 503 : 403, code: refusal, message: message))
        }
        return nil
    }

    private func check(_ call: String) throws {
        calls.append(call)
        if let error = failure() { throw error }
    }

    private func newId() -> Int {
        nextId += 1
        return nextId
    }

    private var clock: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    // MARK: Projects

    func projects() async throws -> [Project] {
        try check("GET /v1/projects")
        return projectList
    }

    func createWorkspace(projectId: Int, base: WorkspaceBase, title: String?) async throws -> Workspace {
        try check("POST /v1/projects/\(projectId)/workspaces")
        guard let index = projectList.firstIndex(where: { $0.id == projectId }) else { throw CosError.notFound("unknown project \(projectId)") }
        let id = newId()
        let workspace = Workspace(
            id: id, projectId: projectId, name: "dakar", title: title, branch: "\(projectList[index].name)/dakar",
            path: "/Users/me/.cos/worktrees/\(projectList[index].name)-\(projectId)/dakar", base: base, createdAt: Date()
        )
        projectList[index].workspaces.append(workspace)
        emit("workspace.created", ["workspace_id": .number(Double(id)), "project_id": .number(Double(projectId))])
        return workspace
    }

    // MARK: Agents and approvals

    func agents(workspaceId: Int?, state: AgentState?) async throws -> [Agent] {
        try check("GET /v1/agents")
        return agentList
            .filter { workspaceId == nil || $0.workspaceId == workspaceId }
            .filter { state == nil || $0.state == state }
            .sorted { $0.startedMs > $1.startedMs }
    }

    func agent(id: Int) async throws -> Agent {
        try check("GET /v1/agents/\(id)")
        guard let agent = agentList.first(where: { $0.id == id }) else { throw CosError.notFound("unknown agent \(id)") }
        return agent
    }

    func createAgent(workspaceId: Int, _ request: NewAgent) async throws -> Agent {
        try check("POST /v1/workspaces/\(workspaceId)/agents")
        let id = newId()
        let agent = Agent(id: id, workspaceId: workspaceId, title: request.title ?? String((request.prompt ?? "Agent \(id)").prefix(60)), state: .starting, startedMs: clock)
        agentList.append(agent)
        emit("agent.started", ["agent_id": .number(Double(id))])
        return agent
    }

    func sendInput(agentId: Int, text: String) async throws {
        try check("POST /v1/agents/\(agentId)/input")
        guard let index = agentList.firstIndex(where: { $0.id == agentId }) else { throw CosError.notFound("unknown agent \(agentId)") }
        guard agentList[index].state.isAlive else { throw CosError.conflict("agent \(agentId) has exited; resume it first") }
        setState(agentId, .working)
        if let replyDelay {
            Task {
                try? await Task.sleep(for: replyDelay)
                self.finishTurn(agentId, message: "Done. I looked into “\(text.prefix(60))” and left a summary in the terminal.")
            }
        }
    }

    private func finishTurn(_ agentId: Int, message: String) {
        guard let index = agentList.firstIndex(where: { $0.id == agentId }) else { return }
        agentList[index].lastMessage = message
        setState(agentId, .idle)
    }

    private func setState(_ agentId: Int, _ state: AgentState) {
        guard let index = agentList.firstIndex(where: { $0.id == agentId }) else { return }
        let from = agentList[index].state
        agentList[index].state = state
        agentList[index].lastActivityMs = clock
        if !state.isAlive { agentList[index].exitedMs = clock }
        emit("agent.state_changed", ["agent_id": .number(Double(agentId)), "state": .string(state.rawValue), "from": .string(from.rawValue)])
    }

    // MARK: Test hooks

    /// Starts or stops refusing this device, as approving it on the host would.
    func setRefusal(_ code: RefusalCode?) {
        refusal = code
    }

    /// Moves an agent to a state, with its `agent.state_changed` event.
    func setAgentState(_ id: Int, _ state: AgentState) {
        setState(id, state)
    }

    func trustAgent(id: Int) async throws {
        try check("POST /v1/agents/\(id)/trust")
        guard agentList.first(where: { $0.id == id })?.state == .needsTrust else { throw CosError.conflict("agent \(id) isn't asking for trust") }
        setState(id, .idle)
    }

    func stopAgent(id: Int) async throws {
        try check("POST /v1/agents/\(id)/stop")
        setState(id, .exited)
    }

    func resumeAgent(id: Int) async throws -> Agent {
        try check("POST /v1/agents/\(id)/resume")
        guard let index = agentList.firstIndex(where: { $0.id == id }) else { throw CosError.notFound("unknown agent \(id)") }
        guard agentList[index].state.canResume else { throw CosError.conflict("agent \(id) is still running") }
        agentList[index].exitedMs = nil
        agentList[index].exitCode = nil
        setState(id, .idle)
        return agentList[index]
    }

    func screen(agentId: Int) async throws -> AgentScreen {
        try check("GET /v1/agents/\(agentId)/screen")
        guard let agent = agentList.first(where: { $0.id == agentId }) else { throw CosError.notFound("unknown agent \(agentId)") }
        return FakeData.screen(for: agent)
    }

    func approvals(state: ApprovalState?) async throws -> [Approval] {
        try check("GET /v1/approvals")
        return approvalList.filter { state == nil || $0.state == state }.sorted { $0.createdMs > $1.createdMs }
    }

    func decide(approvalId: Int, _ decision: ApprovalDecision, message: String?) async throws -> Approval {
        try check("POST /v1/approvals/\(approvalId)")
        guard let index = approvalList.firstIndex(where: { $0.id == approvalId }) else { throw CosError.notFound("unknown approval \(approvalId)") }
        guard approvalList[index].state == .pending else { throw CosError.conflict("approval \(approvalId) is no longer pending") }
        approvalList[index].state = decision == .allow ? .allowed : .denied
        approvalList[index].decidedBy = "user"
        approvalList[index].decidedMs = clock
        let agentId = approvalList[index].agentId
        if let agent = agentList.firstIndex(where: { $0.id == agentId }) { agentList[agent].pendingApprovalId = nil }
        emit("approval.decided", ["approval_id": .number(Double(approvalId)), "state": .string(approvalList[index].state.rawValue), "decided_by": .string("user")])
        setState(agentId, .working)
        return approvalList[index]
    }

    // MARK: Runs

    func runs() async throws -> [Run] {
        try check("GET /v1/runs")
        return runList.map { run in
            var run = run
            run.counts = TaskCounts(tasks.filter { $0.runId == run.id })
            return run
        }.sorted { $0.createdAt > $1.createdAt }
    }

    func createRun(_ request: NewRun) async throws -> Run {
        try check("POST /v1/runs")
        if runList.contains(where: { $0.workspaceId == request.workspaceId && !$0.state.isFinished }) {
            throw CosError.conflict("workspace \(request.workspaceId) already has an unfinished run")
        }
        let run = Run(id: newId(), workspaceId: request.workspaceId, goal: request.goal, state: .planning, createdAt: Date())
        runList.append(run)
        emit("run.created", ["run_id": .number(Double(run.id)), "workspace_id": .number(Double(run.workspaceId))], runId: run.id)
        return run
    }

    func board(runId: Int) async throws -> Board {
        try check("GET /v1/runs/\(runId)")
        guard let run = runList.first(where: { $0.id == runId }) else { throw CosError.notFound("unknown run \(runId)") }
        let workspace = projectList.lazy.flatMap(\.workspaces).first { $0.id == run.workspaceId }
        return Board(run: run, workspace: workspace, tasks: tasks.filter { $0.runId == runId })
    }

    func approvePlan(runId: Int) async throws -> Board {
        try check("POST /v1/runs/\(runId)/approve-plan")
        guard let index = runList.firstIndex(where: { $0.id == runId }) else { throw CosError.notFound("unknown run \(runId)") }
        guard runList[index].state == .awaitingPlanApproval else { throw CosError.conflict("run \(runId) has no plan waiting") }
        runList[index].state = .running
        runList[index].updatedAt = Date()
        for task in tasks.indices where tasks[task].runId == runId && tasks[task].dependsOn.isEmpty {
            tasks[task].state = .ready
        }
        emit("plan.approved", ["run_id": .number(Double(runId))], runId: runId)
        emit("run.state_changed", ["run_id": .number(Double(runId)), "from": .string("awaiting_plan_approval"), "to": .string("running")], runId: runId)
        return try await board(runId: runId)
    }

    func cancelRun(id: Int, reason: String?) async throws -> Board {
        try check("POST /v1/runs/\(id)/cancel")
        guard let index = runList.firstIndex(where: { $0.id == id }) else { throw CosError.notFound("unknown run \(id)") }
        guard !runList[index].state.isFinished else { throw CosError.conflict("run \(id) is already finished") }
        let from = runList[index].state
        runList[index].state = .cancelled
        runList[index].reason = reason ?? "Cancelled from the iPhone"
        for task in tasks.indices where tasks[task].runId == id && !tasks[task].state.isFinished {
            tasks[task].state = .cancelled
        }
        emit("run.state_changed", ["run_id": .number(Double(id)), "from": .string(from.rawValue), "to": .string("cancelled")], runId: id)
        return try await board(runId: id)
    }

    func questions(state: QuestionState?) async throws -> [Question] {
        try check("GET /v1/questions")
        return questionList.filter { state == nil || $0.state == state }
    }

    func answer(questionId: Int, _ answer: String) async throws -> Question {
        try check("POST /v1/questions/\(questionId)/answer")
        guard let index = questionList.firstIndex(where: { $0.id == questionId }) else { throw CosError.notFound("unknown question \(questionId)") }
        guard questionList[index].state == .open else { throw CosError.conflict("question \(questionId) is already answered") }
        questionList[index].state = .answered
        questionList[index].answer = answer
        questionList[index].answeredMs = clock
        emit("question.answered", ["question_id": .number(Double(questionId)), "run_id": .number(Double(questionList[index].runId ?? 0))])
        return questionList[index]
    }

    func messageChief(runId: Int, text: String) async throws {
        try check("POST /v1/runs/\(runId)/message")
    }

    func report(runId: Int) async throws -> String {
        try check("GET /v1/runs/\(runId)/report")
        guard let report = reports[runId] else { throw CosError.notFound("run \(runId) has no report yet") }
        return report
    }

    // MARK: The Chief of Staff

    func staff(projectId: Int) async throws -> Staff {
        try check("GET /v1/projects/\(projectId)/staff")
        guard let staff = staffs[projectId] else { throw CosError.notFound("unknown project \(projectId)") }
        return staff
    }

    func startStaff(projectId: Int) async throws -> Staff {
        try check("POST /v1/projects/\(projectId)/staff/start")
        return try ensureStaff(projectId)
    }

    private func ensureStaff(_ projectId: Int) throws -> Staff {
        guard var staff = staffs[projectId] else { throw CosError.notFound("unknown project \(projectId)") }
        if staff.agentId == nil {
            let id = newId()
            let name = projectList.first { $0.id == projectId }?.name ?? "project"
            agentList.append(Agent(id: id, workspaceId: 900 + projectId, role: .staff, title: "Chief of Staff", state: .idle, model: staff.model, startedMs: clock))
            staff.agentId = id
            staff.state = .idle
            staff.branch = "\(name)/chief"
            staffs[projectId] = staff
            transcripts[id] = []
            emit("staff.started", ["project_id": .number(Double(projectId)), "agent_id": .number(Double(id))])
        }
        return staffs[projectId]!
    }

    func messageStaff(projectId: Int, text: String) async throws {
        try check("POST /v1/projects/\(projectId)/staff/message")
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw CosError.http(status: 400, message: "the message is empty") }
        let staff = try ensureStaff(projectId)
        guard let agentId = staff.agentId else { return }
        append(agentId, role: .user, kind: .text, text: text)
        setStaffState(projectId, .working)
        if let replyDelay {
            Task {
                try? await Task.sleep(for: replyDelay)
                self.reply(projectId, agentId, to: text)
            }
        }
    }

    private func reply(_ projectId: Int, _ agentId: Int, to text: String) {
        append(agentId, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__workspaces_list"))
        append(agentId, role: .assistant, kind: .text, text: "On it. I checked the workspaces: **lisbon** is still running the test suite and **porto** is waiting for your approval. I'll report back when either finishes.")
        setStaffState(projectId, .idle)
    }

    private func append(_ agentId: Int, role: MessageRole, kind: MessageKind, text: String = "", tool: ToolCall? = nil) {
        let n = (transcripts[agentId]?.last?.n ?? 0) + 1
        transcripts[agentId, default: []].append(TranscriptMessage(n: n, role: role, kind: kind, text: text, tool: tool, tsMs: clock))
        emit("agent.transcript", ["agent_id": .number(Double(agentId)), "next": .number(Double(n))])
    }

    private func setStaffState(_ projectId: Int, _ state: AgentState) {
        guard let agentId = staffs[projectId]?.agentId else { return }
        staffs[projectId]?.state = state
        setState(agentId, state)
    }

    func staffInbox(projectId: Int) async throws -> [StaffItem] {
        try check("GET /v1/projects/\(projectId)/staff/inbox")
        return (inboxes[projectId] ?? []).sorted { $0.createdMs > $1.createdMs }
    }

    func transcript(agentId: Int, after: Int) async throws -> Transcript {
        try check("GET /v1/agents/\(agentId)/transcript?after=\(after)")
        let messages = transcripts[agentId] ?? []
        return Transcript(messages: messages.filter { $0.n > after }, next: messages.last?.n ?? 0)
    }

    // MARK: Environment

    func globalEnv() async throws -> EnvVars {
        try check("GET /v1/env")
        return ["CLAUDE_CONFIG_DIR": "~/.claude"]
    }

    func projectEnv(projectId: Int) async throws -> ProjectEnv {
        try check("GET /v1/projects/\(projectId)/env")
        let own: EnvVars = projectId == 2 ? ["CLAUDE_CONFIG_DIR": "~/.claude-work"] : [:]
        return ProjectEnv(variables: own, global: ["CLAUDE_CONFIG_DIR": "~/.claude"])
    }

    // MARK: The tailnet

    func tailnetPeers(port: Int?) async throws -> TailnetPeers {
        try check("GET /v1/tailnet/peers")
        if let peersDelay { try await Task.sleep(for: peersDelay) }
        if peersHostOnly {
            throw CosError.refused(Refusal(
                status: 403, code: .hostOnly,
                message: "devices and remote access can only be managed on the host (`cos devices`, `cos remote`)"
            ))
        }
        return data.peers(answering: hostName)
    }
}
