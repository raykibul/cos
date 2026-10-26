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
    /// What it is working on now. It keeps this itself (`brief_update`),
    /// and you can set it too.
    public var brief: String
    /// Who it is and how it works, written by you; empty when not set, or
    /// on an older daemon. Only you change it.
    public var identity: String
    public var worktree: String?
    public var branch: String?
    /// The agents it started that are `starting` or `working`.
    public var workingAgents: Int
    /// The agents it started that wait on you (an approval or the trust dialog).
    public var waitingOnYou: Int
    /// Coordination paused: news collects in its inbox, but no wakeups go in.
    public var paused: Bool
    /// News from its agents not delivered to it yet.
    public var updatesWaiting: Int

    public init(
        projectId: Int, agentId: Int? = nil, state: AgentState? = nil, model: String? = nil, brief: String = "",
        identity: String = "", worktree: String? = nil, branch: String? = nil, workingAgents: Int = 0, waitingOnYou: Int = 0,
        paused: Bool = false, updatesWaiting: Int = 0
    ) {
        self.projectId = projectId
        self.agentId = agentId
        self.state = state
        self.model = model
        self.brief = brief
        self.identity = identity
        self.worktree = worktree
        self.branch = branch
        self.workingAgents = workingAgents
        self.waitingOnYou = waitingOnYou
        self.paused = paused
        self.updatesWaiting = updatesWaiting
    }

    private enum CodingKeys: String, CodingKey {
        case projectId, agentId, state, model, brief, identity, worktree, branch, workingAgents, waitingOnYou, paused, updatesWaiting
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        projectId = try c.id(.projectId)
        agentId = c.int(.agentId)
        state = c.optional(AgentState.self, .state)
        model = c.string(.model).nilIfBlank
        brief = c.string(.brief) ?? ""
        identity = c.string(.identity) ?? ""
        worktree = c.string(.worktree)
        branch = c.string(.branch)
        workingAgents = c.int(.workingAgents) ?? 0
        waitingOnYou = c.int(.waitingOnYou) ?? 0
        paused = c.optional(Bool.self, .paused) ?? false
        updatesWaiting = c.int(.updatesWaiting) ?? 0
    }

    /// The most bytes a brief holds; the daemon refuses more.
    public static let maxBriefBytes = 4 * 1024
    /// The most bytes the identity holds; the daemon refuses more.
    public static let maxIdentityBytes = 8 * 1024
    /// About how long an identity should be, in words: a guide, not a limit.
    public static let identityWords = 500

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

