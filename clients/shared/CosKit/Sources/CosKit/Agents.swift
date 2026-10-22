import Foundation

// Agents and approvals, as in docs/agents-api.md.
// States and roles are open strings rather than closed enums, so a value
// added by a newer daemon still decodes.

/// One Claude Code session the daemon started, running Claude's own UI in a
/// PTY the daemon owns, in a workspace's folder.
public struct Agent: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    /// `nil` only from a daemon that leaves it out. A Chief of Staff's is
    /// its hidden worktree, which no workspace list shows.
    public var workspaceId: Int?
    /// The run that started it; `nil` for agents you started yourself.
    public var runId: String?
    /// The task a worker or reviewer works on.
    public var taskId: Int?
    public var role: AgentRole
    public var title: String
    public var sessionId: String?
    public var pid: Int?
    public var state: AgentState
    public var model: String?
    public var permissionMode: String?
    /// Whether Claude loads your own settings.
    public var userSettings: Bool
    public var rows: Int
    public var cols: Int
    /// Unix milliseconds, like every time in this API.
    public var startedMs: Int64
    public var lastActivityMs: Int64?
    public var exitedMs: Int64?
    /// `nil` once exited means it was killed.
    public var exitCode: Int?
    /// The last thing Claude said at the end of a turn, cut to 2,000 characters.
    public var lastMessage: String?
    public var pendingApprovalId: Int?
    /// Only when it runs outside the workspace folder (a task's worktree).
    public var cwd: String?
    /// The Chief of Staff agent that started it, if one did.
    public var spawnedBy: Int?
    /// Claude's JSONL transcript, once a hook has reported it.
    public var transcriptPath: String?
    /// The extra environment variables it was launched with (names only).
    public var envNames: [String]

    public init(
        id: Int, workspaceId: Int?, runId: String? = nil, taskId: Int? = nil, role: AgentRole = .user, title: String,
        sessionId: String? = nil, pid: Int? = nil, state: AgentState, model: String? = nil,
        permissionMode: String? = nil, userSettings: Bool = true, rows: Int = 40, cols: Int = 120, startedMs: Int64,
        lastActivityMs: Int64? = nil, exitedMs: Int64? = nil, exitCode: Int? = nil,
        lastMessage: String? = nil, pendingApprovalId: Int? = nil, cwd: String? = nil, spawnedBy: Int? = nil,
        transcriptPath: String? = nil, envNames: [String] = []
    ) {
        self.id = id
        self.workspaceId = workspaceId
        self.runId = runId
        self.taskId = taskId
        self.role = role
        self.title = title
        self.sessionId = sessionId
        self.pid = pid
        self.state = state
        self.model = model
        self.permissionMode = permissionMode
        self.userSettings = userSettings
        self.rows = rows
        self.cols = cols
        self.startedMs = startedMs
        self.lastActivityMs = lastActivityMs
        self.exitedMs = exitedMs
        self.exitCode = exitCode
        self.lastMessage = lastMessage
        self.pendingApprovalId = pendingApprovalId
        self.cwd = cwd
        self.spawnedBy = spawnedBy
        self.transcriptPath = transcriptPath
        self.envNames = envNames
    }

    private enum CodingKeys: String, CodingKey {
        case id, workspaceId, runId, taskId, role, title, sessionId, pid, state, model, permissionMode, userSettings
        case rows, cols, startedMs, lastActivityMs, exitedMs, exitCode, lastMessage, pendingApprovalId
        case cwd, spawnedBy, transcriptPath, envNames
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        workspaceId = c.int(.workspaceId)
        // A number from the daemon; a string is taken too.
        runId = c.string(.runId)
        taskId = c.int(.taskId)
        role = c.optional(AgentRole.self, .role) ?? .user
        title = c.string(.title).nilIfBlank ?? "Agent \(id)"
        sessionId = c.string(.sessionId)
        pid = c.int(.pid)
        state = c.optional(AgentState.self, .state) ?? .starting
        model = c.string(.model)
        permissionMode = c.string(.permissionMode)
        userSettings = c.optional(Bool.self, .userSettings) ?? true
        rows = c.int(.rows) ?? 40
        cols = c.int(.cols) ?? 120
        startedMs = c.int64(.startedMs) ?? 0
        lastActivityMs = c.int64(.lastActivityMs)
        exitedMs = c.int64(.exitedMs)
        exitCode = c.int(.exitCode)
        lastMessage = c.string(.lastMessage)
        pendingApprovalId = c.int(.pendingApprovalId)
        cwd = c.string(.cwd)
        spawnedBy = c.int(.spawnedBy)
        transcriptPath = c.string(.transcriptPath)
        envNames = c.strings(.envNames)
    }

    public var startedAt: Date { Date(unixMilliseconds: startedMs) }
    public var lastActivityAt: Date? { lastActivityMs.map(Date.init(unixMilliseconds:)) }

    /// Changes whenever the agent gets a new process (a resume), so a
    /// terminal knows to open a fresh stream.
    public var generation: String { "\(pid ?? -1)@\(startedMs)" }
}

