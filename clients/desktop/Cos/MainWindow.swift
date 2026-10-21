import AppKit
import CosKit
import SwiftUI

enum SidebarItem: Hashable {
    case events
    case waiting
    case project(Int)
    case workspace(Int)
    case agent(Int)
    /// An external Claude session, by pid.
    case session(Int)
}

/// What the window is showing or asking. Shared by the sidebar's context
/// menus and the detail views, so both open the same sheets and dialogs.
@MainActor
@Observable
final class WindowState {
    var selection: SidebarItem? = .events
    var newWorkspaceIn: Project?
    var renaming: Workspace?
    var removingWorkspace: Workspace?
    var removingProject: Project?
    var newAgentIn: Workspace?
    var removingAgent: Agent?
    var alert: AlertMessage?

    /// Runs a daemon action and shows its error, which the daemon writes to
    /// be shown as-is.
    func perform(_ failureTitle: String, _ action: @escaping @MainActor () async throws -> Void) {
        Task {
            do {
                try await action()
            } catch {
                alert = AlertMessage(title: failureTitle, message: error.localizedDescription)
            }
        }
    }
}

struct AlertMessage: Identifiable {
    let id = UUID()
    var title: String
    var message: String
}

/// Projects and their workspaces in a sidebar, the selection on the right.
struct MainWindow: View {
    static let id = "main"

    let monitor: DaemonMonitor
    @Bindable var window: WindowState

    init(monitor: DaemonMonitor, window: WindowState) {
        self.monitor = monitor
        self.window = window
    }

