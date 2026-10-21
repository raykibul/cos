import Foundation

// Runs, tasks, the board, questions and limits, as in "Runs, tasks and the
// board" in docs/daemon.md and docs/orchestration.md. States are open
// strings, and every field the orchestration contract adds is optional, so
// both an older daemon and a newer one decode.

/// One goal in one workspace. The chief splits it into tasks.
public struct Run: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var workspaceId: Int
    public var goal: String
    public var state: RunState
    public var preset: ApprovalPreset
    public var approvalCap: Int
    /// The report, once written. May be relative to the daemon's home.
    public var reportPath: String?
    /// Why it failed or was cancelled.
    public var reason: String?
    /// The chief agent, once it has started.
    public var chiefAgentId: Int?
    public var createdAt: Date
    public var updatedAt: Date
    /// Task counts. `GET /v1/runs` sends them with each run (as `tasks`);
    /// a lone run (`POST /v1/runs`, a board's `run`) doesn't.
    public var counts: TaskCounts?

    public init(
        id: Int, workspaceId: Int, goal: String, state: RunState, preset: ApprovalPreset = .normal, approvalCap: Int = 20,
        reportPath: String? = nil, reason: String? = nil, chiefAgentId: Int? = nil, createdAt: Date, updatedAt: Date? = nil,
        counts: TaskCounts? = nil
    ) {
        self.id = id
        self.workspaceId = workspaceId
        self.goal = goal
        self.state = state
        self.preset = preset
        self.approvalCap = approvalCap
        self.reportPath = reportPath
        self.reason = reason
        self.chiefAgentId = chiefAgentId
        self.createdAt = createdAt
        self.updatedAt = updatedAt ?? createdAt
        self.counts = counts
    }

    private enum CodingKeys: String, CodingKey {
        case id, workspaceId, goal, state, preset, approvalCap, reportPath, reason, chiefAgentId, createdAt, updatedAt
        case counts = "tasks"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int.self, forKey: .id)
        workspaceId = try c.decode(Int.self, forKey: .workspaceId)
        goal = try c.decodeIfPresent(String.self, forKey: .goal) ?? ""
        state = try c.decode(RunState.self, forKey: .state)
        preset = (try? c.decodeIfPresent(ApprovalPreset.self, forKey: .preset)) ?? .normal
        approvalCap = (try? c.decodeIfPresent(Int.self, forKey: .approvalCap)) ?? 20
        reportPath = try? c.decodeIfPresent(String.self, forKey: .reportPath)
        reason = try? c.decodeIfPresent(String.self, forKey: .reason)
        chiefAgentId = try? c.decodeIfPresent(Int.self, forKey: .chiefAgentId)
        createdAt = (try? c.decodeIfPresent(Date.self, forKey: .createdAt)) ?? Date(timeIntervalSince1970: 0)
        updatedAt = (try? c.decodeIfPresent(Date.self, forKey: .updatedAt)) ?? createdAt
        // Only a summary's `tasks` is counts; anything else there is ignored.
        counts = try? c.decodeIfPresent(TaskCounts.self, forKey: .counts)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(workspaceId, forKey: .workspaceId)
        try c.encode(goal, forKey: .goal)
        try c.encode(state, forKey: .state)
        try c.encode(preset, forKey: .preset)
        try c.encode(approvalCap, forKey: .approvalCap)
        try c.encodeIfPresent(reportPath, forKey: .reportPath)
        try c.encodeIfPresent(reason, forKey: .reason)
        try c.encodeIfPresent(chiefAgentId, forKey: .chiefAgentId)
        try c.encode(createdAt, forKey: .createdAt)
        try c.encode(updatedAt, forKey: .updatedAt)
        try c.encodeIfPresent(counts, forKey: .counts)
    }

    /// Where the report is on disk: `report_path`, resolved against the
    /// daemon's home when relative.
    public func reportURL(home: String?) -> URL? {
        guard let reportPath, !reportPath.isEmpty else { return nil }
        if reportPath.hasPrefix("/") { return URL(fileURLWithPath: reportPath) }
        guard let home else { return nil }
        return URL(fileURLWithPath: home).appendingPathComponent(reportPath)
    }
}

