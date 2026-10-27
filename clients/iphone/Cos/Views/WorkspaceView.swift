import CosKit
import SwiftUI

/// A workspace: its branch and its agents (state and last message). An
/// agent opens its chat; swipe it for its terminal or details, or
/// long-press it to start another session here or remove it.
struct WorkspaceView: View {
    var session: HostSession
    var workspaceId: Int
    @Environment(Navigator.self) private var navigator
    @State private var error: String?

    var body: some View {
        let workspace = session.workspace(id: workspaceId)
        let agents = session.agents(inWorkspace: workspaceId)
        List {
            if let workspace {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Label(workspace.branch, systemImage: "arrow.triangle.branch")
                            .labelStyle(.tight)
                            .font(.subheadline.monospaced())
                            .foregroundStyle(Palette.ink2)
                        Text("From \(workspace.baseRef.isEmpty ? workspace.base.rawValue : workspace.baseRef) · \(workspace.shortCommit) · \(shortAgo(workspace.createdAt))")
                            .font(.caption)
                            .foregroundStyle(Palette.ink3)
                    }
                    .padding(.vertical, 2)
                }
            }

            Section("Agents") {
                if agents.isEmpty {
                    Text("No agents yet. Ask the Chief of Staff to start one.")
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink3)
                }
                ForEach(agents) { agent in
                    NavigationLink(value: Route.session(agentId: agent.id)) {
                        AgentRow(agent: agent, waiting: session.pendingApproval(forAgent: agent.id)?.isForYou == true)
                    }
                    .swipeActions {
                        Button("Details", systemImage: "info.circle") { navigator.open(.agent(id: agent.id)) }
                            .tint(Palette.ink2)
                        Button("Terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: agent.id)) }
                            .tint(Palette.blue)
                    }
                    .sessionActions(session: session, agent: agent)
                }
            }
        }
        .pageBackground()
        .refreshable { await session.refresh() }
        .navigationTitle(workspace?.displayName ?? "Workspace")
        .toolbar {
            if session.shellsSupported, let workspace {
                ToolbarItem(placement: .primaryAction) {
                    Button("New shell", systemImage: "apple.terminal") {
                        Task { error = await navigator.openShell(session: session, cwd: workspace.path, projectId: workspace.projectId) }
                    }
                }
            }
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
    }
}

struct AgentRow: View {
    var agent: Agent
    var waiting: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Image(systemName: agent.state.style.symbol)
                    .font(.caption)
                    .foregroundStyle(agent.state.style.color)
                Text(agent.title)
                    .font(.body.weight(.medium))
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text(roleOrState)
                    .font(.caption)
                    .foregroundStyle(waiting || agent.state.needsYou ? Palette.claude : Palette.ink3)
            }
            if let message = agent.lastMessage, !message.isEmpty {
                Text(inlineMarkdown(message, base: .footnote))
                    .font(.footnote)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(2)
            }
        }
        .padding(.vertical, 2)
    }

    private var roleOrState: String {
        if agent.role == .staff { return "chief · \(agent.state.style.label.lowercased())" }
        return agent.state.style.label
    }
}
