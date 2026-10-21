import CosKit
import SwiftUI

// MARK: State

extension AgentState {
    var title: String {
        switch self {
        case .starting: "Starting"
        case .needsTrust: "Needs trust"
        case .working: "Working"
        case .waitingApproval: "Needs approval"
        case .idle: "Idle"
        case .exited: "Exited"
        case .interrupted: "Interrupted"
        default: rawValue.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    var color: Color {
        switch self {
        case .working: .blue
        case .starting: .secondary
        case .needsTrust, .waitingApproval: .orange
        case .idle: .green
        case .interrupted: .yellow
        default: .secondary
        }
    }

    /// Hollow dot for agents with no process.
    var isHollow: Bool { !isAlive }
}

/// A small dot for lists and the sidebar.
struct AgentStateDot: View {
    let state: AgentState

    var body: some View {
        Group {
            if state.isHollow {
                Circle().strokeBorder(state.color, lineWidth: 1.5)
            } else {
                Circle().fill(state.color)
            }
        }
        .frame(width: 8, height: 8)
        .help(state.title)
        .accessibilityLabel(state.title)
    }
}

/// The state as a capsule: a spinner while working, an attention mark when
/// it needs you.
struct AgentStateBadge: View {
    let state: AgentState

    var body: some View {
        HStack(spacing: 5) {
            switch state {
            case .working, .starting:
                ProgressView().controlSize(.mini)
            case .waitingApproval, .needsTrust:
                Image(systemName: "exclamationmark.circle.fill")
            default:
                AgentStateDot(state: state)
            }
            Text(state.title)
        }
        .font(.caption.weight(.medium))
        .foregroundStyle(state.needsYou ? Color.orange : Color.primary.opacity(0.8))
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(state.color.opacity(state.needsYou ? 0.18 : 0.12), in: Capsule())
    }
}

// MARK: Workspace page

/// The workspace's agents, with a button to start one.
struct AgentsSection: View {
    let workspace: Workspace
    let monitor: DaemonMonitor
    let window: WindowState

    /// Yours; a run's chief, workers and reviewers are on the run's page.
    private var agents: [Agent] { monitor.agents(inWorkspace: workspace.id).filter { $0.runId == nil } }
    private var canStart: Bool { monitor.connection == .running && monitor.agentsSupported }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Agents").font(.headline)
                Spacer()
                Button("New Agent…", systemImage: "plus") { window.newAgentIn = workspace }
                    .disabled(!canStart)
            }
            if !monitor.agentsSupported {
                placeholder(
                    "This cos daemon doesn't run agents yet",
                    "Update cos and restart the daemon to start agents here."
                )
            } else if agents.isEmpty {
                placeholder(
                    "No agents yet",
                    "Start one with a prompt. Agents run Claude Code in this workspace's folder, and you can watch or type into their terminal here."
                )
            } else {
                VStack(spacing: 6) {
                    ForEach(agents) { agent in
                        AgentRow(agent: agent, approval: monitor.pendingApproval(forAgent: agent.id)) {
                            window.selection = .agent(agent.id)
                        }
                    }
                }
            }
            if let error = monitor.agentsError, monitor.connection == .running {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        }
    }

    private func placeholder(_ title: String, _ detail: String) -> some View {
        VStack(spacing: 8) {
            Image(systemName: "person.2")
                .font(.title)
                .foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, minHeight: 140)
        .padding()
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10))
    }
}

/// One agent in a list: state, title, role and the last thing it said.
struct AgentRow: View {
    let agent: Agent
    let approval: Approval?
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(agent.title)
                        .font(.body.weight(.medium))
                        .lineLimit(1)
                    AgentStateBadge(state: agent.state)
                    Spacer()
                    Text(agent.details)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(agent.lastActivityAt ?? agent.startedAt, format: .relative(presentation: .named))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let approval {
                    Text("Wants to run: \(approval.detail)")
                        .font(.callout.monospaced())
                        .foregroundStyle(.orange)
                        .lineLimit(1)
                        .truncationMode(.middle)
                } else if let message = agent.lastMessage, !message.isEmpty {
                    Text(message)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.tail)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

extension Agent {
    /// `user · sonnet`.
    var details: String {
        [role.rawValue, model].compactMap { $0 }.joined(separator: " · ")
    }
}

// MARK: Terminal page

/// One agent: a header with its state and actions, banners when it needs
/// you, and its live terminal filling the rest.
struct AgentPage: View {
    let agentId: Int
    let monitor: DaemonMonitor
    let window: WindowState
    @State private var session: TerminalSession

