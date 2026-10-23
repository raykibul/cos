import CosKit
import SwiftUI

/// What a chief's info page shows below its header.
enum ChiefInfoTab: String, Hashable, CaseIterable, Identifiable, Sendable {
    case workspaces, sessions, runs

    var id: String { rawValue }

    var title: String {
        switch self {
        case .workspaces: "Workspaces"
        case .sessions: "Sessions"
        case .runs: "Runs"
        }
    }
}

/// A chief's info page, as a chat app shows a contact's: its project, quick
/// actions, and everything under it: each workspace with its sessions, all
/// sessions, and its runs. Reached from the chat's ⋯ menu or its title.
struct ChiefInfoView: View {
    var session: HostSession
    var projectId: Int
    @State private var tab: ChiefInfoTab
    @State private var newRun = false
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss

    init(session: HostSession, projectId: Int, tab: ChiefInfoTab = .workspaces) {
        self.session = session
        self.projectId = projectId
        _tab = State(initialValue: tab)
    }

    var body: some View {
        let project = session.project(id: projectId)
        let staff = session.staffs[projectId]
        List {
            Section {
                VStack(spacing: 6) {
                    ProjectAvatar(name: project?.name ?? "?", size: 84)
                    Text(project?.name ?? "Project \(projectId)")
                        .font(.title2.weight(.bold))
                        .foregroundStyle(Palette.ink)
                        .padding(.top, 4)
                    if let root = project?.root {
                        Text(root)
                            .font(.footnote.monospaced())
                            .foregroundStyle(Palette.ink3)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Text(chiefLine(staff))
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink2)
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, 4)
                .listRowBackground(Color.clear)

                HStack(spacing: 8) {
                    QuickAction(title: "Chat", symbol: "bubble.left.fill") { openChat() }
                    QuickAction(title: "Terminal", symbol: "terminal") {
                        if let id = staff?.agentId { navigator.open(.terminal(agentId: id)) }
                    }
                    .disabled(staff?.agentId == nil)
                    QuickAction(title: "New run", symbol: "plus") { newRun = true }
                }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 4, trailing: 0))

                Picker("Show", selection: $tab) {
                    ForEach(ChiefInfoTab.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 0, trailing: 0))
            }
            .listRowSeparator(.hidden)