public struct RunState: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// The chief is working out the tasks.
    public static let planning = RunState(rawValue: "planning")
    /// A plan is on the board, waiting for you.
    public static let awaitingPlanApproval = RunState(rawValue: "awaiting_plan_approval")
    /// Workers are on the tasks.
    public static let running = RunState(rawValue: "running")
    /// The chief is writing the report.
    public static let reporting = RunState(rawValue: "reporting")
    public static let done = RunState(rawValue: "done")
    public static let failed = RunState(rawValue: "failed")
    public static let cancelled = RunState(rawValue: "cancelled")

    /// `done`, `failed` or `cancelled`. Unknown states count as unfinished,
    /// so a newer daemon's run still shows as current.
    public var isFinished: Bool { self == .done || self == .failed || self == .cancelled }
}

public struct TaskState: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let proposed = TaskState(rawValue: "proposed")
    public static let ready = TaskState(rawValue: "ready")
    public static let running = TaskState(rawValue: "running")
    public static let checking = TaskState(rawValue: "checking")
    public static let review = TaskState(rawValue: "review")
    public static let merged = TaskState(rawValue: "merged")
    public static let blocked = TaskState(rawValue: "blocked")
    public static let failed = TaskState(rawValue: "failed")
    public static let cancelled = TaskState(rawValue: "cancelled")

    /// `merged`, `failed` or `cancelled`.
    public var isFinished: Bool { self == .merged || self == .failed || self == .cancelled }
    /// Someone has to decide: retry, fail or cancel.
    public var needsDecision: Bool { self == .blocked }
}

/// How much the chief may approve on your behalf.
public struct ApprovalPreset: RawRepresentable, Codable, Hashable, Sendable, Identifiable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public var id: String { rawValue }

    /// Only you approve.
    public static let strict = ApprovalPreset(rawValue: "strict")
    /// The chief approves what stays inside the task's worktree.
    public static let normal = ApprovalPreset(rawValue: "normal")
    /// The chief also approves network reads and installs into the worktree.
    public static let trusted = ApprovalPreset(rawValue: "trusted")

    public static let all: [ApprovalPreset] = [.strict, .normal, .trusted]
}

/// How many of a run's tasks are in each state.
public struct TaskCounts: Codable, Equatable, Sendable {
    public var proposed = 0
    public var ready = 0
    public var running = 0
    public var checking = 0
    public var review = 0
    public var merged = 0
    public var blocked = 0
    public var failed = 0
    public var cancelled = 0
    public var total = 0

    public init() {}

    public init(_ tasks: [RunTask]) {
        for task in tasks {
            switch task.state {
            case .proposed: proposed += 1
            case .ready: ready += 1
            case .running: running += 1
            case .checking: checking += 1
            case .review: review += 1
            case .merged: merged += 1
            case .blocked: blocked += 1
            case .failed: failed += 1
            case .cancelled: cancelled += 1
            default: break
            }
        }
        total = tasks.count
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func count(_ key: CodingKeys) -> Int { (try? c.decodeIfPresent(Int.self, forKey: key)) ?? 0 }
        proposed = count(.proposed)
        ready = count(.ready)
        running = count(.running)
        checking = count(.checking)
        review = count(.review)
        merged = count(.merged)
        blocked = count(.blocked)
        failed = count(.failed)
        cancelled = count(.cancelled)
        let sum = proposed + ready + running + checking + review + merged + blocked + failed + cancelled
        total = (try? c.decodeIfPresent(Int.self, forKey: .total)) ?? sum
    }

    /// Tasks that won't move again: merged, failed or cancelled.
    public var finished: Int { merged + failed + cancelled }
    /// Tasks someone is on: running, being checked or reviewed.
    public var active: Int { running + checking + review }
    /// 0…1, finished tasks over all of them.
    public var progress: Double { total == 0 ? 0 : Double(finished) / Double(total) }
}

