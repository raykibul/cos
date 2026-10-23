import Foundation

// The Chief of Staff (docs/chief-of-staff.md): its record per project, its
// inbox, and any agent's transcript. Decoded leniently: unknown fields are
// ignored and kinds stay open strings.

/// `GET /v1/projects/{id}/staff`: a project's Chief of Staff. It exists
/// even before it first starts.
public struct Staff: Codable, Equatable, Sendable {
    public var projectId: Int
    /// `nil` before it first starts.
    public var agentId: Int?
    /// `nil` before it first starts.
    public var state: AgentState?
    /// The model it runs; `nil` for Claude's default (or `COS_STAFF_MODEL`).
    public var model: String?
    public var brief: String
    public var worktree: String?
    public var branch: String?
    /// The agents it started that are `starting` or `working`.
    public var workingAgents: Int
    /// The agents it started that wait on you (an approval or the trust dialog).
    public var waitingOnYou: Int

    public init(
        projectId: Int, agentId: Int? = nil, state: AgentState? = nil, model: String? = nil, brief: String = "",
        worktree: String? = nil, branch: String? = nil, workingAgents: Int = 0, waitingOnYou: Int = 0
    ) {
        self.projectId = projectId
        self.agentId = agentId
        self.state = state
        self.model = model
        self.brief = brief
        self.worktree = worktree
        self.branch = branch
        self.workingAgents = workingAgents
        self.waitingOnYou = waitingOnYou
    }

    private enum CodingKeys: String, CodingKey {
        case projectId, agentId, state, model, brief, worktree, branch, workingAgents, waitingOnYou
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        projectId = try c.id(.projectId)
        agentId = c.int(.agentId)
        state = c.optional(AgentState.self, .state)
        model = c.string(.model).nilIfBlank
        brief = c.string(.brief) ?? ""
        worktree = c.string(.worktree)
        branch = c.string(.branch)
        workingAgents = c.int(.workingAgents) ?? 0
        waitingOnYou = c.int(.waitingOnYou) ?? 0
    }

    /// Whether it has ever been started.
    public var hasStarted: Bool { agentId != nil }

    /// "Ready for your next message", "Working…", "Not started".
    public var stateLabel: String {
        guard hasStarted, let state else { return "Not started" }
        switch state {
        case .idle: return "Ready for your next message"
        case .working, .starting: return "Working…"
        case .waitingApproval: return "Waiting for your approval"
        case .needsTrust: return "Waiting for the trust dialog"
        case .needsLogin: return "Waiting for a Claude login"
        case .exited, .interrupted: return "Stopped · a message restarts it"
        default: return state.rawValue.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }
}

/// One piece of news in a chief's inbox: what an agent or run it started
/// reported. The first line of `text` is its heading.
public struct StaffItem: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var projectId: Int?
    /// `agent`, `run`, `message`, or something newer.
    public var kind: String
    public var agentId: Int?
    public var runId: Int?
    public var text: String
    public var createdMs: Int64
    public var deliveredMs: Int64?

    public init(
        id: Int, projectId: Int? = nil, kind: String, agentId: Int? = nil, runId: Int? = nil, text: String,
        createdMs: Int64, deliveredMs: Int64? = nil
    ) {
        self.id = id
        self.projectId = projectId
        self.kind = kind
        self.agentId = agentId
        self.runId = runId
        self.text = text
        self.createdMs = createdMs
        self.deliveredMs = deliveredMs
    }

    private enum CodingKeys: String, CodingKey {
        case id, projectId, kind, type, agentId, runId, text, message, createdMs, tsMs, deliveredMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        projectId = c.int(.projectId)
        kind = c.string(.kind).nilIfBlank ?? c.string(.type).nilIfBlank ?? "update"
        agentId = c.int(.agentId)
        runId = c.int(.runId)
        text = c.string(.text) ?? c.string(.message) ?? ""
        createdMs = c.int64(.createdMs) ?? c.int64(.tsMs) ?? 0
        deliveredMs = c.int64(.deliveredMs)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encodeIfPresent(projectId, forKey: .projectId)
        try c.encode(kind, forKey: .kind)
        try c.encodeIfPresent(agentId, forKey: .agentId)
        try c.encodeIfPresent(runId, forKey: .runId)
        try c.encode(text, forKey: .text)
        try c.encode(createdMs, forKey: .createdMs)
        try c.encodeIfPresent(deliveredMs, forKey: .deliveredMs)
    }

    /// The first line.
    public var heading: String {
        String(text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false).first ?? "")
    }

    /// Everything after the first line.
    public var body: String {
        let parts = text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
        return parts.count > 1 ? String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines) : ""
    }

    public var createdAt: Date { Date(unixMilliseconds: createdMs) }
}

