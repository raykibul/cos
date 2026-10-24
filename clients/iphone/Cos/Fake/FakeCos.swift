import CosKit
import Foundation

/// An in-memory daemon with invented data, for `-fakeData` (screenshots and
/// trying the app without a Mac) and for the view-model tests. It mirrors
/// the desktop's fake bridge (clients/desktop/src/bridge/fake):
///
/// - three projects: atlas (lisbon, porto, madrid), harbor (kyoto,
///   nairobi) and opal (oslo); porto and kyoto have no title
/// - atlas's Chief of Staff (agent 40) has a conversation and started
///   agents 7 and 8; harbor's hasn't started; opal's (agent 41) is idle
///   with no conversation
/// - an approval waiting for you (agent 8)
/// - agents needing a Claude login (11) and the trust dialog (12)
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
    private var staffs: [Int: Staff] = [:]
    private var transcripts: [Int: [TranscriptMessage]] = [:]
    private var inboxes: [Int: [StaffItem]] = [:]
    /// Uploaded files with their bytes, by the folder they went into.
    private var files: [String: [(attachment: Attachment, data: Data)]] = [:]
    private var log: [LoggedEvent] = []
    private var subscribers: [UUID: AsyncThrowingStream<LoggedEvent, Error>.Continuation] = [:]
    /// Open terminal streams, by agent.
    private var terminals: [Int: [UUID: AsyncThrowingStream<TerminalEvent, Error>.Continuation]] = [:]
    /// Keystrokes sent to each agent's terminal, for tests.
    private(set) var keys: [Int: [UInt8]] = [:]
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
        staffs = data.staffs
        transcripts = data.transcripts
        inboxes = data.inboxes
        for (projectId, seeded) in data.files {
            let folder = Self.staffFolder(projectId)
            files[folder] = seeded.map { file -> (attachment: Attachment, data: Data) in
                let attachment = Attachment(
                    name: file.name, path: "\(folder)/.cos/attachments/\(file.name)", size: file.data.count, contentType: file.contentType
                )
                return (attachment: attachment, data: file.data)
            }
        }
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

    private func emit(_ kind: String, _ data: [String: JSONValue]) {
        let event = LoggedEvent(seq: (log.last?.seq ?? 0) + 1, ts: Date(), kind: kind, data: .object(data))
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
        try await messageStaff(projectId: projectId, text: text, attachments: [])
    }

    func messageStaff(projectId: Int, text: String, attachments names: [String]) async throws {
        try check("POST /v1/projects/\(projectId)/staff/message")
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !names.isEmpty else {
            throw CosError.http(status: 400, message: "the message is empty")
        }
        let folder = Self.staffFolder(projectId)
        let attached = try names.map { name -> AttachedFile in
            guard let file = files[folder]?.first(where: { $0.attachment.name == name || $0.attachment.path == name }) else {
                throw CosError.http(status: 400, message: "there is no attachment \(name)")
            }
            return file.attachment.attached
        }
        let staff = try ensureStaff(projectId)
        guard let agentId = staff.agentId else { return }
        append(agentId, role: .user, kind: .text, text: text, attachments: attached)
        setStaffState(projectId, .working)
        if let replyDelay {
            // A tool call first, then the answer: the chat shows what it's doing in between.
            Task {
                try? await Task.sleep(for: replyDelay)
                self.append(agentId, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__workspaces_list"))
                try? await Task.sleep(for: replyDelay)
                self.reply(projectId, agentId, to: text)
            }
        }
    }

    private func reply(_ projectId: Int, _ agentId: Int, to text: String) {
        append(agentId, role: .assistant, kind: .text, text: "On it. I checked the workspaces: **lisbon** is still running the test suite and **porto** is waiting for your approval. I'll report back when either finishes.")
        setStaffState(projectId, .idle)
    }

    private func append(
        _ agentId: Int, role: MessageRole, kind: MessageKind, text: String = "", tool: ToolCall? = nil, attachments: [AttachedFile] = []
    ) {
        let n = (transcripts[agentId]?.last?.n ?? 0) + 1
        transcripts[agentId, default: []].append(
            TranscriptMessage(n: n, role: role, kind: kind, text: text, tool: tool, tsMs: clock, attachments: attachments)
        )
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

    // MARK: Attachments

    func uploadAttachment(to target: AttachTarget, name: String, contentType: String, data: Data) async throws -> Attachment {
        try check("POST \(target.path)")
        guard data.count <= Attachment.maxBytes else { throw CosError.http(status: 413, message: "a file can be at most 32 MB") }
        let folder: String
        switch target {
        case .staff(let projectId):
            guard staffs[projectId] != nil else { throw CosError.notFound("unknown project \(projectId)") }
            folder = Self.staffFolder(projectId)
        case .agent(let id):
            guard agentList.contains(where: { $0.id == id }) else { throw CosError.notFound("unknown agent \(id)") }
            folder = agentFolder(id)
        case .workspace(let id):
            folder = "/Users/me/.cos/worktrees/workspace-\(id)"
        }
        return save(folder, name: name, contentType: contentType, data: data)
    }

    func attachmentData(agentId: Int, name: String) async throws -> Data {
        try check("GET /v1/agents/\(agentId)/attachments/\(name)")
        guard let file = files[agentFolder(agentId)]?.first(where: { $0.attachment.name == name }) else {
            throw CosError.notFound("there is no attachment \(name)")
        }
        return file.data
    }

    /// Saves a file as the daemon does: a safe name, made unique in its folder.
    private func save(_ folder: String, name: String, contentType: String, data: Data) -> Attachment {
        let clean = Self.safeName(name)
        let ext = (clean as NSString).pathExtension
        let stem = (clean as NSString).deletingPathExtension
        var candidate = clean
        var n = 2
        while files[folder]?.contains(where: { $0.attachment.name == candidate }) == true {
            candidate = ext.isEmpty ? "\(stem)-\(n)" : "\(stem)-\(n).\(ext)"
            n += 1
        }
        let attachment = Attachment(name: candidate, path: "\(folder)/.cos/attachments/\(candidate)", size: data.count, contentType: contentType)
        files[folder, default: []].append((attachment: attachment, data: data))
        return attachment
    }

    /// A chief's worktree, where its files go before and after it starts.
    private static func staffFolder(_ projectId: Int) -> String {
        "/Users/me/.cos/worktrees/project-\(projectId)/chief"
    }

    private func agentFolder(_ agentId: Int) -> String {
        if let projectId = staffs.first(where: { $0.value.agentId == agentId })?.key { return Self.staffFolder(projectId) }
        return "/Users/me/.cos/agents/\(agentId)"
    }

    private static func safeName(_ name: String) -> String {
        let base = (name as NSString).lastPathComponent
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: ".-_"))
        var clean = String(String.UnicodeScalarView(base.unicodeScalars.map { allowed.contains($0) ? $0 : "-" }))
        while clean.hasPrefix(".") || clean.hasPrefix("-") { clean.removeFirst() }
        return clean.isEmpty ? "file" : String(clean.prefix(100))
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

// MARK: Terminals

extension FakeCos: TerminalSource, TerminalControl {
    /// The agent's screen as a snapshot, then whatever is typed, echoed
    /// back, until the view goes away.
    nonisolated func terminal(agentId: Int) -> AsyncThrowingStream<TerminalEvent, Error> {
        AsyncThrowingStream { continuation in
            let id = UUID()
            let task = Task {
                do {
                    let screen = try await self.screen(agentId: agentId)
                    continuation.yield(FakeData.snapshot(of: screen))
                    await self.follow(agentId: agentId, id: id, continuation)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in
                task.cancel()
                Task { await self.unfollow(agentId: agentId, id: id) }
            }
        }
    }

    func sendKeys(agentId: Int, _ bytes: [UInt8]) async throws {
        try check("POST /v1/agents/\(agentId)/keys")
        keys[agentId, default: []] += bytes
        // Echo, as a shell would: a return starts a new line.
        let echo = bytes.flatMap { $0 == 0x0D ? [0x0D, 0x0A] : [$0] }
        for continuation in terminals[agentId, default: [:]].values {
            continuation.yield(.output(echo))
        }
    }

    func resizeAgent(id: Int, rows: Int, cols: Int) async throws {
        try check("POST /v1/agents/\(id)/resize")
        for continuation in terminals[id, default: [:]].values {
            continuation.yield(.resize(rows: rows, cols: cols))
        }
    }

    private func follow(agentId: Int, id: UUID, _ continuation: AsyncThrowingStream<TerminalEvent, Error>.Continuation) {
        terminals[agentId, default: [:]][id] = continuation
    }

    private func unfollow(agentId: Int, id: UUID) {
        terminals[agentId]?[id] = nil
    }
}
