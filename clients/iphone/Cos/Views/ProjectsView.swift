import CosKit
import SwiftUI

/// Every project, as the desktop's sidebar shows them: its Chief of Staff
/// first, then its workspaces, newest first.
struct ProjectsView: View {
    var session: HostSession

    var body: some View {
        List {
            if session.projects.isEmpty {
                ContentUnavailableView("No projects", systemImage: "folder", description: Text("Add one on the host: cos project add <path>"))
                    .listRowBackground(Color.clear)
            }
            ForEach(session.projects) { project in
                Section {
                    NavigationLink(value: Route.chief(projectId: project.id)) {
                        ChiefRow(staff: session.staffs[project.id])
                    }
                    ForEach(project.workspaces.sorted { $0.createdAt > $1.createdAt }) { workspace in
                        NavigationLink(value: Route.workspace(id: workspace.id)) {
                            WorkspaceRow(session: session, workspace: workspace)
                        }
                    }
                } header: {
                    HStack {
                        Text(project.name)
                        Spacer()
                        Text(project.workspaces.count == 1 ? "1 workspace" : "\(project.workspaces.count) workspaces")
                            .textCase(nil)
                            .foregroundStyle(Palette.ink3)
                    }
                }
            }
        }
        .pageBackground()
        .refreshable { await session.refresh() }
        .navigationTitle("Projects")
    }
}

struct ChiefRow: View {
    var staff: Staff?

    var body: some View {
        HStack(spacing: 12) {
            ChiefStar(size: 17)
                .frame(width: 30, height: 30)
                .background(Palette.amber.opacity(0.14), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("Chief of Staff")
                    .font(.body.weight(.semibold))
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(Palette.ink3)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            if let waiting = staff?.waitingOnYou, waiting > 0 {
                Text("\(waiting)")
                    .font(.caption.weight(.bold).monospacedDigit())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Palette.claude, in: Capsule())
                    .accessibilityLabel("\(waiting) waiting on you")
            }
        }
        .padding(.vertical, 2)
    }

    private var detail: String {
        guard let staff else { return "Not started" }
        var parts = [staff.stateLabel]
        if staff.workingAgents > 0 { parts.append("\(staff.workingAgents) working") }
        return parts.joined(separator: " · ")
    }
}

struct WorkspaceRow: View {
    var session: HostSession
    var workspace: Workspace

    var body: some View {
        let agents = session.agents(inWorkspace: workspace.id)
        let activeRun = session.runs(inWorkspace: workspace.id).first { !$0.state.isFinished }
        HStack(spacing: 12) {
            Image(systemName: "arrow.triangle.branch")
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Palette.ink2)
                .frame(width: 30, height: 30)
                .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text(workspace.displayName)
                    .font(.body)
                    .foregroundStyle(workspace.title == nil ? Palette.ink2 : Palette.ink)
                Text(subtitle(agents: agents, run: activeRun))
                    .font(.caption)
                    .foregroundStyle(Palette.ink3)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            HStack(spacing: 3) {
                ForEach(agents.prefix(5)) { agent in
                    StateDot(color: agent.state.style.color)
                }
            }
        }
        .padding(.vertical, 2)
    }

    private func subtitle(agents: [Agent], run: Run?) -> String {
        var parts = [workspace.branch]
        let live = agents.filter(\.state.isAlive).count
        if live > 0 { parts.append(live == 1 ? "1 agent" : "\(live) agents") }
        if let run { parts.append(run.state.style.label.lowercased()) }
        return parts.joined(separator: " · ")
    }
}