/// `GET /v1/projects/{id}/staff/inbox`, newest first.
public struct StaffInbox: Codable, Equatable, Sendable {
    public var items: [StaffItem]

    public init(items: [StaffItem]) {
        self.items = items
    }

    public init(from decoder: Decoder) throws {
        items = try decoder.container(keyedBy: CodingKeys.self).list(StaffItem.self, .items)
            .sorted { $0.createdMs > $1.createdMs }
    }
}

// MARK: Transcripts

/// Who said it.
public struct MessageRole: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let user = MessageRole(rawValue: "user")
    public static let assistant = MessageRole(rawValue: "assistant")
}

/// What a transcript message is.
public struct MessageKind: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// What you typed or Claude said.
    public static let text = MessageKind(rawValue: "text")
    /// One tool call, with its result folded in.
    public static let tool = MessageKind(rawValue: "tool")
    /// A cos inbox wakeup: a user turn that starts with `[cos ·`.
    public static let update = MessageKind(rawValue: "update")
    /// Claude's `<task-notification>`: a subagent or background command it
    /// started finished. Claude types it in as a user turn; CosKit tags it.
    public static let task = MessageKind(rawValue: "task")
}

/// One tool call in a transcript.
public struct ToolCall: Codable, Equatable, Sendable {
    public var name: String
    /// Its main argument on one line, such as Bash's command.
    public var summary: String
    public var isError: Bool
    /// The agent one of cos's agent tools acted on: the one `agent_spawn`
    /// started (once its result is in), or the one it messaged, read or
    /// stopped.
    public var agentId: Int?

    public init(name: String, summary: String = "", isError: Bool = false, agentId: Int? = nil) {
        self.name = name
        self.summary = summary
        self.isError = isError
        self.agentId = agentId
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = c.string(.name).nilIfBlank ?? "tool"
        summary = c.string(.summary) ?? ""
        isError = c.optional(Bool.self, .isError) ?? false
        agentId = c.int(.agentId)
    }

    /// cos's `agent_spawn`: the chief started an agent.
    public var isSpawn: Bool { name == "mcp__cos__agent_spawn" }

    /// The name as a page shows it: cos's own tools in words.
    public var title: String {
        if name.hasPrefix("mcp__cos__") {
            let tool = String(name.dropFirst("mcp__cos__".count))
            return Self.cosTools[tool] ?? tool.replacingOccurrences(of: "_", with: " ")
        }
        if name.hasPrefix("mcp__") {
            let parts = name.dropFirst(5).components(separatedBy: "__")
            if parts.count >= 2 {
                return "\(parts[1...].joined(separator: "__").replacingOccurrences(of: "_", with: " ")) (\(parts[0]))"
            }
        }
        return name
    }

    /// One of cos's own tools, which the chief uses to coordinate.
    public var isCosTool: Bool { name.hasPrefix("mcp__cos__") }

    private static let cosTools = [
        "workspaces_list": "Looked at the workspaces",
        "workspace_create": "Created a workspace",
        "agent_spawn": "Started an agent",
        "agent_send": "Messaged an agent",
        "agent_read": "Checked on an agent",
        "agent_stop": "Stopped an agent",
        "run_start": "Started a run",
        "brief_update": "Updated its brief",
    ]
}

/// One message of an agent's transcript.
public struct TranscriptMessage: Codable, Equatable, Sendable, Identifiable {
    /// Its position in the transcript; `after=n` asks for what follows.
    public var n: Int
    public var role: MessageRole
    public var kind: MessageKind
    /// Markdown, for text and updates.
    public var text: String
    /// For tool calls.
    public var tool: ToolCall?
    public var tsMs: Int64

    public var id: Int { n }

    public init(n: Int, role: MessageRole, kind: MessageKind, text: String = "", tool: ToolCall? = nil, tsMs: Int64 = 0) {
        self.n = n
        self.role = role
        self.kind = kind
        self.text = text
        self.tool = tool
        self.tsMs = tsMs
    }

    private enum CodingKeys: String, CodingKey {
        case n, role, kind, text, tool, tsMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = try c.id(.n)
        role = c.optional(MessageRole.self, .role) ?? .assistant
        tool = c.optional(ToolCall.self, .tool)
        text = c.string(.text) ?? ""
        var kind = c.optional(MessageKind.self, .kind) ?? (tool != nil ? .tool : .text)
        // A wakeup the daemon didn't tag still reads as one.
        if kind == .text, role == .user, text.hasPrefix("[cos ·") { kind = .update }
        if kind == .text, role == .user, TaskNotification(text) != nil { kind = .task }
        self.kind = kind
        tsMs = c.int64(.tsMs) ?? 0
    }

