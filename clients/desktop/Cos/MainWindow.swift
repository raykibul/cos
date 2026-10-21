import AppKit
import CosKit
import SwiftUI

enum SidebarItem: Hashable {
    case events
    case project(Int)
    case workspace(Int)
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
    @State private var window: WindowState

    init(monitor: DaemonMonitor, window: WindowState = WindowState()) {
        self.monitor = monitor
        _window = State(initialValue: window)
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
                            Label(workspace.title, systemImage: "arrow.triangle.branch")
                                .help(workspace.branch)
                                .tag(SidebarItem.workspace(workspace.id))
                                .contextMenu { workspaceMenu(workspace) }
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