public struct AgentState: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// Spawned; no hook yet.
    public static let starting = AgentState(rawValue: "starting")
    /// Claude is showing its trust dialog for the folder.
    public static let needsTrust = AgentState(rawValue: "needs_trust")
    /// Claude is showing its first-run or login screen: the agent's Claude
    /// config dir (`CLAUDE_CONFIG_DIR`) isn't logged in. Someone logs in in
    /// its terminal, then it carries on.
    public static let needsLogin = AgentState(rawValue: "needs_login")
    /// A prompt is running.
    public static let working = AgentState(rawValue: "working")
    /// A permission request is open.
    public static let waitingApproval = AgentState(rawValue: "waiting_approval")
    /// The turn ended; waiting for input.
    public static let idle = AgentState(rawValue: "idle")
    /// The process ended.
    public static let exited = AgentState(rawValue: "exited")
    /// It was running when the daemon stopped. Resume brings it back.
    public static let interrupted = AgentState(rawValue: "interrupted")

    /// Whether a Claude process is running for it. Unknown states count as
    /// running, since the daemon only adds states for live agents' phases.
    public var isAlive: Bool { self != .exited && self != .interrupted }
    /// Whether Resume applies.
    public var canResume: Bool { self == .exited || self == .interrupted }
    /// Whether it is blocked until you answer something.
    public var needsYou: Bool { self == .needsTrust || self == .waitingApproval || self == .needsLogin }
    /// Busy on a turn (or starting one).
    public var isBusy: Bool { self == .working || self == .starting }
}

public struct AgentRole: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// Started by you from an app or the CLI.
    public static let user = AgentRole(rawValue: "user")
    public static let chief = AgentRole(rawValue: "chief")
    public static let worker = AgentRole(rawValue: "worker")
    public static let reviewer = AgentRole(rawValue: "reviewer")
    /// A project's Chief of Staff.
    public static let staff = AgentRole(rawValue: "staff")
}

/// `POST /v1/workspaces/{id}/agents`. Every field is optional; empty
/// strings are left out so the daemon picks its defaults.
public struct NewAgent: Encodable, Equatable, Sendable {
    public var prompt: String?
    public var title: String?
    /// `sonnet`, `opus`, `haiku`, or `nil` for Claude's default.
    public var model: String?
    public var permissionMode: String?
    public var rows: Int?
    public var cols: Int?

    public init(prompt: String? = nil, title: String? = nil, model: String? = nil, permissionMode: String? = nil, rows: Int? = nil, cols: Int? = nil) {
        self.prompt = prompt.nonEmpty
        self.title = title.nonEmpty
        self.model = model.nonEmpty
        self.permissionMode = permissionMode.nonEmpty
        self.rows = rows
        self.cols = cols
    }

    private enum CodingKeys: String, CodingKey {
        case prompt, title, model, rows, cols
        case permissionMode = "permission_mode"
    }
}