    public var date: Date { Date(unixMilliseconds: tsMs) }

    /// For an update: `[cos · 3 updates]` → 3, one when there's no count.
    public var updateCount: Int {
        guard let match = text.firstMatch(of: /^\s*\[cos\s*·\s*(\d+)\s+update/) else { return 1 }
        return Int(match.1) ?? 1
    }

    /// For an update: its text without the `[cos · N updates]` header.
    public var updateBody: String {
        text.replacing(/^\s*\[cos[^\]\n]*\]\s*/, with: "").trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

extension TranscriptMessage {
    /// For a `<task-notification>` Claude typed in: what finished, and how.
    public var taskNotification: TaskNotification? {
        role == .user ? TaskNotification(text) : nil
    }
}

/// What Claude types into a session when a subagent or background command
/// it started finishes:
///
///     <task-notification>
///     <task-id>a066…</task-id>
///     <status>completed</status>
///     <summary>Agent "Update on open PRs" finished</summary>
///     <result>…markdown…</result>
///     </task-notification>
///
/// The format is Claude's and undocumented, so every tag is optional.
public struct TaskNotification: Equatable, Sendable {
    public var taskId: String?
    /// `completed`, `failed`, `killed`…
    public var status: String
    /// Claude's one line: `Agent "X" finished`.
    public var summary: String
    /// What the subagent said at the end, as markdown.
    public var result: String
    /// How long it ran, when Claude says.
    public var durationMs: Int64?

    public init(taskId: String? = nil, status: String, summary: String, result: String, durationMs: Int64? = nil) {
        self.taskId = taskId
        self.status = status
        self.summary = summary
        self.result = result
        self.durationMs = durationMs
    }

    /// `nil` unless `text` is a task notification.
    public init?(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("<task-notification>") else { return nil }
        func tag(_ name: String) -> String? {
            guard let open = trimmed.range(of: "<\(name)>"),
                  let close = trimmed.range(of: "</\(name)>", range: open.upperBound..<trimmed.endIndex)
            else { return nil }
            return String(trimmed[open.upperBound..<close.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        taskId = tag("task-id")
        status = tag("status") ?? "completed"
        summary = tag("summary") ?? ""
        result = tag("result") ?? ""
        let usage = tag("usage") ?? ""
        durationMs = usage.firstMatch(of: /duration_ms\D{0,3}(\d+)/).flatMap { Int64($0.1) }
    }

    /// What finished: the quoted name in `Agent "X" finished`, else the summary.
    public var title: String {
        if let match = summary.firstMatch(of: /"([^"]+)"/) { return String(match.1) }
        return summary.isEmpty ? "Background task" : summary
    }

    /// A subagent (Claude's Agent tool) rather than a background command.
    public var isAgent: Bool { summary.hasPrefix("Agent ") || summary.isEmpty }

    /// It ended well.
    public var succeeded: Bool { status == "completed" }
}

/// `GET /v1/agents/{id}/transcript?after=N`.
public struct Transcript: Codable, Equatable, Sendable {
    /// Oldest first.
    public var messages: [TranscriptMessage]
    /// Every message up to `next` is final; ask again with `after=next`.
    public var next: Int

    public init(messages: [TranscriptMessage], next: Int) {
        self.messages = messages
        self.next = next
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        messages = c.list(TranscriptMessage.self, .messages).sorted { $0.n < $1.n }
        next = c.int(.next) ?? messages.last?.n ?? 0
    }

    /// `messages` with a later page merged in, by `n`, oldest first. A
    /// message read again (a tool call that got its result) is replaced.
    public static func merge(_ current: [TranscriptMessage], _ incoming: [TranscriptMessage]) -> [TranscriptMessage] {
        guard !incoming.isEmpty else { return current }
        var byN: [Int: TranscriptMessage] = [:]
        for message in current { byN[message.n] = message }
        for message in incoming { byN[message.n] = message }
        return byN.values.sorted { $0.n < $1.n }
    }
}

// MARK: Events

extension EventKind {
    public static let agentTranscript = "agent.transcript"
    public static let staffStarted = "staff.started"
    public static let staffInbox = "staff.inbox"
    public static let staffChanged = "staff.changed"
    public static let staffStopped = "staff.stopped"
    public static let envChanged = "env.changed"
}

extension LoggedEvent {
    /// Whether this event may have changed a project's Chief of Staff.
    /// Agent events count too, since its state is an agent's.
    public var changesStaff: Bool {
        kind.hasPrefix("staff.") || kind.hasPrefix("agent.state") || kind == EventKind.agentExited
            || kind == EventKind.agentStarted || kind.hasPrefix("approval.")
    }
}