    init(agentId: Int, monitor: DaemonMonitor, window: WindowState) {
        self.agentId = agentId
        self.monitor = monitor
        self.window = window
        _session = State(initialValue: TerminalSession(agentId: agentId, client: monitor.client))
    }

    var body: some View {
        if let agent = monitor.agent(id: agentId) {
            VStack(spacing: 0) {
                AgentHeader(agent: agent, workspace: monitor.workspace(id: agent.workspaceId), monitor: monitor, window: window)
                Divider()
                if agent.state == .needsTrust {
                    TrustBanner(agent: agent, monitor: monitor, window: window)
                }
                if let approval = monitor.pendingApproval(forAgent: agent.id) {
                    ApprovalBanner(approval: approval, monitor: monitor, window: window)
                }
                ZStack(alignment: .bottom) {
                    AgentTerminalView(session: session)
                    TerminalStatusBar(agent: agent, session: session, monitor: monitor, window: window)
                }
            }
            .navigationTitle(agent.title)
            // A resume gives the agent a new process: open a fresh stream.
            .onChange(of: agent.generation) { session.restart() }
        } else {
            ContentUnavailableView("No such agent", systemImage: "terminal", description: Text("It may have been removed."))
        }
    }
}

private struct AgentHeader: View {
    let agent: Agent
    let workspace: Workspace?
    let monitor: DaemonMonitor
    let window: WindowState

