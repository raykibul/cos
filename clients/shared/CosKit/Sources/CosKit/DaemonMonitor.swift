import Foundation
import Observation

/// Whether the app can see the daemon right now.
public enum ConnectionState: Equatable, Sendable {
    case connecting
    case running
    /// Nothing answers on the socket. Checking again automatically.
    case notRunning
    /// Some other problem, such as a bad answer. Retrying automatically.
    case failed(String)
}

/// The daemon's state for a UI: connection, status, recent events and
/// projects, kept up to date from the event stream for as long as it is
/// started. Both apps drive their views from this.
@MainActor
@Observable
public final class DaemonMonitor {
    public private(set) var connection: ConnectionState = .connecting
    /// The latest status, or `nil` while the daemon isn't reachable.
    public private(set) var status: Status?
    /// The most recent events, oldest first.
    public private(set) var events: [LoggedEvent] = []
    /// Registered projects with their workspaces, in the order they were added.
    /// Kept while the daemon is away, so the last known list stays visible.
    public private(set) var projects: [Project] = []
    /// Why the last project refresh failed, if it did.
    public private(set) var projectsError: String?
    /// Agents in every workspace, newest first. Kept while the daemon is away.
    public private(set) var agents: [Agent] = []
    /// Approvals waiting for an answer, oldest first.
    public private(set) var approvals: [Approval] = []
    /// False when the daemon answers 404 for the agents routes: an older
    /// daemon that doesn't run agents yet.
    public private(set) var agentsSupported = true
    /// Why the last agent or approval refresh failed, if it did.
    public private(set) var agentsError: String?

    public let client: CosClient
    /// How many events to keep in memory.
    public let keep: Int
    private var task: Task<Void, Never>?
    private var projectsRefresh: CoalescedRefresh<[Project]>!
    private var agentsRefresh: CoalescedRefresh<[Agent]>!
    private var approvalsRefresh: CoalescedRefresh<[Approval]>!