    var body: some View {
        NavigationSplitView {
            Sidebar(monitor: monitor, window: window)
                .navigationSplitViewColumnWidth(min: 200, ideal: 250, max: 400)
        } detail: {
            VStack(spacing: 0) {
                if monitor.connection == .notRunning, window.selection != .events {
                    NotRunningBanner { window.selection = .events }
                }
                detail
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 760, minHeight: 440)
        .sheet(item: $window.newWorkspaceIn) { project in
            NewWorkspaceSheet(project: project, monitor: monitor) { workspace in
                window.selection = .workspace(workspace.id)
            }
        }
        .sheet(item: $window.renaming) { workspace in
            RenameWorkspaceSheet(workspace: workspace, monitor: monitor)
        }
        .sheet(item: $window.newAgentIn) { workspace in
            NewAgentSheet(workspace: workspace, monitor: monitor) { agent in
                window.selection = .agent(agent.id)
            }
        }
        .confirmationDialog(
            "Remove agent “\(window.removingAgent?.title ?? "")”?",
            isPresented: isPresented($window.removingAgent),
            presenting: window.removingAgent
        ) { agent in
            Button("Remove Agent", role: .destructive) {
                window.perform("Couldn't remove the agent") {
                    try await monitor.removeAgent(id: agent.id)
                    if window.selection == .agent(agent.id) {
                        window.selection = .workspace(agent.workspaceId)
                    }
                }
            }
        } message: { agent in
            Text("cos forgets this agent and its terminal. The Claude session \(agent.sessionId ?? "") stays in Claude's history, and the workspace's files aren't touched.")
        }
        .confirmationDialog(
            "Remove workspace “\(window.removingWorkspace?.title ?? "")”?",
            isPresented: isPresented($window.removingWorkspace),
            presenting: window.removingWorkspace
        ) { workspace in
            Button("Remove Workspace", role: .destructive) {
                window.perform("Couldn't remove the workspace") {
                    try await monitor.removeWorkspace(workspace)
                    if window.selection == .workspace(workspace.id) {
                        window.selection = .project(workspace.projectId)
                    }
                }
            }
        } message: { workspace in
            Text("This deletes the folder \(workspace.path). The branch \(workspace.branch) is kept, so no commits are lost. cos refuses if the workspace has uncommitted changes.")
        }
        .confirmationDialog(
            "Remove project “\(window.removingProject?.name ?? "")”?",
            isPresented: isPresented($window.removingProject),
            presenting: window.removingProject
        ) { project in
            Button("Remove Project", role: .destructive) {
                window.perform("Couldn't remove the project") {
                    try await monitor.removeProject(id: project.id)
                    if window.selection == .project(project.id) {
                        window.selection = .events
                    }
                }
            }
        } message: { project in
            Text("cos stops tracking it. Your repo at \(project.root) isn't touched. A project that still has workspaces can't be removed.")
        }
        .alert(window.alert?.title ?? "", isPresented: isPresented($window.alert), presenting: window.alert) { _ in
            Button("OK") {}
        } message: { alert in
            Text(alert.message)
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch window.selection {
        case .events, nil:
            EventLogView(monitor: monitor)
        case .waiting:
            WaitingOnYouView(monitor: monitor, window: window)
        case .agent(let id):
            // A fresh page (and terminal session) per agent.
            AgentPage(agentId: id, monitor: monitor, window: window)
                .id(id)
        case .session(let pid):
            if let session = monitor.sessions.first(where: { $0.pid == pid }) {
                ExternalSessionDetail(session: session, monitor: monitor)
            } else {
                ContentUnavailableView("Session ended", systemImage: "terminal", description: Text("This Claude session is no longer running."))
            }
        case .project(let id):
            if let project = monitor.project(id: id) {
                ProjectDetail(project: project, monitor: monitor, window: window)
            } else {
                ContentUnavailableView("No such project", systemImage: "folder", description: Text("It may have been removed."))
            }
        case .workspace(let id):
            if let workspace = monitor.workspace(id: id) {
                WorkspaceDetail(workspace: workspace, project: monitor.project(id: workspace.projectId), monitor: monitor, window: window)
            } else {
                ContentUnavailableView("No such workspace", systemImage: "arrow.triangle.branch", description: Text("It may have been removed."))
            }
        }
    }

    /// A Bool binding for "this optional is set", for dialogs that present an item.
    private func isPresented<T>(_ item: Binding<T?>) -> Binding<Bool> {
        Binding(get: { item.wrappedValue != nil }, set: { if !$0 { item.wrappedValue = nil } })
    }
}

private struct NotRunningBanner: View {
    let showHelp: () -> Void

    var body: some View {
        HStack {
            Circle().fill(.red).frame(width: 8, height: 8)
            Text("cos is not running. Changes can't be made until it starts.")
            Spacer()
            Button("How to Start It", action: showHelp)
        }
        .font(.callout)
        .padding(.horizontal)
        .padding(.vertical, 8)
        .background(.red.opacity(0.08))
        .overlay(alignment: .bottom) { Divider() }
    }
}

private struct Sidebar: View {
    let monitor: DaemonMonitor
    @Bindable var window: WindowState
    /// Projects start expanded; this remembers the ones you fold away.
    @State private var collapsed: Set<Int> = []

    private var canChange: Bool { monitor.connection == .running }

    var body: some View {
        List(selection: $window.selection) {
            Label("Event Log", systemImage: "list.bullet.rectangle")
                .tag(SidebarItem.events)
            if monitor.waitingCount > 0 || window.selection == .waiting {
                Label("Waiting on you", systemImage: "exclamationmark.bubble.fill")
                    .foregroundStyle(monitor.waitingCount > 0 ? Color.orange : Color.secondary)
                    .badge(monitor.waitingCount)
                    .tag(SidebarItem.waiting)
            }

            Section("Projects") {
                if monitor.projects.isEmpty {
                    Text(monitor.connection == .running ? "No projects yet. Add a git repo to get started." : "No projects to show.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .selectionDisabled()
                }
                ForEach(monitor.projects) { project in
                    DisclosureGroup(isExpanded: expanded(project.id)) {
                        ForEach(project.workspaces) { workspace in
                            workspaceRow(workspace)
                        }
                    } label: {
                        Label(project.name, systemImage: "folder")
                            .help(project.root)
                            .tag(SidebarItem.project(project.id))
                            .contextMenu { projectMenu(project) }
                    }
                }
                if let error = monitor.projectsError, monitor.connection == .running {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .selectionDisabled()
                }
            }

            if !monitor.sessions.isEmpty {
                Section("Other Claude sessions") {
                    ForEach(SessionGroup.groups(monitor)) { group in
                        Text(group.title)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .selectionDisabled()
                        ForEach(group.sessions) { session in
                            HStack(spacing: 6) {
                                SessionStatusDot(session: session)
                                Text(session.displayName).lineLimit(1)
                            }
                            .help(session.cwd)
                            .tag(SidebarItem.session(session.pid))
                            .contextMenu {
                                Button("Reveal in Finder") { Files.reveal(session.cwd) }
                                Button("Open in Terminal") { Files.openInTerminal(session.cwd) }
                            }
                        }
                    }
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            VStack(alignment: .leading, spacing: 8) {
                Divider()
                Button {
                    addProject()
                } label: {
                    Label("Add Project…", systemImage: "plus")
                }
                .buttonStyle(.borderless)
                .disabled(!canChange)
                Button {
                    window.selection = .events
                } label: {
                    HStack(spacing: 6) {
                        Circle().fill(monitor.connection.color).frame(width: 7, height: 7)
                        Text(monitor.connection.title)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
            .padding([.horizontal, .bottom], 12)
        }
    }

    /// A workspace, with its agents nested under it when it has any.
    @ViewBuilder
    private func workspaceRow(_ workspace: Workspace) -> some View {
        let agents = monitor.agents(inWorkspace: workspace.id)
        let label = Label(workspace.title, systemImage: "arrow.triangle.branch")
            .help(workspace.branch)
            .tag(SidebarItem.workspace(workspace.id))
            .contextMenu { workspaceMenu(workspace) }
        if agents.isEmpty {
            label
        } else {
            DisclosureGroup(isExpanded: expanded(workspaceKey(workspace.id))) {
                ForEach(agents) { agent in
                    HStack(spacing: 6) {
                        AgentStateDot(state: agent.state)
                        Text(agent.title).lineLimit(1)
                        if monitor.pendingApproval(forAgent: agent.id) != nil || agent.state == .needsTrust {
                            Spacer()
                            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange)
                        }
                    }
                    .help("\(agent.title): \(agent.state.title)")
                    .tag(SidebarItem.agent(agent.id))
                    .contextMenu { agentMenu(agent) }
                }
            } label: {
                label
            }
        }
    }

    /// Workspaces share the collapsed set with projects, on their own keys.
    private func workspaceKey(_ id: Int) -> Int { -id - 1 }

    @ViewBuilder
    private func agentMenu(_ agent: Agent) -> some View {
        if agent.state.isAlive {
            Button("Stop") { window.perform("Couldn't stop the agent") { try await monitor.stopAgent(id: agent.id) } }
                .disabled(!canChange)
        } else {
            Button("Resume") { window.perform("Couldn't resume the agent") { try await monitor.resumeAgent(id: agent.id) } }
                .disabled(!canChange)
        }
        Divider()
        Button("Remove Agent…") { window.removingAgent = agent }
            .disabled(!canChange || agent.state.isAlive)
    }

    @ViewBuilder
    private func projectMenu(_ project: Project) -> some View {
        Button("New Workspace…") { window.newWorkspaceIn = project }
            .disabled(!canChange)
        Divider()
        Button("Reveal in Finder") { Files.reveal(project.root) }
        Button("Open in Terminal") { Files.openInTerminal(project.root) }
        Divider()
        Button("Remove Project…") { window.removingProject = project }
            .disabled(!canChange)
    }

    @ViewBuilder
    private func workspaceMenu(_ workspace: Workspace) -> some View {
        Button("New Agent…") { window.newAgentIn = workspace }
            .disabled(!canChange || !monitor.agentsSupported)
        Button("Rename…") { window.renaming = workspace }
            .disabled(!canChange)
        Divider()
        Button("Reveal in Finder") { Files.reveal(workspace.path) }
        Button("Open in Terminal") { Files.openInTerminal(workspace.path) }
        Divider()
        Button("Remove Workspace…") { window.removingWorkspace = workspace }
            .disabled(!canChange)
    }

    private func expanded(_ id: Int) -> Binding<Bool> {
        Binding(
            get: { !collapsed.contains(id) },
            set: { isExpanded in
                if isExpanded { collapsed.remove(id) } else { collapsed.insert(id) }
            }
        )
    }

    private func addProject() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Add Project"
        panel.message = "Choose a git repository. Any folder inside it works."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        window.perform("Couldn't add the project") {
            let project = try await monitor.addProject(path: url.path)
            window.selection = .project(project.id)
        }
    }
}

/// Finder and Terminal for a folder.
enum Files {
    static func reveal(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    static func openInTerminal(_ path: String) {
        guard let terminal = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal") else { return }
        // Opening a folder with Terminal starts a new window in that folder.
        NSWorkspace.shared.open([URL(fileURLWithPath: path, isDirectory: true)], withApplicationAt: terminal, configuration: NSWorkspace.OpenConfiguration())
    }
}
