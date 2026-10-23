import CosKit
import Foundation
import Observation

/// Whether the phone can see the host's daemon right now.
enum Connection: Equatable, Sendable {
    /// Connecting, or reconnecting after the stream dropped.
    case connecting
    case live
    /// The daemon answered, but refused this device (`device_pending`, …).
    /// The follower keeps asking, so an approval on the host lifts it.
    case refused(Refusal)
    /// Nothing answers: the name doesn't resolve, the port is closed, or
    /// Tailscale is off. Retried automatically.
    case unreachable(String)
    case failed(String)

    var isLive: Bool { self == .live }
}

/// One host's state for the app: its projects, agents, approvals, runs,
/// questions and Chiefs of Staff, kept current from the event stream while
/// the app is in the foreground. Every screen reads from it.
@MainActor
@Observable
final class HostSession {
    let host: Host
    @ObservationIgnored let service: any CosService

    private(set) var connection: Connection = .connecting
    private(set) var status: Status?
    /// The lists have loaded at least once, so screens can show them even
    /// while reconnecting.
    private(set) var hasLoaded = false

    private(set) var projects: [Project] = []
    /// Newest first.
    private(set) var agents: [Agent] = []
    /// Pending approvals, oldest first.
    private(set) var approvals: [Approval] = []
    /// Open questions from run chiefs, oldest first.
    private(set) var questions: [Question] = []
    /// Newest first.
    private(set) var runs: [Run] = []
    /// Each project's Chief of Staff, by project id.
    private(set) var staffs: [Int: Staff] = [:]

    /// Bumped per agent when its transcript grows (`agent.transcript`), so
    /// a chat on screen reads what's new.
    private(set) var transcriptTicks: [Int: Int] = [:]
    /// Bumped whenever a run, its board or its report may have changed.
    private(set) var runTick = 0

    /// When you last opened each chief's chat, for the chiefs list.
    let readMarks: ReadMarks

    @ObservationIgnored private let notifications: any Notifications
    @ObservationIgnored private let backoff: Backoff
    @ObservationIgnored private var follow: Task<Void, Never>?
    @ObservationIgnored private var lastSeq: Int?
    @ObservationIgnored private var refreshers: [Refresh: Coalescer] = [:]
    @ObservationIgnored private var loaded: Set<Refresh> = []
    /// What already waited on you when the lists first loaded, and what was
    /// reported since; `nil` until then.
    @ObservationIgnored private var seenWaiting: Set<String>?

    private enum Refresh: CaseIterable {
        case projects, agents, approvals, runs, questions, staffs
    }

    init(
        host: Host, service: any CosService, notifications: any Notifications = NoNotifications(), backoff: Backoff = Backoff(),
        readMarks: ReadMarks? = nil
    ) {
        self.host = host
        self.service = service
        self.readMarks = readMarks ?? ReadMarks(hostId: host.id)
        self.notifications = notifications
        self.backoff = backoff
        for kind in Refresh.allCases {
            refreshers[kind] = Coalescer { [weak self] in await self?.load(kind) }
        }
    }

    // MARK: Following

    /// Follows the host's events: connects, loads everything, and refreshes
    /// what each event touches. Reconnects by itself, resuming after the
    /// last event, with backoff (0.5 s doubling to 5 s), which also retries
    /// a refused device every few seconds.
    func start() {
        guard follow == nil else { return }
        let follower = EventFollower(source: service, backlog: 0, backoff: backoff)
        let after = lastSeq
        follow = Task { [weak self] in
            for await update in follower.updates(after: after) {
                guard let self else { return }
                self.apply(update)
            }
        }
    }

    /// Stops following, as the app leaves the foreground. What's loaded stays.
    func stop() {
        follow?.cancel()
        follow = nil
    }

    /// Drops the current attempt and connects again now.
    func reconnect() {
        stop()
        if case .live = connection {} else { connection = .connecting }
        start()
    }

    var isFollowing: Bool { follow != nil }

    func apply(_ update: FollowUpdate) {
        switch update {
        case .connected(let status):
            self.status = status
            connection = .live
            if lastSeq == nil { lastSeq = status.lastSeq }
            requestAll()
        case .event(let event):
            lastSeq = max(lastSeq ?? 0, event.seq)
            route(event)
        case .logReset:
            lastSeq = nil
            requestAll()
        case .disconnected(let error, _):
            switch error {
            case .refused(let refusal):
                connection = .refused(refusal)
            case .unreachable, .notRunning:
                connection = .unreachable(error.localizedDescription)
            case .connectionClosed:
                connection = .connecting
            default:
                connection = .failed(error.localizedDescription)
            }
        }
    }

