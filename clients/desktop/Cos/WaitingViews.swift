import CosKit
import SwiftUI

/// Everything blocked on you: permission requests and trust dialogs, oldest first.
struct WaitingOnYouView: View {
    let monitor: DaemonMonitor
    let window: WindowState

    private var trusting: [Agent] { monitor.agents.filter { $0.state == .needsTrust } }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Waiting on You").font(.largeTitle.weight(.semibold))
                if monitor.approvals.isEmpty && trusting.isEmpty {
                    ContentUnavailableView(
                        "Nothing is waiting on you",
                        systemImage: "checkmark.circle",
                        description: Text("Permission requests from agents show up here, and in the menu bar.")
                    )
                    .frame(maxWidth: .infinity, minHeight: 240)
                }
                ForEach(monitor.approvals) { approval in
                    WaitingCard(
                        agent: monitor.agent(id: approval.agentId),
                        workspace: monitor.workspace(id: approval.workspaceId ?? monitor.agent(id: approval.agentId)?.workspaceId ?? -1),
                        since: approval.createdAt,
                        headline: approval.headline,
                        detail: approval.detail,
                        window: window
                    ) {
                        ApprovalButtons(approval: approval, monitor: monitor, window: window)
                    }
                }
                ForEach(trusting) { agent in
                    WaitingCard(
                        agent: agent,
                        workspace: monitor.workspace(id: agent.workspaceId),
                        since: agent.lastActivityAt ?? agent.startedAt,
                        headline: "Claude asks whether to trust this folder",
                        detail: monitor.workspace(id: agent.workspaceId).map { ($0.path as NSString).abbreviatingWithTildeInPath } ?? "",
                        window: window
                    ) {
                        Button("Trust Folder") {
                            window.perform("Couldn't answer the trust dialog") { try await monitor.trustAgent(id: agent.id) }
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle("Waiting on You")
    }
}

private struct WaitingCard<Actions: View>: View {
    let agent: Agent?
    let workspace: Workspace?
    let since: Date
    let headline: String
    let detail: String
    let window: WindowState
    @ViewBuilder let actions: Actions

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    if let agent {
                        Button(agent.title) { window.selection = .agent(agent.id) }
                            .buttonStyle(.link)
                            .fontWeight(.medium)
                    }
                    if let workspace {
                        Text("in \(workspace.title)").foregroundStyle(.secondary)
                    }
                    Text(since, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                }
                .font(.callout)
                Text(headline).fontWeight(.semibold)
                Text(detail)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .lineLimit(3)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 12)
            actions
        }
        .padding(12)
        .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }
}

/// The menu bar panel's list of what's waiting, with inline answers.
struct MenuBarWaiting: View {
    let monitor: DaemonMonitor
    let window: WindowState
    let show: (SidebarItem) -> Void

    private let limit = 4

    var body: some View {
        let trusting = monitor.agents.filter { $0.state == .needsTrust }
        VStack(alignment: .leading, spacing: 8) {
            Button {
                show(.waiting)
            } label: {
                Label("Waiting on you (\(monitor.waitingCount))", systemImage: "exclamationmark.bubble.fill")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.orange)
            }
            .buttonStyle(.plain)

            ForEach(monitor.approvals.prefix(limit)) { approval in
                row(title: monitor.agent(id: approval.agentId)?.title ?? "Agent \(approval.agentId)", detail: approval.detail, agentId: approval.agentId) {
                    ApprovalButtons(approval: approval, monitor: monitor, window: window, compact: true)
                }
            }
            ForEach(trusting.prefix(max(0, limit - monitor.approvals.count))) { agent in
                row(title: agent.title, detail: "Trust this folder?", agentId: agent.id) {
                    Button("Trust") {
                        window.perform("Couldn't answer the trust dialog") { try await monitor.trustAgent(id: agent.id) }
                    }
                    .controlSize(.small)
                }
            }
            if monitor.waitingCount > limit {
                Button("and \(monitor.waitingCount - limit) more…") { show(.waiting) }
                    .buttonStyle(.link)
                    .font(.callout)
            }
        }
    }

    private func row(title: String, detail: String, agentId: Int, @ViewBuilder actions: () -> some View) -> some View {
        HStack(spacing: 8) {
            Button {
                show(.agent(agentId))
            } label: {
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).fontWeight(.medium).lineLimit(1)
                    Text(detail)
                        .font(.callout.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            actions()
        }
        .font(.callout)
    }
}