/// One task of a run: one worker in its own short-lived worktree.
public struct RunTask: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var runId: Int
    /// Its place in the plan, from 0.
    public var position: Int
    public var title: String
    public var instructions: String
    /// Paths or globs relative to the repo: the task's scope.
    public var files: [String]
    public var testCommand: String?
    /// Ids of tasks that must merge first.
    public var dependsOn: [Int]
    public var state: TaskState
    public var retries: Int
    public var branch: String?
    public var worktree: String?
    /// Why it was last blocked, failed or retried.
    public var lastFailure: String?
    /// The current worker agent.
    public var agentId: Int?
    public var checks: TaskChecks?
    public var review: TaskReview?
    public var createdAt: Date?
    public var updatedAt: Date?

    public init(
        id: Int, runId: Int, position: Int, title: String, instructions: String = "", files: [String] = [],
        testCommand: String? = nil, dependsOn: [Int] = [], state: TaskState, retries: Int = 0, branch: String? = nil,
        worktree: String? = nil, lastFailure: String? = nil, agentId: Int? = nil, checks: TaskChecks? = nil,
        review: TaskReview? = nil, createdAt: Date? = nil, updatedAt: Date? = nil
    ) {
        self.id = id
        self.runId = runId
        self.position = position
        self.title = title
        self.instructions = instructions
        self.files = files
        self.testCommand = testCommand
        self.dependsOn = dependsOn
        self.state = state
        self.retries = retries
        self.branch = branch
        self.worktree = worktree
        self.lastFailure = lastFailure
        self.agentId = agentId
        self.checks = checks
        self.review = review
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    private enum CodingKeys: String, CodingKey {
        case id, runId, position, title, instructions, description, files, testCommand, dependsOn, state, retries
        case branch, worktree, lastFailure, agentId, checks, review, createdAt, updatedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int.self, forKey: .id)
        runId = (try? c.decodeIfPresent(Int.self, forKey: .runId)) ?? 0
        position = (try? c.decodeIfPresent(Int.self, forKey: .position)) ?? 0
        title = try c.decode(String.self, forKey: .title)
        instructions = (try? c.decodeIfPresent(String.self, forKey: .instructions))
            ?? (try? c.decodeIfPresent(String.self, forKey: .description)) ?? ""
        files = (try? c.decodeIfPresent([String].self, forKey: .files)) ?? []
        testCommand = try? c.decodeIfPresent(String.self, forKey: .testCommand)
        dependsOn = (try? c.decodeIfPresent([Int].self, forKey: .dependsOn)) ?? []
        state = try c.decode(TaskState.self, forKey: .state)
        retries = (try? c.decodeIfPresent(Int.self, forKey: .retries)) ?? 0
        branch = try? c.decodeIfPresent(String.self, forKey: .branch)
        worktree = try? c.decodeIfPresent(String.self, forKey: .worktree)
        lastFailure = try? c.decodeIfPresent(String.self, forKey: .lastFailure)
        agentId = try? c.decodeIfPresent(Int.self, forKey: .agentId)
        checks = try? c.decodeIfPresent(TaskChecks.self, forKey: .checks)
        review = try? c.decodeIfPresent(TaskReview.self, forKey: .review)
        createdAt = try? c.decodeIfPresent(Date.self, forKey: .createdAt)
        updatedAt = try? c.decodeIfPresent(Date.self, forKey: .updatedAt)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(runId, forKey: .runId)
        try c.encode(position, forKey: .position)
        try c.encode(title, forKey: .title)
        try c.encode(instructions, forKey: .instructions)
        try c.encode(files, forKey: .files)
        try c.encodeIfPresent(testCommand, forKey: .testCommand)
        try c.encode(dependsOn, forKey: .dependsOn)
        try c.encode(state, forKey: .state)
        try c.encode(retries, forKey: .retries)
        try c.encodeIfPresent(branch, forKey: .branch)
        try c.encodeIfPresent(worktree, forKey: .worktree)
        try c.encodeIfPresent(lastFailure, forKey: .lastFailure)
        try c.encodeIfPresent(agentId, forKey: .agentId)
        try c.encodeIfPresent(checks, forKey: .checks)
        try c.encodeIfPresent(review, forKey: .review)
        try c.encodeIfPresent(createdAt, forKey: .createdAt)
        try c.encodeIfPresent(updatedAt, forKey: .updatedAt)
    }

    /// Each task gets one retry.
    public var canRetry: Bool { state == .blocked && retries < 1 }
    /// Fail applies to a stuck task; the pipeline fails the others itself.
    public var canFail: Bool { state == .blocked }
    public var canCancel: Bool { !state.isFinished }
}

