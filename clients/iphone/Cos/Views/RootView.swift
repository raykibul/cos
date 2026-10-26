import CosKit
import SwiftUI

/// Somewhere a tab's navigation stack can go.
enum Route: Hashable {
    /// Everything blocked on you, from the pinned row of the chiefs list.
    case waiting
    case chief(projectId: Int)
    /// A chief's workspaces and sessions, from its chat's ⋯ menu.
    case chiefInfo(projectId: Int, tab: ChiefInfoTab = .workspaces)
    /// A chief's identity, brief and memory notes.
    case chiefMemory(projectId: Int)
    case workspace(id: Int)
    case agent(id: Int)
    /// A session as a chat: its transcript and a composer.
    case session(agentId: Int)
    /// An agent's live terminal, full screen: the chief's or a session's.
    case terminal(agentId: Int)
}

enum AppTab: Hashable {
    case chiefs, hosts
}

/// Each tab's navigation, kept here so a launch argument (screenshots) or a
/// row in another tab can open a screen.
@MainActor
@Observable
final class Navigator {
    var tab: AppTab = .chiefs
    /// The add-host sheet is up (hosts tab, or the welcome screen).
    var addingHost = false
    var chiefs: [Route] = []

    /// Opens `route` in the chiefs tab, where every screen lives.
    func open(_ route: Route) {
        tab = .chiefs
        chiefs.append(route)
    }

    /// `-screen <name>`: the screen a screenshot wants, with the fake data's ids.
    func apply(screen: String?) {
        switch screen {
        case "waiting": tab = .chiefs; chiefs = [.waiting]
        case "chief": tab = .chiefs; chiefs = [.chief(projectId: 1)]
        case "chief-new": tab = .chiefs; chiefs = [.chief(projectId: 2)]
        case "chief-info": tab = .chiefs; chiefs = [.chief(projectId: 1), .chiefInfo(projectId: 1)]
        case "chief-memory": tab = .chiefs; chiefs = [.chief(projectId: 1), .chiefMemory(projectId: 1)]
        case "terminal": tab = .chiefs; chiefs = [.chief(projectId: 1), .terminal(agentId: 7)]
        case "chief-terminal": tab = .chiefs; chiefs = [.chief(projectId: 1), .terminal(agentId: 40)]
        case "workspace": tab = .chiefs; chiefs = [.chiefInfo(projectId: 1), .workspace(id: 5)]
        case "agent": tab = .chiefs; chiefs = [.chiefInfo(projectId: 1), .agent(id: 7)]
        case "session": tab = .chiefs; chiefs = [.chief(projectId: 1), .session(agentId: 7)]
        case "session-approval": tab = .chiefs; chiefs = [.chief(projectId: 1), .session(agentId: 8)]
        case "agent-approval": tab = .chiefs; chiefs = [.chiefInfo(projectId: 1), .agent(id: 8)]
        case "hosts": tab = .hosts
        case "add-host": tab = .hosts; addingHost = true
        default: tab = .chiefs
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
            Tab("Chiefs", systemImage: "bubble.left.and.bubble.right.fill", value: AppTab.chiefs) {
                NavigationStack(path: $navigator.chiefs) {
                    SessionGate { ChiefsView(session: $0) }
                        .withRoutes()
                }
            }
            .badge(app.session?.waitingCount ?? 0)

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
            case .waiting: WaitingView(session: session)
            case .chief(let projectId): ChiefChatView(session: session, projectId: projectId)
            case .chiefInfo(let projectId, let tab): ChiefInfoView(session: session, projectId: projectId, tab: tab)
            case .chiefMemory(let projectId): ChiefMemoryView(session: session, projectId: projectId)
            case .workspace(let id): WorkspaceView(session: session, workspaceId: id)
            case .agent(let id): AgentView(session: session, agentId: id)
            case .session(let agentId): SessionChatView(session: session, agentId: agentId)
            case .terminal(let agentId): TerminalScreen(session: session, agentId: agentId)
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