    private func route(_ event: LoggedEvent) {
        if event.changesProjects { request(.projects) }
        if event.changesAgents { request(.agents) }
        if event.changesApprovals { request(.approvals) }
        if event.changesRuns {
            request(.runs)
            runTick += 1
        }
        if event.changesQuestions { request(.questions) }
        if event.changesStaff || event.changesProjects { request(.staffs) }
        if event.kind == EventKind.agentTranscript, let agentId = event.number("agent_id") {
            transcriptTicks[agentId, default: 0] += 1
        }
        if let status, event.seq > status.lastSeq {
            self.status?.lastSeq = event.seq
            self.status?.events += 1
        }
    }

    private func requestAll() {
        Refresh.allCases.forEach(request)
        runTick += 1
    }

    private func request(_ kind: Refresh) {
        refreshers[kind]?.request()
    }

    /// Reloads everything and waits for it, for pull to refresh.
    func refresh() async {
        await load(.projects)
        await withTaskGroup(of: Void.self) { group in
            for kind in Refresh.allCases where kind != .projects {
                group.addTask { await self.load(kind) }
            }
        }
        runTick += 1
    }

    private func load(_ kind: Refresh) async {
        do {
            switch kind {
            case .projects:
                projects = try await service.projects()
                // Each project has a Chief of Staff to look up.
                if Set(projects.map(\.id)) != Set(staffs.keys) { request(.staffs) }
            case .agents:
                agents = try await service.agents(workspaceId: nil, state: nil)
            case .approvals:
                approvals = try await service.approvals(state: .pending)
                    .filter { $0.state == .pending }
                    .sorted { $0.createdMs < $1.createdMs }
            case .runs:
                runs = try await service.runs()
            case .questions:
                questions = try await service.questions(state: .open)
                    .filter { $0.state == .open }
                    .sorted { $0.createdMs < $1.createdMs }
            case .staffs:
                var loaded: [Int: Staff] = [:]
                for project in projects {
                    if let staff = try? await service.staff(projectId: project.id) { loaded[project.id] = staff }
                }
                staffs = loaded
            }
            loaded.insert(kind)
            if !hasLoaded, loaded.isSuperset(of: [.projects, .agents, .approvals, .runs, .questions]) {
                hasLoaded = true
            }
            noticeWaiting()
        } catch {
            // The follower sees the same trouble and shows it; a 404 is an
            // older daemon without that route, which just leaves it empty.
        }
    }

    // MARK: Waiting on you

    /// Everything blocked on you, in the order the home tab lists it.
    var waiting: [WaitingItem] {
        var items: [WaitingItem] = approvals.filter(\.isForYou).map { .approval($0) }
        items += questions.map { .question($0) }
        items += runs.filter { $0.state == .awaitingPlanApproval }.map { .plan($0) }
        items += agents.filter { $0.state == .needsTrust }.map { .trust($0) }
        items += agents.filter { $0.state == .needsLogin }.map { .login($0) }
        return items
    }

    var waitingCount: Int { waiting.count }

    private func noticeWaiting() {
        guard hasLoaded else { return }
        let items = waiting
        let ids = Set(items.map(\.id))
        guard let seen = seenWaiting else {
            seenWaiting = ids
            Task { await notifications.setBadge(ids.count) }
            return
        }
        let fresh = items.filter { !seen.contains($0.id) }
        seenWaiting = seen.union(ids)
        let notes = fresh.map { notification(for: $0) }
        let count = ids.count
        Task {
            for note in notes { await notifications.notify(note) }
            await notifications.setBadge(count)
        }
    }

    private func notification(for item: WaitingItem) -> CosNotification {
        let thread = host.id.uuidString
        switch item {
        case .approval(let approval):
            let who = agent(id: approval.agentId)?.title ?? "An agent"
            return CosNotification(kind: .approval, title: "\(who) needs approval", body: approval.detail, thread: thread)
        case .question(let question):
            return CosNotification(kind: .question, title: "The chief asks", body: question.question, thread: thread)
        case .plan(let run):
            return CosNotification(kind: .plan, title: "A plan is ready to review", body: run.goal, thread: thread)
        case .trust(let agent):
            return CosNotification(kind: .trust, title: "\(agent.title) asks to trust its folder", body: workspaceName(agent.workspaceId), thread: thread)
        case .login(let agent):
            return CosNotification(kind: .login, title: "\(agent.title) needs a Claude login", body: workspaceName(agent.workspaceId), thread: thread)
        }
    }

    // MARK: Lookups

    func project(id: Int) -> Project? { projects.first { $0.id == id } }

    func workspace(id: Int?) -> Workspace? {
        guard let id else { return nil }
        return projects.lazy.flatMap(\.workspaces).first { $0.id == id }
    }

    func project(ofWorkspace id: Int?) -> Project? {
        projects.first { $0.workspaces.contains { $0.id == id } }
    }

    func workspaceName(_ id: Int?) -> String {
        workspace(id: id)?.displayName ?? (id.map { "Workspace \($0)" } ?? "")
    }

    func agent(id: Int?) -> Agent? {
        guard let id else { return nil }
        return agents.first { $0.id == id }
    }