    public init(client: CosClient = CosClient(), keep: Int = 1000) {
        self.client = client
        self.keep = keep
        projectsRefresh = CoalescedRefresh(fetch: { try await client.projects() }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let projects):
                self.projects = projects
                projectsError = nil
            case .failure(let error):
                projectsError = error.localizedDescription
            }
        }
        agentsRefresh = CoalescedRefresh(fetch: { try await client.agents() }) { [weak self] result in
            self?.applyAgentsResult(result) { $0.agents = $1 }
        }
        approvalsRefresh = CoalescedRefresh(fetch: { try await client.approvals(state: .pending) }) { [weak self] result in
            self?.applyAgentsResult(result) { monitor, approvals in
                monitor.approvals = approvals.filter { $0.state == .pending }.sorted { $0.createdMs < $1.createdMs }
            }
        }
    }

    /// Applies an agents or approvals answer. A 404 means the daemon has no
    /// agents API, which isn't an error worth showing.
    private func applyAgentsResult<T>(_ result: Result<T, Error>, _ store: (DaemonMonitor, T) -> Void) {
        switch result {
        case .success(let value):
            store(self, value)
            agentsSupported = true
            agentsError = nil
        case .failure(CosError.notFound):
            agentsSupported = false
            agents = []
            approvals = []
            agentsError = nil
        case .failure(let error):
            agentsError = error.localizedDescription
        }
    }

    public func start() {
        guard task == nil else { return }
        let follower = EventFollower(source: client, backlog: min(keep, 200))
        task = Task { [weak self] in
            for await update in follower.updates() {
                self?.apply(update)
            }
        }
    }

    public func stop() {
        task?.cancel()
        task = nil
    }

    func apply(_ update: FollowUpdate) {
        switch update {
        case .connected(let status):
            self.status = status
            connection = .running
            refreshProjects()
            refreshAgents()
        case .event(let event):
            if event.changesProjects { refreshProjects() }
            if event.changesAgents { agentsRefresh.request() }
            if event.changesApprovals { approvalsRefresh.request() }
            events.append(event)
            if events.count > keep { events.removeFirst(events.count - keep) }
            // Events only ever append, so keep the counts current without
            // asking for the status again.
            if var status, event.seq > status.lastSeq {
                status.events += 1
                status.lastSeq = event.seq
                self.status = status
            }
        case .logReset:
            events = []
        case .disconnected(let error, _):
            status = nil
            if case .notRunning = error {
                connection = .notRunning
            } else if case .connectionClosed = error {
                // Usually the daemon stopping; the retry will tell.
                connection = .connecting
            } else {
                connection = .failed(error.localizedDescription)
            }
        }
    }

    // MARK: Projects

    /// Reloads the project list. Calls made while one is in flight are folded
    /// into a single follow-up, so a burst of events costs two requests.
    public func refreshProjects() {
        projectsRefresh.request()
    }

    /// Reloads agents and pending approvals, folded the same way.
    public func refreshAgents() {
        agentsRefresh.request()
        approvalsRefresh.request()
    }

    // The actions below apply the daemon's answer at once, so the UI doesn't
    // wait for the event, then refresh to pick up anything else. They throw
    // `CosError`s whose descriptions can be shown as-is.

    @discardableResult
    public func addProject(path: String) async throws -> Project {
        let project = try await client.addProject(path: path)
        if let index = projects.firstIndex(where: { $0.id == project.id }) {
            projects[index] = project
        } else {
            projects.append(project)
        }
        refreshProjects()
        return project
    }

    public func removeProject(id: Int) async throws {
        try await client.removeProject(id: id)
        projects.removeAll { $0.id == id }
        refreshProjects()
    }

    @discardableResult
    public func createWorkspace(projectId: Int, base: WorkspaceBase, title: String?) async throws -> Workspace {
        let workspace = try await client.createWorkspace(projectId: projectId, base: base, title: title)
        updateProject(projectId) { $0.workspaces.append(workspace) }
        refreshProjects()
        return workspace
    }

    @discardableResult
    public func renameWorkspace(_ workspace: Workspace, title: String) async throws -> Workspace {
        let renamed = try await client.renameWorkspace(id: workspace.id, title: title)
        updateProject(workspace.projectId) { project in
            if let index = project.workspaces.firstIndex(where: { $0.id == renamed.id }) {
                project.workspaces[index] = renamed
            }
        }
        refreshProjects()
        return renamed
    }

    public func removeWorkspace(_ workspace: Workspace) async throws {
        try await client.removeWorkspace(id: workspace.id)
        updateProject(workspace.projectId) { $0.workspaces.removeAll { $0.id == workspace.id } }
        refreshProjects()
    }

    public func project(id: Int) -> Project? {
        projects.first { $0.id == id }
    }

    public func workspace(id: Int) -> Workspace? {
        projects.lazy.flatMap(\.workspaces).first { $0.id == id }
    }

    private func updateProject(_ id: Int, _ change: (inout Project) -> Void) {
        if let index = projects.firstIndex(where: { $0.id == id }) {
            change(&projects[index])
        }
    }

    // MARK: Agents

    public func agent(id: Int) -> Agent? {
        agents.first { $0.id == id }
    }

    /// A workspace's agents, newest first.
    public func agents(inWorkspace id: Int) -> [Agent] {
        agents.filter { $0.workspaceId == id }
    }

    /// The approval an agent is waiting on, if any.
    public func pendingApproval(forAgent id: Int) -> Approval? {
        approvals.first { $0.agentId == id && $0.state == .pending }
    }

    /// Agents that can't go on until you answer something: a permission
    /// request or the trust dialog.
    public var agentsWaitingOnYou: [Agent] {
        agents.filter { $0.state == .needsTrust || pendingApproval(forAgent: $0.id) != nil || $0.state == .waitingApproval }
    }

    /// Pending approvals plus agents stuck on the trust dialog.
    public var waitingCount: Int {
        approvals.count + agents.filter { $0.state == .needsTrust }.count
    }

    @discardableResult
    public func createAgent(workspaceId: Int, _ request: NewAgent) async throws -> Agent {
        let agent = try await client.createAgent(workspaceId: workspaceId, request)
        upsert(agent)
        agentsRefresh.request()
        return agent
    }

    /// Sends `/exit`. The agent's state follows through events.
    public func stopAgent(id: Int) async throws {
        try await client.stopAgent(id: id)
        agentsRefresh.request()
    }

    @discardableResult
    public func resumeAgent(id: Int) async throws -> Agent {
        let agent = try await client.resumeAgent(id: id)
        upsert(agent)
        agentsRefresh.request()
        return agent
    }

    public func removeAgent(id: Int) async throws {
        try await client.removeAgent(id: id)
        agents.removeAll { $0.id == id }
        approvals.removeAll { $0.agentId == id }
        agentsRefresh.request()
    }

    public func trustAgent(id: Int) async throws {
        try await client.trustAgent(id: id)
        agentsRefresh.request()
    }

    /// Allows or denies. If it was already answered in the terminal (409),
    /// it is dropped from the list anyway and the error is rethrown.
    @discardableResult
    public func decide(_ approval: Approval, _ decision: ApprovalDecision, message: String? = nil) async throws -> Approval {
        defer {
            approvalsRefresh.request()
            agentsRefresh.request()
        }
        do {
            let decided = try await client.decide(approvalId: approval.id, decision, message: message)
            approvals.removeAll { $0.id == approval.id }
            return decided
        } catch CosError.conflict(let message) {
            approvals.removeAll { $0.id == approval.id }
            throw CosError.conflict(message)
        }
    }

    private func upsert(_ agent: Agent) {
        if let index = agents.firstIndex(where: { $0.id == agent.id }) {
            agents[index] = agent
        } else {
            agents.insert(agent, at: 0)
        }
    }
}

/// A reload that never runs twice at once. Requests made while one is in
/// flight are folded into a single follow-up, so a burst of events costs at
/// most two requests, and an answer that a newer request overtook is dropped.
@MainActor
final class CoalescedRefresh<Value: Sendable> {
    private let fetch: @Sendable () async throws -> Value
    private let apply: @MainActor (Result<Value, Error>) -> Void
    private var running = false
    private var again = false

    init(fetch: @escaping @Sendable () async throws -> Value, apply: @escaping @MainActor (Result<Value, Error>) -> Void) {
        self.fetch = fetch
        self.apply = apply
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
                let result: Result<Value, Error>
                do {
                    result = .success(try await fetch())
                } catch {
                    result = .failure(error)
                }
                // A newer refresh was asked for while this one ran (often
                // right after a local change); its answer may be stale.
                if !again { apply(result) }
            } while again
            running = false
        }
    }
}
