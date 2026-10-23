import CosKit
import SwiftUI

/// One agent: its state and last message, what it waits on, a message box,
/// its live terminal, and a read-only snapshot of its screen.
struct AgentView: View {
    var session: HostSession
    var agentId: Int
    @State private var detail: AgentDetail
    @State private var showScreen = false

    init(session: HostSession, agentId: Int) {
        self.session = session
        self.agentId = agentId
        _detail = State(initialValue: AgentDetail(agentId: agentId, session: session))
    }

    var body: some View {
        List {
            if let agent = detail.agent {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        StateBadge(style: agent.state.style)
                        Text(agent.title)
                            .font(.title3.weight(.semibold))
                        Text(facts(agent))
                            .font(.footnote)
                            .foregroundStyle(Palette.ink3)
                        if agent.spawnedBy != nil {
                            Label("Started by the Chief of Staff", systemImage: "star.fill")
                                .labelStyle(.tight)
                                .font(.footnote)
                                .foregroundStyle(Palette.amber)
                        }
                    }
                    .padding(.vertical, 4)
                }

                if let approval = detail.approval, approval.isForYou {
                    Section("Waiting for your approval") {
                        VStack(alignment: .leading, spacing: 10) {
                            Text(approval.toolName == "Bash" ? "Claude wants to run" : "Claude wants to use \(approval.toolName)")
                                .font(.subheadline)
                                .foregroundStyle(Palette.ink2)
                            CodeBox(text: approval.detail)
                            HStack(spacing: 10) {
                                Button("Deny") { Task { await detail.perform { try await $0.decide(approval, .deny) } } }
                                    .buttonStyle(QuietButtonStyle())
                                Button("Allow") { Task { await detail.perform { try await $0.decide(approval, .allow) } } }
                                    .buttonStyle(ClaudeButtonStyle())
                            }
                        }
                        .padding(.vertical, 4)
                    }
                } else if let approval = detail.approval, approval.isWithChief {
                    Section("With the run's chief") {
                        Text(approval.detail).font(.footnote.monospaced()).foregroundStyle(Palette.ink2)
                    }
                }

                if agent.state == .needsTrust {
                    Section("Trust dialog") {
                        Text("Claude asks whether to trust this folder before it starts.")
                            .font(.subheadline)
                        Button("Trust folder") { Task { await detail.perform { try await $0.trust(agentId: agentId) } } }
                            .buttonStyle(ClaudeButtonStyle())
                    }
                }

                if agent.state == .needsLogin {
                    Section("Claude login") {
                        Text("This agent's Claude config isn't logged in. Log in once in its terminal on the host:")
                            .font(.subheadline)
                        CodeBox(text: "cos agent attach \(agent.id)")
                        if !agent.envNames.isEmpty {
                            Text("Uses " + agent.envNames.joined(separator: ", "))
                                .font(.caption)
                                .foregroundStyle(Palette.ink3)
                        }
                    }
                }

                if let message = agent.lastMessage, !message.isEmpty {
                    Section("Last message") {
                        MarkdownView(markdown: message, font: .subheadline)
                            .textSelection(.enabled)
                            .padding(.vertical, 4)
                    }
                }

                Section {
                    NavigationLink(value: Route.terminal(agentId: agentId)) {
                        Label("Open its terminal", systemImage: "apple.terminal")
                    }
                    DisclosureGroup(isExpanded: $showScreen) {
                        ScreenView(detail: detail)
                    } label: {
                        Label("Screen", systemImage: "terminal")
                    }
                } footer: {
                    Text("The terminal is live: you can type into it. The screen is a plain-text snapshot.")
                }

                if agent.state.isAlive {
                    Section {
                        Button("Stop agent", role: .destructive) { Task { await detail.perform { try await $0.stopAgent(id: agentId) } } }
                    }
                }
            } else {
                ContentUnavailableView("Agent \(agentId) is gone", systemImage: "person.crop.circle.badge.xmark")
                    .listRowBackground(Color.clear)
            }

            if let error = detail.error {
                Section { Text(error).font(.footnote).foregroundStyle(Palette.red) }
            }
        }
        .pageBackground()
        .navigationTitle("Agent \(agentId)")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable {
            await session.refresh()
            if showScreen { await detail.loadScreen() }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let agent = detail.agent, agent.role != .staff {
                AgentComposer(detail: detail, agent: agent)
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .onChange(of: showScreen) { _, show in
            if show { Task { await detail.loadScreen() } }
        }
    }

    private func facts(_ agent: Agent) -> String {
        var parts: [String] = []
        if let project = session.project(ofWorkspace: agent.workspaceId) { parts.append(project.name) }
        if agent.workspaceId != nil, session.workspace(id: agent.workspaceId) != nil { parts.append(session.workspaceName(agent.workspaceId)) }
        if agent.role != .user { parts.append(agent.role.rawValue) }
        if let model = agent.model { parts.append(model) }
        parts.append("started \(shortAgo(agent.startedMs))")
        return parts.joined(separator: " · ")
    }
}

struct ScreenView: View {
    var detail: AgentDetail

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let screen = detail.screen {
                ScrollView(.horizontal) {
                    Text(screen.text)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Palette.ink)
                        .textSelection(.enabled)
                        .fixedSize()
                        .padding(10)
                }
                .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                Text("\(screen.cols)×\(screen.rows)")
                    .font(.caption2.monospaced())
                    .foregroundStyle(Palette.ink3)
            } else if detail.isLoadingScreen {
                ProgressView().frame(maxWidth: .infinity)
            }
            Button("Refresh screen", systemImage: "arrow.clockwise") { Task { await detail.loadScreen() } }
                .font(.footnote)
                .disabled(detail.isLoadingScreen)
        }
        .padding(.vertical, 4)
    }
}

/// "Send message": typed into the agent's terminal with Return.
struct AgentComposer: View {
    @Bindable var detail: AgentDetail
    var agent: Agent

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField(agent.state.canResume ? "Resume it with a message" : "Send a message", text: $detail.draft, axis: .vertical)
                .lineLimit(1...5)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
            Button {
                Task { await detail.send() }
            } label: {
                Image(systemName: "arrow.up")
                    .font(.system(size: 15, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 32, height: 32)
                    .background(canSend ? Palette.claude : Palette.ink3.opacity(0.5), in: Circle())
            }
            .disabled(!canSend)
            .padding(.trailing, 6)
            .padding(.bottom, 5)
            .accessibilityLabel("Send message")
        }
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).stroke(Palette.line, lineWidth: 1))
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(Palette.page.opacity(0.96).ignoresSafeArea(edges: .bottom))
    }

    private var canSend: Bool {
        !detail.isSending && !detail.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