/// What the pipeline checked after the worker said it was done.
public struct TaskChecks: Codable, Equatable, Sendable {
    public var scope: ScopeCheck?
    public var tests: TestCheck?

    public init(scope: ScopeCheck? = nil, tests: TestCheck? = nil) {
        self.scope = scope
        self.tests = tests
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        scope = try? c.decodeIfPresent(ScopeCheck.self, forKey: .scope)
        tests = try? c.decodeIfPresent(TestCheck.self, forKey: .tests)
    }
}

/// Whether the worker's commits stayed inside the task's files.
public struct ScopeCheck: Codable, Equatable, Sendable {
    public var ok: Bool
    /// Changed paths outside the task's files.
    public var outOfScope: [String]
    /// Changed paths cos never merges (`.git/`, `.claude/`, `.env*`…).
    public var protected: [String]
    /// Changes the worker left uncommitted.
    public var uncommitted: [String]

    public init(ok: Bool, outOfScope: [String] = [], protected: [String] = [], uncommitted: [String] = []) {
        self.ok = ok
        self.outOfScope = outOfScope
        self.protected = protected
        self.uncommitted = uncommitted
    }

    enum CodingKeys: String, CodingKey {
        case ok, outOfScope, protected, uncommitted
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        outOfScope = Self.paths(c, .outOfScope)
        protected = Self.paths(c, .protected)
        uncommitted = Self.paths(c, .uncommitted)
        ok = (try? c.decodeIfPresent(Bool.self, forKey: .ok)) ?? (outOfScope.isEmpty && protected.isEmpty && uncommitted.isEmpty)
    }

    /// A list of paths; a bare `true` (some daemon might just flag it) becomes "(yes)".
    private static func paths(_ c: KeyedDecodingContainer<CodingKeys>, _ key: CodingKeys) -> [String] {
        if let list = try? c.decodeIfPresent([String].self, forKey: key) { return list }
        if let flag = try? c.decodeIfPresent(Bool.self, forKey: key), flag { return ["(yes)"] }
        return []
    }

    /// Every problem, for one line of text.
    public var problems: [String] {
        var out: [String] = []
        if !outOfScope.isEmpty { out.append("outside its files: " + outOfScope.joined(separator: ", ")) }
        if !protected.isEmpty { out.append("protected paths: " + protected.joined(separator: ", ")) }
        if !uncommitted.isEmpty { out.append("uncommitted: " + uncommitted.joined(separator: ", ")) }
        return out
    }
}

/// The task's test command, run in its worktree.
public struct TestCheck: Codable, Equatable, Sendable {
    /// `passed`, `failed`, `timed_out`, or something newer.
    public var status: String
    public var exitCode: Int?
    /// The end of the output.
    public var outputTail: String?

    public init(status: String, exitCode: Int? = nil, outputTail: String? = nil) {
        self.status = status
        self.exitCode = exitCode
        self.outputTail = outputTail
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        status = (try? c.decodeIfPresent(String.self, forKey: .status)) ?? "unknown"
        exitCode = try? c.decodeIfPresent(Int.self, forKey: .exitCode)
        outputTail = try? c.decodeIfPresent(String.self, forKey: .outputTail)
    }

    public var passed: Bool { status == "passed" || status == "ok" }
    /// The task has no test command, so nothing ran.
    public var skipped: Bool { status == "skipped" }
}

/// The reviewer's verdict.
public struct TaskReview: Codable, Equatable, Sendable {
    /// `approve` or `changes`.
    public var verdict: String
    public var notes: String?

