import CosKit
import SwiftUI

extension View {
    /// A session row's long-press menu (Chat, Terminal, Details, New
    /// session and New shell in its folder, Remove session…) and the
    /// sheets it opens.
    func sessionActions(session: HostSession, agent: Agent) -> some View {
        modifier(SessionActions(session: session, agent: agent))
    }
}

private struct SessionActions: ViewModifier {
    var session: HostSession
    var agent: Agent
    @State private var starting = false
    @State private var removing = false
    @State private var error: String?
    @Environment(Navigator.self) private var navigator

    func body(content: Content) -> some View {
        content
            .contextMenu {
                Button("Chat", systemImage: "text.bubble") { navigator.open(.session(agentId: agent.id)) }
                Button("Terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: agent.id)) }
                Button("Details", systemImage: "info.circle") { navigator.open(.agent(id: agent.id)) }
                Divider()
                SessionFolderActions(session: session, agent: agent, starting: $starting, removing: $removing) {
                    Task { error = await navigator.openShell(session: session, cwd: session.folder(ofAgent: agent), projectId: session.projectId(ofAgent: agent.id)) }
                }
            }
            .sessionSheets(session: session, agent: agent, starting: $starting, removing: $removing)
            .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(error ?? "")
            }
    }
}

/// New session and New shell in its folder, and Remove session…, for a menu.
struct SessionFolderActions: View {
    var session: HostSession
    var agent: Agent
    @Binding var starting: Bool
    @Binding var removing: Bool
    var openShell: () -> Void

    var body: some View {
        if agent.workspaceId != nil, agent.role != .staff {
            Button("New session in \(session.workspaceName(agent.workspaceId))", systemImage: "plus.bubble") { starting = true }
            if session.shellsSupported {
                Button("New shell in \(session.workspaceName(agent.workspaceId))", systemImage: "apple.terminal") { openShell() }
            }
            Button("Remove session…", systemImage: "trash", role: .destructive) { removing = true }
        }
    }
}

extension View {
    /// The New session and Remove session sheets for `agent`.
    func sessionSheets(session: HostSession, agent: Agent, starting: Binding<Bool>, removing: Binding<Bool>) -> some View {
        sheet(isPresented: starting) {
            if let workspaceId = agent.workspaceId {
                NewSessionSheet(session: session, workspaceId: workspaceId)
            }
        }
        .sheet(isPresented: removing) {
            RemoveSessionSheet(session: session, agent: agent)
        }
    }
}

/// Starts another session in a folder, beside the ones working there.
struct NewSessionSheet: View {
    var session: HostSession
    var workspaceId: Int
    @State private var prompt = ""
    @State private var busy = false
    @State private var error: String?
    @FocusState private var focused: Bool
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let workspace = session.workspace(id: workspaceId)
        let others = session.agents(inWorkspace: workspaceId)
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $prompt)
                        .focused($focused)
                        .frame(minHeight: 140)
                        .overlay(alignment: .topLeading) {
                            if prompt.isEmpty {
                                Text("What should it do?")
                                    .foregroundStyle(Palette.ink3)
                                    .padding(.top, 8)
                                    .padding(.leading, 5)
                                    .allowsHitTesting(false)
                            }
                        }
                } header: {
                    Text("Prompt")
                } footer: {
                    Text(footer(workspace: workspace, others: others))
                }
                if let error {
                    Section { Text(error).font(.footnote).foregroundStyle(Palette.red) }
                }
            }
            .navigationTitle("New session in \(session.workspaceName(workspaceId))")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(busy ? "Starting…" : "Start") { start() }
                        .disabled(busy || prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .onAppear { focused = true }
        }
        .interactiveDismissDisabled(busy)
    }

    /// "On atlas/lisbon, with Fix the session timeout."
    private func footer(workspace: Workspace?, others: [Agent]) -> String {
        var text = workspace.map { "Claude starts in this folder, on \($0.branch)" } ?? "Claude starts in this folder"
        if !others.isEmpty {
            text += ", beside " + others.prefix(3).map(\.title).joined(separator: ", ")
            if others.count > 3 { text += " and \(others.count - 3) more" }
        }
        return text + "."
    }

    private func start() {
        busy = true
        Task {
            defer { busy = false }
            do {
                let agent = try await session.startSession(workspaceId: workspaceId, prompt: prompt)
                dismiss()
                navigator.open(.session(agentId: agent.id))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// Removes a session, as the desktop's sheet does: a running one is
/// stopped first ("Stop and remove"); when no other session works in its
/// folder, the folder goes too unless you untick it (the branch stays);
/// when others share it, only the session goes and the sheet names them.
struct RemoveSessionSheet: View {
    var session: HostSession
    var agent: Agent
    @State private var withFolder = true
    @State private var busy = false
    @State private var error: String?
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let others = session.sessionsSharingFolder(with: agent)
        let folder = session.workspaceName(agent.workspaceId)
        let workspace = session.workspace(id: agent.workspaceId)
        let running = session.agent(id: agent.id)?.state.isAlive ?? agent.state.isAlive
        NavigationStack {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(agent.title)
                            .font(.headline)
                        Text(running
                            ? "It's running, so cos stops it first. Its conversation stays in Claude's folder."
                            : "cos forgets it. Its conversation stays in Claude's folder.")
                            .font(.subheadline)
                            .foregroundStyle(Palette.ink2)
                    }
                    .padding(.vertical, 2)
                }
                Section {
                    if others.isEmpty {
                        Toggle("Delete its folder, \(folder), too", isOn: $withFolder)
                    } else {
                        Text("\(names(others)) \(others.count == 1 ? "works" : "work") in \(folder) too, so the folder stays.")
                            .font(.subheadline)
                            .foregroundStyle(Palette.ink2)
                    }
                } footer: {
                    if others.isEmpty, withFolder {
                        Text(workspace.map { "The branch \($0.branch) stays. A folder with uncommitted changes stays too: untick this to remove just the session." }
                            ?? "A folder with uncommitted changes stays: untick this to remove just the session.")
                    }
                }
                if let error {
                    Section { Text(error).font(.footnote).foregroundStyle(Palette.red) }
                }
            }
            .navigationTitle("Remove session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(busy ? (running ? "Stopping…" : "Removing…") : (running ? "Stop and remove" : "Remove"), role: .destructive) {
                        remove(withFolder: others.isEmpty && withFolder)
                    }
                    .disabled(busy)
                    .tint(Palette.red)
                }
            }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(busy)
    }

    private func names(_ agents: [Agent]) -> String {
        let titles = agents.prefix(3).map(\.title)
        let rest = agents.count - titles.count
        return titles.joined(separator: ", ") + (rest > 0 ? " and \(rest) more" : "")
    }

    private func remove(withFolder: Bool) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await session.removeSession(agent, withFolder: withFolder)
                navigator.forget(agentId: agent.id, workspaceId: withFolder ? agent.workspaceId : nil)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
