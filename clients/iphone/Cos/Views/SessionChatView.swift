import CosKit
import SwiftUI

/// One session as a chat, as the desktop shows a session: its conversation
/// from the transcript (the chief's first prompt marked as the chief's,
/// without cos's line), tool calls as slim rows, what it waits on you for
/// as a message with its buttons, and a composer that types into it,
/// resuming it first when it has stopped. The terminal button is the
/// Transcript | Terminal switch: it opens the live terminal, whose ⋯ menu
/// comes back here.
struct SessionChatView: View {
    var session: HostSession
    let agentId: Int
    @State private var chat: Chat
    @FocusState private var composing: Bool
    @Environment(Navigator.self) private var navigator
    @State private var confirmStop = false
    @State private var error: String?

    init(session: HostSession, agentId: Int) {
        self.session = session
        self.agentId = agentId
        _chat = State(initialValue: Chat(agentId: agentId, session: session))
    }

    var body: some View {
        let agent = chat.agent
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    if chat.rows.isEmpty && !chat.isLoading {
                        EmptySession(agent: agent) { navigator.open(.terminal(agentId: agentId)) }
                    }
                    ForEach(chat.rows) { row in
                        ChatRowView(chat: chat, row: row)
                            .id(row.id)
                    }
                    ForEach(chat.agentsWaiting, id: \.agent.id) { entry in
                        WaitingMessage(session: session, agent: entry.agent, approval: entry.approval)
                            .id("waiting-\(entry.agent.id)")
                    }
                    if chat.isTyping {
                        TypingBubble(activity: chat.activity) { navigator.open(.terminal(agentId: agentId)) }
                            .id("typing")
                            .transition(.opacity)
                    }
                    if let error = chat.error, !chat.rows.isEmpty {
                        Text(error)
                            .font(.footnote)
                            .foregroundStyle(Palette.red)
                            .frame(maxWidth: .infinity)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 10)
                .padding(.top, 10)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .background(ChatWallpaper())
            .refreshable {
                await session.refresh()
                await chat.read()
            }
            .animation(.easeOut(duration: 0.2), value: chat.isTyping)
            .onChange(of: chat.rows.count + chat.agentsWaiting.count + (chat.isTyping ? 1 : 0)) { old, _ in
                if old == 0 {
                    Task { @MainActor in
                        await Task.yield()
                        proxy.scrollTo("bottom", anchor: .bottom)
                    }
                } else {
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if agent != nil {
                Composer(chat: chat, focused: $composing)
                    .background(Palette.page.opacity(0.97).ignoresSafeArea(edges: .bottom))
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(Palette.page, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Button {
                    navigator.open(.agent(id: agentId))
                } label: {
                    SessionTitle(title: agent?.title ?? "Session \(agentId)", subtitle: subtitle(agent), color: titleColor(agent))
                }
                .buttonStyle(.plain)
                .accessibilityHint("Shows the session's details")
            }
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button("Terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: agentId)) }
                menu(agent)
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .confirmationDialog("Stop this session?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop", role: .destructive) { perform { try await session.stopAgent(id: agentId) } }
        } message: {
            Text("Claude stops. A message from here resumes it.")
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
        .task { await chat.load() }
        .onChange(of: session.transcriptTicks[agentId]) {
            Task { await chat.read() }
        }
        .onChange(of: chat.replyingTo) { _, reply in
            if reply != nil { composing = true }
        }
    }

    private func menu(_ agent: Agent?) -> some View {
        Menu {
            Button("Session details", systemImage: "info.circle") { navigator.open(.agent(id: agentId)) }
            Button("Terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: agentId)) }
            if let projectId = chat.projectId, session.staffs[projectId] != nil {
                Button("Chat with the chief", systemImage: "bubble.left") { navigator.open(.chief(projectId: projectId)) }
            }
            if let agent {
                if agent.state.canResume {
                    Divider()
                    Button("Resume", systemImage: "play.circle") { perform { try await session.resume(agentId: agentId) } }
                } else if agent.state.isAlive {
                    Divider()
                    Button("Stop session", systemImage: "stop.circle", role: .destructive) { confirmStop = true }
                }
            }
        } label: {
            Label("More", systemImage: "ellipsis.circle")
        }
    }

    /// "typing…" while it works, else its state and where it works.
    private func subtitle(_ agent: Agent?) -> String {
        guard let agent else { return "Gone" }
        if chat.isTyping { return "typing…" }
        var parts = [agent.state.style.label.lowercased()]
        if agent.workspaceId != nil, session.workspace(id: agent.workspaceId) != nil { parts.append(session.workspaceName(agent.workspaceId)) }
        if agent.spawnedBy != nil { parts.append("★ chief's") }
        return parts.joined(separator: " · ")
    }

    private func titleColor(_ agent: Agent?) -> Color {
        guard let agent else { return Palette.ink3 }
        if chat.isTyping { return Palette.blue }
        return agent.state.needsYou ? Palette.claude : Palette.ink3
    }

    private func perform(_ work: @escaping @MainActor () async throws -> Void) {
        Task {
            do {
                try await work()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// A session chat's title: its name and what it's doing.
struct SessionTitle: View {
    var title: String
    var subtitle: String
    var color: Color

    var body: some View {
        VStack(spacing: 0) {
            Text(title)
                .font(.headline)
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(color)
                .lineLimit(1)
        }
        .frame(maxWidth: 240)
    }
}

/// Before its transcript has anything: an agent that hasn't reached Claude
/// yet (or a daemon without transcripts), with its terminal.
struct EmptySession: View {
    var agent: Agent?
    var openTerminal: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "text.bubble")
                .font(.system(size: 30))
                .foregroundStyle(Palette.ink3)
                .padding(.top, 60)
            Text(agent == nil ? "This session is gone" : "No conversation yet")
                .font(.headline)
                .foregroundStyle(Palette.ink)
            if agent != nil {
                Text("It shows here once Claude has written something. Its terminal is live now.")
                    .font(.subheadline)
                    .foregroundStyle(Palette.ink2)
                    .multilineTextAlignment(.center)
                Button("Open its terminal", action: openTerminal)
                    .buttonStyle(ClaudeButtonStyle())
                    .frame(maxWidth: 220)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
    }
}