/// One piece of news in a chief's inbox: what an agent it started
/// reported. The first line of `text` is its heading.
public struct StaffItem: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var projectId: Int?
    /// `agent`, `message`, or something newer.
    public var kind: String
    public var agentId: Int?
    public var text: String
    public var createdMs: Int64
    public var deliveredMs: Int64?

    public init(
        id: Int, projectId: Int? = nil, kind: String, agentId: Int? = nil, text: String,
        createdMs: Int64, deliveredMs: Int64? = nil
    ) {
        self.id = id
        self.projectId = projectId
        self.kind = kind
        self.agentId = agentId
        self.text = text
        self.createdMs = createdMs
        self.deliveredMs = deliveredMs
    }

    private enum CodingKeys: String, CodingKey {
        case id, projectId, kind, type, agentId, text, message, createdMs, tsMs, deliveredMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        projectId = c.int(.projectId)
        kind = c.string(.kind).nilIfBlank ?? c.string(.type).nilIfBlank ?? "update"
        agentId = c.int(.agentId)
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

// MARK: Memory notes

/// One of the chief's memory notes ("Memory notes" in
/// docs/chief-of-staff.md): what lasts, one subject per note. Only the name
/// and description go in its prompt; it reads the body when it needs it.
public struct StaffNote: Codable, Equatable, Hashable, Sendable, Identifiable {
    /// 1 to 64 lowercase letters, digits and `-`, unique in the project.
    public var name: String
    /// One line: what the note holds.
    public var description: String
    public var body: String
    public var createdMs: Int64
    public var updatedMs: Int64

    public var id: String { name }

    public init(name: String, description: String, body: String, createdMs: Int64 = 0, updatedMs: Int64 = 0) {
        self.name = name
        self.description = description
        self.body = body
        self.createdMs = createdMs
        self.updatedMs = updatedMs
    }

    private enum CodingKeys: String, CodingKey {
        case name, description, body, createdMs, updatedMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        guard let name = c.string(.name).nilIfBlank else {
            throw DecodingError.keyNotFound(CodingKeys.name, .init(codingPath: c.codingPath, debugDescription: "no name"))
        }
        self.name = name
        description = c.string(.description) ?? ""
        body = c.string(.body) ?? ""
        createdMs = c.int64(.createdMs) ?? 0
        updatedMs = c.int64(.updatedMs) ?? 0
    }

    public var updatedAt: Date { Date(unixMilliseconds: updatedMs) }

    /// The most bytes a body holds.
    public static let maxBodyBytes = 8 * 1024
    /// The most characters a description holds.
    public static let maxDescription = 200
    /// The most notes a project's chief keeps.
    public static let maxNotes = 50

    /// Why a name won't do, or `nil`.
    public static func nameProblem(_ name: String) -> String? {
        if name.isEmpty { return "Give it a name." }
        if name.count > 64 { return "Keep the name to 64 characters." }
        if name.wholeMatch(of: /[a-z0-9]([a-z0-9-]*[a-z0-9])?/) == nil {
            return "Use lowercase letters, digits and dashes, starting and ending with a letter or digit."
        }
        return nil
    }

    /// A description as it is saved: one line, trimmed.
    public static func oneLine(_ description: String) -> String {
        description.replacing(/\s*\n\s*/, with: " ").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Why a description won't do, or `nil`.
    public static func descriptionProblem(_ description: String) -> String? {
        let line = oneLine(description)
        if line.isEmpty { return "Say in a line what the note holds, so the chief knows when to read it." }
        if line.count > maxDescription { return "Keep it to \(maxDescription) characters." }
        return nil
    }

    /// Why a body won't do, or `nil`.
    public static func bodyProblem(_ body: String) -> String? {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return "Write what the chief should know." }
        if trimmed.utf8.count > maxBodyBytes { return "Keep it under \(maxBodyBytes.formatted()) bytes; split it into more notes." }
        return nil
    }
}

/// `GET /v1/projects/{id}/staff/memory`, by name.
public struct StaffMemory: Codable, Equatable, Sendable {
    public var notes: [StaffNote]

    public init(notes: [StaffNote]) {
        self.notes = notes
    }

    public init(from decoder: Decoder) throws {
        notes = try decoder.container(keyedBy: CodingKeys.self).list(StaffNote.self, .notes)
            .sorted { $0.name < $1.name }
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
        "brief_update": "Updated its brief",
        "memory_list": "Looked at its notes",
        "memory_read": "Read a note",
        "memory_write": "Saved a note",
        "memory_delete": "Deleted a note",
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
    /// Files you attached through cos, which `text` leaves out.
    public var attachments: [AttachedFile]

    public var id: Int { n }

    public init(
        n: Int, role: MessageRole, kind: MessageKind, text: String = "", tool: ToolCall? = nil, tsMs: Int64 = 0,
        attachments: [AttachedFile] = []
    ) {
        self.n = n
        self.role = role
        self.kind = kind
        self.text = text
        self.tool = tool
        self.tsMs = tsMs
        self.attachments = attachments
    }

    private enum CodingKeys: String, CodingKey {
        case n, role, kind, text, tool, tsMs, attachments
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
        attachments = c.list(AttachedFile.self, .attachments)
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

    /// For the first prompt of an agent a Chief of Staff started: the
    /// prompt without the line cos appends to it ("[cos] The Chief of Staff
    /// of <project> started you. …"); `nil` for anything else.
    public var spawnedPrompt: String? {
        guard role == .user, let match = text.firstMatch(of: /\s*\[cos\] The Chief of Staff of .+? started you\.(?s:.*)$/) else {
            return nil
        }
        return String(text[..<match.range.lowerBound])
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
