import CosKit
import SwiftUI

/// Everything blocked on you: plans to approve, chiefs' questions,
/// permission requests routed to you and trust dialogs, oldest first.
/// Requests routed to a chief or allowed by a rule aren't here; they're in
/// their run's activity.
struct WaitingOnYouView: View {
    let monitor: DaemonMonitor
    let window: WindowState

    private var trusting: [Agent] { monitor.agents.filter { $0.state == .needsTrust } }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Waiting on You").font(.largeTitle.weight(.semibold))
                if monitor.waitingCount == 0 {
                    ContentUnavailableView(
                        "Nothing is waiting on you",
                        systemImage: "checkmark.circle",
                        description: Text("Plans to approve, chiefs' questions and permission requests show up here, and in the menu bar.")
                    )
                    .frame(maxWidth: .infinity, minHeight: 240)
                }
                ForEach(monitor.plansAwaitingApproval) { run in
                    PlanWaitingCard(run: run, monitor: monitor, window: window)
                }
                ForEach(monitor.questions) { question in
                    QuestionCard(question: question, monitor: monitor, window: window)
                }
                ForEach(monitor.approvalsForYou) { approval in
                    WaitingCard(
                        agent: monitor.agent(id: approval.agentId),
                        workspace: monitor.workspace(id: approval.workspaceId ?? monitor.agent(id: approval.agentId)?.workspaceId ?? -1),
                        since: approval.createdAt,
                        headline: approval.headline,
                        detail: approval.detail,
                        why: approval.routeExplanation,
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
                        why: nil,
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

/// A plan waiting for approval. It opens the run's page, where the plan is
/// shown in full, rather than approving blind.
private struct PlanWaitingCard: View {
    let run: Run
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Image(systemName: "list.bullet.clipboard.fill").foregroundStyle(.orange)
                    if let workspace = monitor.workspace(id: run.workspaceId) {
                        Text("in \(workspace.title)").foregroundStyle(.secondary)
                    }
                    Text(run.updatedAt, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                }
                .font(.callout)
                Text("The chief's plan is ready: \(Format.count(run.counts?.total ?? 0, "task"))").fontWeight(.semibold)
                Text(run.goal)
                    .lineLimit(2)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            Button("Review Plan") { window.selection = .run(run.id) }
                .buttonStyle(.borderedProminent)
        }
        .padding(12)
        .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }
}

private struct WaitingCard<Actions: View>: View {
    let agent: Agent?
    let workspace: Workspace?
    let since: Date
    let headline: String
    let detail: String
    let why: String?
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
                        if agent.role != .user {
                            Text(agent.role.rawValue).foregroundStyle(.secondary)
                        }
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
                if let why {
                    Text(why)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
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

    private enum Item: Identifiable {
        case plan(Run)
        case question(Question)
        case approval(Approval)
        case trust(Agent)

        var id: String {
            switch self {
            case .plan(let run): "p\(run.id)"
            case .question(let question): "q\(question.id)"
            case .approval(let approval): "a\(approval.id)"
            case .trust(let agent): "t\(agent.id)"
            }
        }
    }

    private var items: [Item] {
        monitor.plansAwaitingApproval.map(Item.plan)
            + monitor.questions.map(Item.question)
            + monitor.approvalsForYou.map(Item.approval)
            + monitor.agents.filter { $0.state == .needsTrust }.map(Item.trust)
    }

    var body: some View {
        let items = items
        VStack(alignment: .leading, spacing: 8) {
            Button {
                show(.waiting)
            } label: {
                Label("Waiting on you (\(monitor.waitingCount))", systemImage: "exclamationmark.bubble.fill")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.orange)
            }
            .buttonStyle(.plain)

            ForEach(items.prefix(limit)) { item in
                switch item {
                case .plan(let run):
                    row(title: "Plan ready: \(Format.count(run.counts?.total ?? 0, "task"))", detail: run.goal, monospaced: false, open: .run(run.id)) {
                        Button("Review") { show(.run(run.id)) }
                            .controlSize(.small)
                    }
                case .question(let question):
                    row(title: "The chief asks", detail: question.question, monospaced: false, open: question.runId.map(SidebarItem.run) ?? .waiting) {
                        Button("Answer") { show(question.runId.map(SidebarItem.run) ?? .waiting) }
                            .controlSize(.small)
                    }
                case .approval(let approval):
                    row(title: monitor.agent(id: approval.agentId)?.title ?? "Agent \(approval.agentId)", detail: approval.detail, open: .agent(approval.agentId)) {
                        ApprovalButtons(approval: approval, monitor: monitor, window: window, compact: true)
                    }
                case .trust(let agent):
                    row(title: agent.title, detail: "Trust this folder?", open: .agent(agent.id)) {
                        Button("Trust") {
                            window.perform("Couldn't answer the trust dialog") { try await monitor.trustAgent(id: agent.id) }
                        }
                        .controlSize(.small)
                    }
                }
            }
            if items.count > limit {
                Button("and \(items.count - limit) more…") { show(.waiting) }
                    .buttonStyle(.link)
                    .font(.callout)
            }
        }
    }

    private func row(title: String, detail: String, monospaced: Bool = true, open: SidebarItem, @ViewBuilder actions: () -> some View) -> some View {
        HStack(spacing: 8) {
            Button {
                show(open)
            } label: {
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).fontWeight(.medium).lineLimit(1)
                    Text(detail)
                        .font(monospaced ? .callout.monospaced() : .callout)
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