    private var canChange: Bool { monitor.connection == .running }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Text(agent.title)
                        .font(.title2.weight(.semibold))
                        .lineLimit(1)
                    AgentStateBadge(state: agent.state)
                }
                HStack(spacing: 6) {
                    if let workspace {
                        Button(workspace.title) { window.selection = .workspace(workspace.id) }
                            .buttonStyle(.link)
                        Text("·")
                    }
                    if let runId = agent.runNumber {
                        Button(monitor.run(id: runId).map { "Run: \($0.goal)" } ?? "Run \(runId)") { window.selection = .run(runId) }
                            .buttonStyle(.link)
                            .lineLimit(1)
                        Text("·")
                    }
                    Text(agent.details)
                    if let sessionId = agent.sessionId {
                        Text("·")
                        Text(sessionId)
                            .monospaced()
                            .textSelection(.enabled)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help("Claude session id. Resume it elsewhere with claude --resume \(sessionId)")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            if agent.state.isAlive {
                Button("Stop", systemImage: "stop.fill") {
                    window.perform("Couldn't stop the agent") { try await monitor.stopAgent(id: agent.id) }
                }
                .help("Sends /exit; the process is killed if it hasn't ended after 10 seconds.")
                .disabled(!canChange)
            } else {
                Button("Resume", systemImage: "play.fill") {
                    window.perform("Couldn't resume the agent") { try await monitor.resumeAgent(id: agent.id) }
                }
                .disabled(!canChange)
            }
            Button("Remove…", systemImage: "trash") { window.removingAgent = agent }
                .disabled(!canChange || agent.state.isAlive)
                .help(agent.state.isAlive ? "Stop the agent before removing it." : "Forget this agent.")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}

/// "Claude asks whether to trust this folder" with a Trust button.
private struct TrustBanner: View {
    let agent: Agent
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        Banner(systemImage: "lock.shield", tint: .orange) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Claude asks whether to trust this folder").fontWeight(.semibold)
                Text("It won't start working until the folder is trusted. Trusting it lets Claude read and run code there.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } actions: {
            Button("Trust Folder") {
                window.perform("Couldn't answer the trust dialog") { try await monitor.trustAgent(id: agent.id) }
            }
            .buttonStyle(.borderedProminent)
            .tint(.orange)
        }
    }
}

/// "Claude wants to run: npm install …" with Allow and Deny.
struct ApprovalBanner: View {
    let approval: Approval
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        // Routed to the chief: it's deciding, so this isn't a call to action.
        let chief = approval.isWithChief
        Banner(systemImage: chief ? "person.badge.clock" : "hand.raised.fill", tint: chief ? .indigo : .orange) {
            VStack(alignment: .leading, spacing: 3) {
                Text(chief ? "The chief is deciding whether Claude may \(approval.toolName == "Bash" ? "run" : "use \(approval.toolName)"):" : approval.headline)
                    .fontWeight(.semibold)
                Text(approval.detail)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .lineLimit(3)
                    .truncationMode(.middle)
                if let why = approval.routeExplanation {
                    Text(why)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Text(chief ? "You can still answer first, here or in the terminal below." : "You can also answer in the terminal below.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } actions: {
            ApprovalButtons(approval: approval, monitor: monitor, window: window)
        }
    }
}

/// Deny and Allow. A request already answered in the terminal just goes away.
struct ApprovalButtons: View {
    let approval: Approval
    let monitor: DaemonMonitor
    let window: WindowState
    var compact = false

    var body: some View {
        HStack(spacing: 6) {
            Button("Deny") { decide(.deny) }
                .controlSize(compact ? .small : .regular)
            Button("Allow") { decide(.allow) }
                .buttonStyle(.borderedProminent)
                .controlSize(compact ? .small : .regular)
        }
    }

    private func decide(_ decision: ApprovalDecision) {
        window.perform("Couldn't answer the request") {
            do {
                try await monitor.decide(approval, decision)
            } catch CosError.conflict {
                // Answered in the terminal meanwhile; nothing left to do.
            }
        }
    }
}

extension Approval {
    /// "Claude wants to run:" for commands, "Claude wants to use Write:" otherwise.
    var headline: String {
        toolName == "Bash" ? "Claude wants to run:" : "Claude wants to use \(toolName):"
    }
}

private struct Banner<Content: View, Actions: View>: View {
    let systemImage: String
    let tint: Color
    @ViewBuilder let content: Content
    @ViewBuilder let actions: Actions

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: systemImage)
                .font(.title2)
                .foregroundStyle(tint)
            content
            Spacer(minLength: 12)
            actions
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(tint.opacity(0.12))
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// Over the bottom of the terminal: why it isn't live, and Resume.
private struct TerminalStatusBar: View {
    let agent: Agent
    let session: TerminalSession
    let monitor: DaemonMonitor
    let window: WindowState

    var body: some View {
        if let message {
            HStack(spacing: 10) {
                if showsSpinner { ProgressView().controlSize(.small) }
                Text(message)
                    .font(.callout.weight(.medium))
                Spacer()
                if agent.state.canResume || isExited {
                    Button("Resume", systemImage: "play.fill") {
                        window.perform("Couldn't resume the agent") { try await monitor.resumeAgent(id: agent.id) }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(monitor.connection != .running || agent.state.isAlive)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .padding(12)
            .frame(maxWidth: 560)
        }
    }

    private var isExited: Bool {
        if case .exited = session.phase { true } else { false }
    }

    private var showsSpinner: Bool {
        switch session.phase {
        case .connecting, .reconnecting: true
        default: false
        }
    }

    private var message: String? {
        if agent.state == .interrupted {
            return "Interrupted: cos stopped while this agent was running."
        }
        switch session.phase {
        case .live:
            return nil
        case .connecting:
            return "Connecting to the terminal…"
        case .reconnecting:
            return "Reconnecting to the terminal…"
        case .exited(let code):
            let code = code ?? agent.exitCode
            return code.map { "Exited (code \($0))" } ?? "Exited (killed)"
        case .gone(let message):
            return message
        }
    }
}
