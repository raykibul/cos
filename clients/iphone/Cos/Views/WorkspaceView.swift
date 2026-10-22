import CosKit
import SwiftUI

/// A workspace: its branch, its agents (state and last message) and its runs.
struct WorkspaceView: View {
    var session: HostSession
    var workspaceId: Int
    @State private var newRun = false

    var body: some View {
        let workspace = session.workspace(id: workspaceId)
        let agents = session.agents(inWorkspace: workspaceId)
        let runs = session.runs(inWorkspace: workspaceId)
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
                    NavigationLink(value: Route.agent(id: agent.id)) {
                        AgentRow(agent: agent, waiting: session.pendingApproval(forAgent: agent.id)?.isForYou == true)
                    }
                }
            }

            Section("Runs") {
                if runs.isEmpty {
                    Text("No runs in this workspace.")
                        .font(.subheadline)
                        .foregroundStyle(Palette.ink3)
                }
                ForEach(runs) { run in
                    NavigationLink(value: Route.run(id: run.id)) {
                        RunRow(run: run, place: nil)
                    }
                }
                Button {
                    newRun = true
                } label: {
                    Label("New run", systemImage: "plus")
                }
                .disabled(runs.contains { !$0.state.isFinished })
            }
        }
        .pageBackground()
        .refreshable { await session.refresh() }
        .navigationTitle(workspace?.displayName ?? "Workspace")
        .sheet(isPresented: $newRun) {
            NewRunSheet(session: session, workspaceId: workspaceId)
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
        if agent.role == .chief { return "chief · \(agent.state.style.label.lowercased())" }
        if agent.role == .worker { return "worker · \(agent.state.style.label.lowercased())" }
        if agent.role == .reviewer { return "reviewer · \(agent.state.style.label.lowercased())" }
        return agent.state.style.label
    }
}

/// `POST /v1/runs`: a goal in a workspace.
struct NewRunSheet: View {
    var session: HostSession
    var workspaceId: Int?
    @Environment(\.dismiss) private var dismiss
    @Environment(Navigator.self) private var navigator
    @State private var goal = ""
    @State private var chosen: Int?
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("What should get done?", text: $goal, axis: .vertical)
                        .lineLimit(3...8)
                } header: {
                    Text("Goal")
                } footer: {
                    Text("A chief plans it as tasks and asks you to approve the plan before workers start.")
                }
                if workspaceId == nil {
                    Section("Workspace") {
                        Picker("Workspace", selection: $chosen) {
                            Text("Choose…").tag(Int?.none)
                            ForEach(session.projects) { project in
                                ForEach(project.workspaces) { workspace in
                                    Text("\(project.name) · \(workspace.displayName)").tag(Int?.some(workspace.id))
                                }
                            }
                        }
                        .pickerStyle(.navigationLink)
                    }
                }
                if let error {
                    Section { Text(error).foregroundStyle(Palette.red) }
                }
            }
            .navigationTitle("New run")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Start", action: start)
                        .disabled(busy || goal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (workspaceId ?? chosen) == nil)
                }
            }
        }
    }

    private func start() {
        guard let workspace = workspaceId ?? chosen else { return }
        busy = true
        Task {
            defer { busy = false }
            do {
                let run = try await session.startRun(workspaceId: workspace, goal: goal)
                dismiss()
                navigator.open(.run(id: run.id))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