    public init(verdict: String, notes: String? = nil) {
        self.verdict = verdict
        self.notes = notes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        verdict = (try? c.decodeIfPresent(String.self, forKey: .verdict)) ?? "unknown"
        notes = try? c.decodeIfPresent(String.self, forKey: .notes)
    }

    public var approved: Bool { verdict == "approve" || verdict == "approved" }
}

/// `GET /v1/runs/{id}`: a run with its tasks, in plan order.
public struct Board: Codable, Equatable, Sendable {
    public var run: Run
    /// `nil` once the workspace is removed.
    public var workspace: Workspace?
    public var tasks: [RunTask]
    public var counts: TaskCounts

    public init(run: Run, workspace: Workspace? = nil, tasks: [RunTask], counts: TaskCounts? = nil) {
        self.run = run
        self.workspace = workspace
        self.tasks = tasks
        self.counts = counts ?? TaskCounts(tasks)
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        run = try c.decode(Run.self, forKey: .run)
        workspace = try? c.decodeIfPresent(Workspace.self, forKey: .workspace)
        tasks = (try c.decodeIfPresent([RunTask].self, forKey: .tasks) ?? []).sorted { $0.position < $1.position }
        counts = (try? c.decodeIfPresent(TaskCounts.self, forKey: .counts)) ?? TaskCounts(tasks)
    }

    public func task(id: Int) -> RunTask? {
        tasks.first { $0.id == id }
    }
}

/// `POST /v1/runs`.
public struct NewRun: Encodable, Equatable, Sendable {
    public var workspaceId: Int
    public var goal: String
    public var preset: ApprovalPreset?
    public var approvalCap: Int?

    public init(workspaceId: Int, goal: String, preset: ApprovalPreset? = nil, approvalCap: Int? = nil) {
        self.workspaceId = workspaceId
        self.goal = goal.trimmingCharacters(in: .whitespacesAndNewlines)
        self.preset = preset
        self.approvalCap = approvalCap
    }

    private enum CodingKeys: String, CodingKey {
        case goal, preset
        case workspaceId = "workspace_id"
        case approvalCap = "approval_cap"
    }
}

/// `GET /v1/runs`.
public struct RunsPage: Codable, Equatable, Sendable {
    public var runs: [Run]
}

/// Something the chief asked you (`ask_user`).
public struct Question: Codable, Equatable, Sendable, Identifiable {
    public var id: Int
    public var runId: Int?
    public var agentId: Int?
    public var question: String
    /// Suggested answers. You may still answer in your own words.
    public var choices: [String]
    public var state: QuestionState
    public var answer: String?
    public var createdMs: Int64
    public var answeredMs: Int64?

    public init(
        id: Int, runId: Int? = nil, agentId: Int? = nil, question: String, choices: [String] = [],
        state: QuestionState = .open, answer: String? = nil, createdMs: Int64, answeredMs: Int64? = nil
    ) {
        self.id = id
        self.runId = runId
        self.agentId = agentId
        self.question = question
        self.choices = choices
        self.state = state
        self.answer = answer
        self.createdMs = createdMs
        self.answeredMs = answeredMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int.self, forKey: .id)
        runId = try? c.decodeIfPresent(Int.self, forKey: .runId)
        agentId = try? c.decodeIfPresent(Int.self, forKey: .agentId)
        question = try c.decode(String.self, forKey: .question)
        choices = (try? c.decodeIfPresent([String].self, forKey: .choices)) ?? []
        state = (try? c.decodeIfPresent(QuestionState.self, forKey: .state)) ?? .open
        answer = try? c.decodeIfPresent(String.self, forKey: .answer)
        createdMs = (try? c.decodeIfPresent(Int64.self, forKey: .createdMs)) ?? 0
        answeredMs = try? c.decodeIfPresent(Int64.self, forKey: .answeredMs)
    }

    public var createdAt: Date { Date(unixMilliseconds: createdMs) }
}

public struct QuestionState: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let open = QuestionState(rawValue: "open")
    public static let answered = QuestionState(rawValue: "answered")
}

