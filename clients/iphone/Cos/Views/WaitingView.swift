import CosKit
import SwiftUI

/// The home tab: everything blocked on you, answerable in place.
struct WaitingView: View {
    var session: HostSession
    @State private var error: String?

    var body: some View {
        let items = session.waiting
        Group {
            if items.isEmpty {
                ContentUnavailableView {
                    Label("Nothing waits on you", systemImage: "checkmark.circle")
                } description: {
                    Text("Approvals, trust dialogs and agents that need a login show up here.")
                }
                .background(Palette.page)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(items) { item in
                            WaitingCard(session: session, item: item, error: $error)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                }
                .background(Palette.page)
            }
        }
        .refreshable { await session.refresh() }
        .navigationTitle("Waiting on you")
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
    }
}

/// One card per item, styled like the desktop's Waiting page.
struct WaitingCard: View {
    var session: HostSession
    var item: WaitingItem
    @Binding var error: String?
    @Environment(Navigator.self) private var navigator
    @State private var busy = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch item {
            case .approval(let approval): approvalCard(approval)
            case .trust(let agent): trustCard(agent)
            case .login(let agent): loginCard(agent)
            }
        }
        .padding(14)
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Palette.line, lineWidth: 0.5))
        .disabled(busy)
    }

    private func header(_ symbol: String, _ kind: String, _ title: String, _ subtitle: String?) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Label(kind, systemImage: symbol)
                .labelStyle(.tight)
                .font(.caption.weight(.semibold))
                .foregroundStyle(Palette.claude)
            Text(title)
                .font(.headline)
                .foregroundStyle(Palette.ink)
            if let subtitle, !subtitle.isEmpty {
                Text(subtitle)
                    .font(.footnote)
                    .foregroundStyle(Palette.ink3)
            }
        }
    }

    private func place(of agent: Agent?) -> String? {
        guard let agent else { return nil }
        let workspace = session.workspace(id: agent.workspaceId)
        let project = session.project(ofWorkspace: agent.workspaceId)
        return [project?.name, workspace?.displayName].compactMap { $0 }.joined(separator: " · ")
    }

    @ViewBuilder
    private func approvalCard(_ approval: Approval) -> some View {
        let agent = session.agent(id: approval.agentId)
        header("hand.raised", "Approval", agent?.title ?? "Agent \(approval.agentId)", place(of: agent))
        Text(approval.toolName == "Bash" ? "Claude wants to run" : "Claude wants to use \(approval.toolName)")
            .font(.subheadline)
            .foregroundStyle(Palette.ink2)
        CodeBox(text: approval.detail, lineLimit: 8)
        if let reason = approval.reason, !reason.isEmpty {
            Text("cos asks you because \(reason)")
                .font(.footnote)
                .foregroundStyle(Palette.ink3)
        }
        HStack(spacing: 10) {
            Button("Deny") { decide(approval, .deny) }
                .buttonStyle(QuietButtonStyle())
            Button("Allow") { decide(approval, .allow) }
                .buttonStyle(ClaudeButtonStyle())
        }
        .padding(.top, 2)
    }

    @ViewBuilder
    private func trustCard(_ agent: Agent) -> some View {
        header("lock", "Trust dialog", agent.title, place(of: agent))
        Text("Claude asks whether to trust this workspace's folder before it starts.")
            .font(.subheadline)
            .foregroundStyle(Palette.ink2)
        HStack(spacing: 10) {
            Button("Open") { navigator.open(.agent(id: agent.id)) }
                .buttonStyle(QuietButtonStyle())
            Button("Trust folder") {
                act { try await session.trust(agentId: agent.id) }
            }
            .buttonStyle(ClaudeButtonStyle())
        }
    }

    @ViewBuilder
    private func loginCard(_ agent: Agent) -> some View {
        header("person.crop.circle.badge.exclamationmark", "Claude login", agent.title, place(of: agent))
        Text(agent.envNames.contains("CLAUDE_CONFIG_DIR")
            ? "Its project's CLAUDE_CONFIG_DIR isn't logged in to Claude. Log in once in its terminal on the host and it carries on:"
            : "Claude isn't logged in for this agent. Log in once in its terminal on the host and it carries on:")
            .font(.subheadline)
            .foregroundStyle(Palette.ink2)
        CodeBox(text: "cos agent attach \(agent.id)")
        Button("Open agent") { navigator.open(.agent(id: agent.id)) }
            .buttonStyle(QuietButtonStyle())
    }

    private func decide(_ approval: Approval, _ decision: ApprovalDecision) {
        act { try await session.decide(approval, decision) }
    }

    private func act(_ work: @escaping () async throws -> Void) {
        busy = true
        Task {
            defer { busy = false }
            do { try await work() } catch { self.error = error.localizedDescription }
        }
    }
}