            switch tab {
            case .workspaces: workspaces(project)
            case .sessions: sessions()
            case .runs: runs()
            }
        }
        .pageBackground()
        .navigationTitle(project?.name ?? "Chief of Staff")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .refreshable { await session.refresh() }
        .sheet(isPresented: $newRun) { NewRunSheet(session: session, workspaceId: nil) }
    }

    // MARK: Tabs

    /// Each workspace, newest first, with its sessions under it.
    @ViewBuilder
    private func workspaces(_ project: Project?) -> some View {
        let workspaces = (project?.workspaces ?? []).sorted { $0.createdAt > $1.createdAt }
        if workspaces.isEmpty {
            Section {
                Text("No workspaces yet. Ask the Chief of Staff to start work, and it makes one.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink3)
            }
        }
        ForEach(workspaces) { workspace in
            let agents = session.agents(inWorkspace: workspace.id)
            let activeRun = session.runs(inWorkspace: workspace.id).first { !$0.state.isFinished }
            Section {
                NavigationLink(value: Route.workspace(id: workspace.id)) {
                    WorkspaceHeaderRow(workspace: workspace, waiting: agents.filter { needsYou($0) }.count)
                }
                ForEach(agents) { agent in
                    NavigationLink(value: Route.terminal(agentId: agent.id)) {
                        SessionRow(agent: agent, waiting: needsYou(agent))
                    }
                    .swipeActions {
                        Button("Details", systemImage: "info.circle") { navigator.open(.agent(id: agent.id)) }
                            .tint(Palette.ink2)
                    }
                }
                if let activeRun {
                    NavigationLink(value: Route.run(id: activeRun.id)) {
                        Label(activeRun.goal, systemImage: "list.bullet.clipboard")
                            .labelStyle(.tight)
                            .font(.subheadline)
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                            .padding(.leading, 28)
                    }
                }
            }
        }
    }

    /// Every session in the project: what needs you first, then the rest,
    /// newest first.
    @ViewBuilder
    private func sessions() -> some View {
        let agents = session.agents(inProject: projectId)
        let waiting = agents.filter { needsYou($0) }
        let others = agents.filter { !needsYou($0) }
        if agents.isEmpty {
            Section {
                Text("No sessions yet.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink3)
            }
        }
        if !waiting.isEmpty {
            Section("Needs you") {
                ForEach(waiting) { agent in
                    NavigationLink(value: Route.terminal(agentId: agent.id)) {
                        SessionRow(agent: agent, waiting: true, place: session.workspaceName(agent.workspaceId))
                    }
                    .swipeActions {
                        Button("Details", systemImage: "info.circle") { navigator.open(.agent(id: agent.id)) }
                            .tint(Palette.ink2)
                    }
                }
            }
        }
        if !others.isEmpty {
            Section(waiting.isEmpty ? "Sessions" : "Others") {
                ForEach(others) { agent in
                    NavigationLink(value: Route.terminal(agentId: agent.id)) {
                        SessionRow(agent: agent, waiting: false, place: session.workspaceName(agent.workspaceId))
                    }
                    .swipeActions {
                        Button("Details", systemImage: "info.circle") { navigator.open(.agent(id: agent.id)) }
                            .tint(Palette.ink2)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func runs() -> some View {
        let runs = session.runs(inProject: projectId)
        Section {
            if runs.isEmpty {
                Text("No runs yet. A run is a goal a chief plans as tasks for workers.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink3)
            }
            ForEach(runs) { run in
                NavigationLink(value: Route.run(id: run.id)) {
                    RunRow(run: run, place: session.workspaceName(run.workspaceId))
                }
            }
            Button("New run", systemImage: "plus") { newRun = true }
        }
    }

    // MARK: Helpers

    private func needsYou(_ agent: Agent) -> Bool {
        agent.state == .needsTrust || agent.state == .needsLogin || session.pendingApproval(forAgent: agent.id)?.isForYou == true
    }

    private func chiefLine(_ staff: Staff?) -> String {
        var parts = ["Chief of Staff"]
        if let staff {
            parts.append(staff.stateLabel)
            if let model = staff.model { parts.append(model) }
        }
        return parts.joined(separator: " · ")
    }

    /// Back to the chat if it's the screen below, else to it.
    private func openChat() {
        if navigator.chiefs.dropLast().last == Route.chief(projectId: projectId) {
            dismiss()
        } else {
            navigator.open(.chief(projectId: projectId))
        }
    }
}

/// A rounded button under the header: Chat, Screen, New run.
struct QuickAction: View {
    var title: String
    var symbol: String
    var action: () -> Void
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Button(action: action) {
            VStack(spacing: 5) {
                Image(systemName: symbol)
                    .font(.system(size: 19, weight: .medium))
                Text(title)
                    .font(.caption.weight(.medium))
            }
            .foregroundStyle(isEnabled ? Palette.claude : Palette.ink3)
            .frame(maxWidth: .infinity, minHeight: 62)
            .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(.plain)
    }
}

/// A workspace as the first row of its section: name, branch, base.
struct WorkspaceHeaderRow: View {
    var workspace: Workspace
    var waiting: Int

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.triangle.branch")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(Palette.ink2)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(workspace.displayName)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                Text("\(workspace.branch) · from \(workspace.baseRef.isEmpty ? workspace.base.rawValue : workspace.baseRef)")
                    .font(.caption.monospaced())
                    .foregroundStyle(Palette.ink3)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            if waiting > 0 {
                CountBadge(count: waiting)
                    .accessibilityLabel("\(waiting) waiting on you")
            }
        }
        .padding(.vertical, 2)
    }
}

/// A session (an agent): its state dot, title, and state or what it last
/// said.
struct SessionRow: View {
    var agent: Agent
    var waiting: Bool
    var place: String?

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            StateDot(color: waiting ? Palette.claude : agent.state.style.color)
                .padding(.top, 6)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.title)
                    .font(.body)
                    .foregroundStyle(agent.state.isAlive ? Palette.ink : Palette.ink2)
                    .lineLimit(1)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(waiting ? Palette.claude : agent.state == .working ? Palette.blue : Palette.ink3)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 1)
    }

    private var detail: String {
        var parts: [String] = []
        if let place, !place.isEmpty { parts.append(place) }
        parts.append(waiting && agent.state == .waitingApproval ? "needs your approval" : agent.state.style.label.lowercased())
        if let ago = agent.lastActivityMs.map({ shortAgo($0) }), !agent.state.isAlive || agent.state == .idle { parts.append(ago) }
        return parts.joined(separator: " · ")
    }
}