/// A permission request from an agent.
public struct Approval: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var agentId: Int
    public var workspaceId: Int?
    public var toolName: String
    public var toolInput: JSONValue
    public var toolUseId: String?
    /// One line such as `Bash npm install left-pad`.
    public var summary: String
    public var state: ApprovalState
    public var decidedBy: String?
    public var message: String?
    public var createdMs: Int64
    public var decidedMs: Int64?
    /// The policy's tier (docs/orchestration.md). `nil` from a daemon without it.
    public var tier: ApprovalTier?
    /// Who answers it: you, the run's chief, or a rule (`auto`).
    public var route: ApprovalRoute?
    /// The policy's one-line reason.
    public var reason: String?

    public init(
        id: Int, agentId: Int, workspaceId: Int? = nil, toolName: String, toolInput: JSONValue = .null,
        toolUseId: String? = nil, summary: String, state: ApprovalState = .pending, decidedBy: String? = nil,
        message: String? = nil, createdMs: Int64, decidedMs: Int64? = nil,
        tier: ApprovalTier? = nil, route: ApprovalRoute? = nil, reason: String? = nil
    ) {
        self.id = id
        self.agentId = agentId
        self.workspaceId = workspaceId
        self.toolName = toolName
        self.toolInput = toolInput
        self.toolUseId = toolUseId
        self.summary = summary
        self.state = state
        self.decidedBy = decidedBy
        self.message = message
        self.createdMs = createdMs
        self.decidedMs = decidedMs
        self.tier = tier
        self.route = route
        self.reason = reason
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.id(.id)
        agentId = try c.id(.agentId)
        workspaceId = c.int(.workspaceId)
        toolName = c.string(.toolName) ?? "tool"
        toolInput = c.optional(JSONValue.self, .toolInput) ?? .null
        toolUseId = c.string(.toolUseId)
        summary = c.string(.summary) ?? toolName
        state = c.optional(ApprovalState.self, .state) ?? .pending
        decidedBy = c.string(.decidedBy)
        message = c.string(.message)
        createdMs = c.int64(.createdMs) ?? 0
        decidedMs = c.int64(.decidedMs)
        tier = c.optional(ApprovalTier.self, .tier)
        route = c.optional(ApprovalRoute.self, .route)
        reason = c.string(.reason)
    }

    public var createdAt: Date { Date(unixMilliseconds: createdMs) }

    /// What the tool will do, for a banner: the command for Bash, the path
    /// for file tools, otherwise the daemon's summary.
    public var detail: String {
        for key in ["command", "file_path", "path", "url", "pattern"] {
            if case .string(let value)? = toolInput[key], !value.isEmpty { return value }
        }
        return summary
    }
}

public struct ApprovalState: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let pending = ApprovalState(rawValue: "pending")
    public static let allowed = ApprovalState(rawValue: "allowed")
    public static let denied = ApprovalState(rawValue: "denied")
    /// Answered in the terminal, or the tool went ahead some other way.
    public static let settled = ApprovalState(rawValue: "settled")
    public static let expired = ApprovalState(rawValue: "expired")
}

public enum ApprovalDecision: String, Codable, Sendable {
    case allow
    case deny
}

/// `GET /v1/agents`.
public struct AgentsPage: Codable, Equatable, Sendable {
    public var agents: [Agent]

    public init(agents: [Agent]) {
        self.agents = agents
    }

    public init(from decoder: Decoder) throws {
        agents = try decoder.container(keyedBy: CodingKeys.self).list(Agent.self, .agents)
    }
}

/// `GET /v1/approvals`.
public struct ApprovalsPage: Codable, Equatable, Sendable {
    public var approvals: [Approval]

    public init(approvals: [Approval]) {
        self.approvals = approvals
    }

    public init(from decoder: Decoder) throws {
        approvals = try decoder.container(keyedBy: CodingKeys.self).list(Approval.self, .approvals)
    }
}

/// `GET /v1/agents/{id}/screen`: the current screen as plain text.
public struct AgentScreen: Codable, Equatable, Sendable {
    public var rows: Int
    public var cols: Int
    public var text: String
    /// `[row, col]`.
    public var cursor: [Int]

    public init(rows: Int, cols: Int, text: String, cursor: [Int] = [0, 0]) {
        self.rows = rows
        self.cols = cols
        self.text = text
        self.cursor = cursor
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        rows = c.int(.rows) ?? 0
        cols = c.int(.cols) ?? 0
        text = c.string(.text) ?? ""
        cursor = c.list(Int.self, .cursor)
    }
}

extension EventKind {
    public static let agentStarted = "agent.started"
    public static let agentStateChanged = "agent.state_changed"
    public static let agentExited = "agent.exited"
    public static let agentRemoved = "agent.removed"
    public static let approvalRequested = "approval.requested"
    public static let approvalDecided = "approval.decided"
}

extension LoggedEvent {
    /// Whether this event may have changed the agent list. An approval
    /// changes its agent's state and `pending_approval_id`; a removed
    /// workspace takes its agents with it.
    public var changesAgents: Bool {
        kind.hasPrefix("agent.") || kind.hasPrefix("approval.") || kind == EventKind.workspaceRemoved
    }

    public var changesApprovals: Bool {
        kind.hasPrefix("approval.") || kind == EventKind.agentExited || kind == EventKind.agentRemoved
    }
}

extension Date {
    public init(unixMilliseconds ms: Int64) {
        self.init(timeIntervalSince1970: Double(ms) / 1000)
    }
}

extension Optional where Wrapped == String {
    /// `nil` for nil, empty or blank strings; otherwise the trimmed text.
    var nonEmpty: String? {
        guard let text = self?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        return text
    }
}