    func run(id: Int) -> Run? { runs.first { $0.id == id } }

    /// A workspace's agents, newest first, without its runs' workers.
    func agents(inWorkspace id: Int) -> [Agent] {
        agents.filter { $0.workspaceId == id && $0.role != .staff }
    }

    func runs(inWorkspace id: Int) -> [Run] {
        runs.filter { $0.workspaceId == id }
    }

    func pendingApproval(forAgent id: Int) -> Approval? {
        approvals.first { $0.agentId == id && $0.state == .pending }
    }

    /// The project a Chief of Staff agent belongs to.
    func staffProject(ofAgent id: Int) -> Int? {
        staffs.first { $0.value.agentId == id }?.key
    }

    /// The agents a project's Chief of Staff started.
    func agentsStarted(byProject projectId: Int) -> [Agent] {
        guard let chief = staffs[projectId]?.agentId else { return [] }
        return agents.filter { $0.spawnedBy == chief }
    }

    // MARK: Actions
    //
    // Each applies the daemon's answer at once, so the screen doesn't wait
    // for the event, then refreshes what else it may have touched. They
    // throw `CosError`s whose descriptions can be shown as-is.

    func decide(_ approval: Approval, _ decision: ApprovalDecision) async throws {
        defer {
            request(.approvals)
            request(.agents)
        }
        do {
            _ = try await service.decide(approvalId: approval.id, decision, message: nil)
            approvals.removeAll { $0.id == approval.id }
        } catch CosError.conflict(let message) {
            // Already answered, in the terminal say: it's gone either way.
            approvals.removeAll { $0.id == approval.id }
            throw CosError.conflict(message)
        }
    }

    func answer(_ question: Question, _ text: String) async throws {
        let answer = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !answer.isEmpty else { return }
        _ = try await service.answer(questionId: question.id, answer)
        questions.removeAll { $0.id == question.id }
        request(.questions)
    }

    @discardableResult
    func approvePlan(runId: Int) async throws -> Board {
        let board = try await service.approvePlan(runId: runId)
        apply(board)
        return board
    }

    @discardableResult
    func cancelRun(id: Int) async throws -> Board {
        let board = try await service.cancelRun(id: id, reason: nil)
        apply(board)
        return board
    }

    @discardableResult
    func startRun(workspaceId: Int, goal: String) async throws -> Run {
        let run = try await service.createRun(NewRun(workspaceId: workspaceId, goal: goal))
        runs.removeAll { $0.id == run.id }
        runs.insert(run, at: 0)
        request(.runs)
        return run
    }

    func trust(agentId: Int) async throws {
        try await service.trustAgent(id: agentId)
        request(.agents)
    }

    func send(agentId: Int, text: String) async throws {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        try await service.sendInput(agentId: agentId, text: text)
        request(.agents)
    }

    @discardableResult
    func resume(agentId: Int) async throws -> Agent {
        let agent = try await service.resumeAgent(id: agentId)
        upsert(agent)
        request(.agents)
        return agent
    }

    func stopAgent(id: Int) async throws {
        try await service.stopAgent(id: id)
        request(.agents)
    }

    /// After a Chief of Staff action, its record changes.
    func refreshStaff() {
        request(.staffs)
    }

    func setStaff(_ staff: Staff) {
        staffs[staff.projectId] = staff
    }

    private func apply(_ board: Board) {
        var run = board.run
        run.counts = run.counts ?? board.counts
        if let index = runs.firstIndex(where: { $0.id == run.id }) {
            runs[index] = run
        } else {
            runs.insert(run, at: 0)
        }
        runTick += 1
        request(.runs)
        request(.questions)
    }

    private func upsert(_ agent: Agent) {
        if let index = agents.firstIndex(where: { $0.id == agent.id }) {
            agents[index] = agent
        } else {
            agents.insert(agent, at: 0)
        }
    }
}

/// One thing blocked on you.
enum WaitingItem: Identifiable, Equatable, Sendable {
    case approval(Approval)
    case question(Question)
    case plan(Run)
    case trust(Agent)
    case login(Agent)

    var id: String {
        switch self {
        case .approval(let approval): "approval-\(approval.id)"
        case .question(let question): "question-\(question.id)"
        case .plan(let run): "plan-\(run.id)"
        case .trust(let agent): "trust-\(agent.id)"
        case .login(let agent): "login-\(agent.id)"
        }
    }
}

/// A reload that never runs twice at once: requests made while one runs
/// fold into a single follow-up, so a burst of events costs two loads.
@MainActor
final class Coalescer {
    private let work: @MainActor () async -> Void
    private var running = false
    private var again = false

    init(_ work: @escaping @MainActor () async -> Void) {
        self.work = work
    }

    func request() {
        if running {
            again = true
            return
        }
        running = true
        Task {
            repeat {
                again = false
                await work()
            } while again
            running = false
        }
    }
}
