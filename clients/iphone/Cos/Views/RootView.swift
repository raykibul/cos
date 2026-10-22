import CosKit
import SwiftUI

/// Somewhere a tab's navigation stack can go.
enum Route: Hashable {
    case chief(projectId: Int)
    case workspace(id: Int)
    case run(id: Int)
    case agent(id: Int)
}

enum AppTab: Hashable {
    case waiting, projects, runs, hosts
}

/// Each tab's navigation, kept here so a launch argument (screenshots) or a
/// row in another tab can open a screen.
@MainActor
@Observable
final class Navigator {
    var tab: AppTab = .waiting
    var waiting: [Route] = []
    var projects: [Route] = []
    var runs: [Route] = []

    /// Opens `route` in the tab it belongs to.
    func open(_ route: Route) {
        switch route {
        case .chief, .workspace:
            tab = .projects
            projects.append(route)
        case .run:
            tab = .runs
            runs.append(route)
        case .agent:
            tab = .projects
            projects.append(route)
        }
    }

    /// `-screen <name>`: the screen a screenshot wants, with the fake data's ids.
    func apply(screen: String?) {
        switch screen {
        case "projects": tab = .projects
        case "chief": tab = .projects; projects = [.chief(projectId: 1)]
        case "chief-new": tab = .projects; projects = [.chief(projectId: 2)]
        case "workspace": tab = .projects; projects = [.workspace(id: 5)]
        case "agent": tab = .projects; projects = [.workspace(id: 3), .agent(id: 7)]
        case "agent-approval": tab = .projects; projects = [.workspace(id: 4), .agent(id: 8)]
        case "runs": tab = .runs
        case "run": tab = .runs; runs = [.run(id: 3)]
        case "plan": tab = .runs; runs = [.run(id: 4)]
        case "report": tab = .runs; runs = [.run(id: 2)]
        case "hosts": tab = .hosts
        default: tab = .waiting
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.scenePhase) private var phase
    @State private var navigator = Navigator()

    var body: some View {
        Group {
            if app.hosts.hosts.isEmpty {
                WelcomeView()
            } else {
                tabs
            }
        }
        .tint(Palette.ink)
        .environment(navigator)
        .onAppear {
            navigator.apply(screen: app.launch.screen)
            app.setActive(phase == .active)
        }
        .onChange(of: phase) { _, phase in
            app.setActive(phase == .active)
        }
    }

    private var tabs: some View {
        @Bindable var navigator = navigator
        return TabView(selection: $navigator.tab) {
            Tab("Waiting", systemImage: "tray.full", value: AppTab.waiting) {
                NavigationStack(path: $navigator.waiting) {
                    SessionGate { WaitingView(session: $0) }
                        .withRoutes()
                }
            }
            .badge(app.session?.waitingCount ?? 0)

            Tab("Projects", systemImage: "square.stack.3d.up", value: AppTab.projects) {
                NavigationStack(path: $navigator.projects) {
                    SessionGate { ProjectsView(session: $0) }
                        .withRoutes()
                }
            }

            Tab("Runs", systemImage: "list.bullet.clipboard", value: AppTab.runs) {
                NavigationStack(path: $navigator.runs) {
                    SessionGate { RunsView(session: $0) }
                        .withRoutes()
                }
            }

            Tab("Hosts", systemImage: "desktopcomputer", value: AppTab.hosts) {
                NavigationStack {
                    HostsView()
                }
            }
        }
    }
}

extension View {
    /// The screens a tab's stack can push.
    func withRoutes() -> some View {
        navigationDestination(for: Route.self) { route in
            RouteView(route: route)
        }
    }
}

struct RouteView: View {
    @Environment(AppModel.self) private var app
    var route: Route

    var body: some View {
        if let session = app.session {
            switch route {
            case .chief(let projectId): ChiefChatView(session: session, projectId: projectId)
            case .workspace(let id): WorkspaceView(session: session, workspaceId: id)
            case .run(let id): RunView(session: session, runId: id)
            case .agent(let id): AgentView(session: session, agentId: id)
            }
        }
    }
}

/// The host picker in each tab's navigation bar: the current host, and a
/// switch to any other.
struct HostMenu: ToolbarContent {
    @Environment(AppModel.self) private var app
    @Environment(Navigator.self) private var navigator

    var body: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Menu {
                ForEach(app.hosts.hosts) { host in
                    Button {
                        app.selectHost(host.id)
                    } label: {
                        if host.id == app.hosts.current?.id {
                            Label(host.name, systemImage: "checkmark")
                        } else {
                            Text(host.name)
                        }
                    }
                }
                Divider()
                Button("Manage hosts…", systemImage: "desktopcomputer") { navigator.tab = .hosts }
            } label: {
                HStack(spacing: 6) {
                    ConnectionDot(connection: app.session?.connection)
                    Text(app.hosts.current?.name ?? "No host")
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                }
                .foregroundStyle(Palette.ink)
            }
            .accessibilityLabel("Host: \(app.hosts.current?.name ?? "none")")
        }
    }
}

struct ConnectionDot: View {
    var connection: Connection?

    var body: some View {
        StateDot(color: color)
    }

    private var color: Color {
        switch connection {
        case .live?: Palette.green
        case .connecting?, nil: Palette.ink3
        case .refused?: Palette.claude
        case .unreachable?, .failed?: Palette.red
        }
    }
}