/// `GET /v1/questions`.
public struct QuestionsPage: Codable, Equatable, Sendable {
    public var questions: [Question]
}

/// `GET /v1/runs/{id}/limits`: how much the chief may approve, and how much it has.
public struct RunLimits: Codable, Equatable, Sendable {
    public var preset: ApprovalPreset
    public var approvalCap: Int
    /// Approvals the chief has given in this run, counted against the cap.
    public var chiefApprovals: Int

    public init(preset: ApprovalPreset, approvalCap: Int, chiefApprovals: Int = 0) {
        self.preset = preset
        self.approvalCap = approvalCap
        self.chiefApprovals = chiefApprovals
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        preset = (try? c.decodeIfPresent(ApprovalPreset.self, forKey: .preset)) ?? .normal
        approvalCap = (try? c.decodeIfPresent(Int.self, forKey: .approvalCap)) ?? 20
        chiefApprovals = (try? c.decodeIfPresent(Int.self, forKey: .chiefApprovals)) ?? 0
    }

    /// Once reached, everything goes to you.
    public var capReached: Bool { chiefApprovals >= approvalCap }
}

/// `GET /v1/runs/{id}/report`.
struct RunReport: Codable, Sendable {
    var markdown: String
}

// MARK: Approvals, as the orchestration contract extends them

/// Which tier `cos-policy` put a request in.
public struct ApprovalTier: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let allowed = ApprovalTier(rawValue: "allowed")
    public static let chief = ApprovalTier(rawValue: "chief")
    public static let askUser = ApprovalTier(rawValue: "ask_user")
    public static let neverDelegated = ApprovalTier(rawValue: "never_delegated")
}

/// Who answers a request.
public struct ApprovalRoute: RawRepresentable, Codable, Hashable, Sendable {
    public var rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    /// Allowed by a rule straight away; nobody is asked.
    public static let auto = ApprovalRoute(rawValue: "auto")
    /// The run's chief decides.
    public static let chief = ApprovalRoute(rawValue: "chief")
    /// You decide.
    public static let user = ApprovalRoute(rawValue: "user")
}

extension Approval {
    /// Whether this one is yours to answer: pending and not routed to the
    /// chief. Approvals without a route (old rows, older daemons) are yours.
    public var isForYou: Bool {
        state == .pending && route != .chief
    }

    /// Pending and routed to the run's chief, which is deciding.
    public var isWithChief: Bool {
        state == .pending && route == .chief
    }
}

// MARK: Events

extension EventKind {
    public static let runCreated = "run.created"
    public static let runStateChanged = "run.state_changed"
    public static let runReportWritten = "run.report_written"
    public static let runLimitsChanged = "run.limits_changed"
    public static let planProposed = "plan.proposed"
    public static let planApproved = "plan.approved"
    public static let taskStateChanged = "task.state_changed"
    public static let taskWorktreeAssigned = "task.worktree_assigned"
    public static let taskChecked = "task.checked"
    public static let taskReviewed = "task.reviewed"
    public static let taskMerged = "task.merged"
    public static let approvalRouted = "approval.routed"
    public static let questionAsked = "question.asked"
    public static let questionAnswered = "question.answered"
}

extension LoggedEvent {
    /// Whether this event may have changed a run or its board. By prefix,
    /// so kinds added later still trigger a refresh.
    public var changesRuns: Bool {
        kind.hasPrefix("run.") || kind.hasPrefix("plan.") || kind.hasPrefix("task.") || kind == EventKind.workspaceRemoved
    }

    public var changesQuestions: Bool {
        kind.hasPrefix("question.") || kind == EventKind.runStateChanged
    }

    /// The run it belongs to: the log's `run_id`, else `data.run_id`.
    public var runNumber: Int? {
        if let runId, let number = Int(runId) { return number }
        switch data["run_id"] {
        case .number(let value)?: return Int(value)
        case .string(let text)?: return Int(text)
        default: return nil
        }
    }
}

extension Agent {
    /// The run that started it, as a number.
    public var runNumber: Int? { runId.flatMap { Int($0) } }
}
