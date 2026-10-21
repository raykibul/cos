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

    public let client: CosClient
    /// How many events to keep in memory.
    public let keep: Int
    private var task: Task<Void, Never>?
    private var refreshing = false
    private var refreshAgain = false

    public init(client: CosClient = CosClient(), keep: Int = 1000) {
        self.client = client
        self.keep = keep
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
        case .event(let event):
            if event.changesProjects { refreshProjects() }
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
        if refreshing {
            refreshAgain = true
            return
        }
        refreshing = true
        Task {
            repeat {
                refreshAgain = false
                do {
                    let projects = try await client.projects()
                    // A newer refresh was asked for while this one ran (often
                    // right after a local change); its answer may be stale.
                    if !refreshAgain {
                        self.projects = projects
                        projectsError = nil
                    }
                } catch {
                    if !refreshAgain { projectsError = error.localizedDescription }
                }
            } while refreshAgain
            refreshing = false
        }
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
}
