import CosKit
import SwiftUI

/// A project's Chief of Staff as a chat, like a messaging app: your
/// messages in bubbles on the right, its replies (markdown) on the left,
/// tool calls as slim rows, news from its sessions as an "N updates" chip,
/// and what its sessions need from you as messages with buttons in them.
/// The ⋯ menu leads to its workspaces, sessions and runs. Live from
/// `agent.transcript`.
struct ChiefChatView: View {
    var session: HostSession
    @State private var chat: ChiefChat
    @FocusState private var composing: Bool
    @Environment(Navigator.self) private var navigator
    @State private var confirmStop = false
    @State private var error: String?

    init(session: HostSession, projectId: Int) {
        self.session = session
        _chat = State(initialValue: ChiefChat(projectId: projectId, session: session))
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    if chat.rows.isEmpty && !chat.isLoading {
                        EmptyChief(project: chat.project?.name ?? "this project", started: chat.staff?.hasStarted ?? false) { suggestion in
                            chat.draft = suggestion
                            composing = true
                        }
                    }
                    ForEach(chat.rows) { row in
                        ChatRowView(chat: chat, row: row)
                            .id(row.id)
                    }
                    ForEach(chat.agentsWaiting, id: \.agent.id) { entry in
                        WaitingMessage(session: session, agent: entry.agent, approval: entry.approval)
                            .id("waiting-\(entry.agent.id)")
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 10)
                .padding(.top, 10)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .background(ChatWallpaper())
            .refreshable { await chat.load() }
            .onChange(of: chat.rows.count + chat.agentsWaiting.count) { old, _ in
                if old == 0 {
                    // The first load jumps to the end, after layout.
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
            Composer(chat: chat, focused: $composing)
                .background(Palette.page.opacity(0.97).ignoresSafeArea(edges: .bottom))
        }
        .navigationBarTitleDisplayMode(.inline)
        // An opaque bar, as a chat app has: the title is unreadable over bubbles.
        .toolbarBackground(Palette.page, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Button {
                    navigator.open(.chiefInfo(projectId: chat.projectId))
                } label: {
                    ChatTitle(name: chat.project?.name ?? "Chief of Staff", subtitle: subtitle, live: (chat.staff?.workingAgents ?? 0) > 0)
                }
                .buttonStyle(.plain)
                .accessibilityHint("Shows its workspaces and sessions")
            }
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button("Chief's terminal", systemImage: "terminal") {
                    if let id = chat.chiefAgentId { navigator.open(.terminal(agentId: id)) }
                }
                .disabled(chat.chiefAgentId == nil)
                menu
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .confirmationDialog("Stop the Chief of Staff?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop", role: .destructive) { stopChief() }
        } message: {
            Text("The sessions it started keep running. Your next message starts it again.")
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
        .task { await chat.load() }
        .onAppear { markRead() }
        .onDisappear { markRead() }
        .onChange(of: session.transcriptTicks[chat.chiefAgentId ?? -1]) {
            markRead()
            Task { await chat.read() }
        }
        .onChange(of: chat.chiefAgentId) {
            Task { await chat.read() }
        }
    }

    /// The three dots: where its workspaces, sessions and runs are.
    private var menu: some View {
        let projectId = chat.projectId
        let sessions = session.agents(inProject: projectId)
        let working = sessions.filter { $0.state == .working || $0.state == .starting }.count
        let needYou = chat.agentsWaiting.count
        let runs = session.runs(inProject: projectId)
        return Menu {
            Button {
                navigator.open(.chiefInfo(projectId: projectId, tab: .workspaces))
            } label: {
                Label("Workspaces", systemImage: "arrow.triangle.branch")
                Text(count(chat.project?.workspaces.count ?? 0, "workspace", "workspaces"))
            }
            Button {
                navigator.open(.chiefInfo(projectId: projectId, tab: .sessions))
            } label: {
                Label("Sessions", systemImage: "person.2")
                Text([count(working, "working", "working"), needYou > 0 ? "\(needYou) need you" : nil].compactMap { $0 }.joined(separator: " · "))
            }
            Button {
                navigator.open(.chiefInfo(projectId: projectId, tab: .runs))
            } label: {
                Label("Runs", systemImage: "list.bullet.clipboard")
                Text(count(runs.filter { !$0.state.isFinished }.count, "active", "active"))
            }
            if let id = chat.chiefAgentId {
                Button("Chief's terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: id)) }
            }
            if let agent = session.agent(id: chat.chiefAgentId), agent.state.isAlive {
                Divider()
                Button("Stop Chief of Staff", systemImage: "stop.circle", role: .destructive) { confirmStop = true }
            }
        } label: {
            Label("More", systemImage: "ellipsis.circle")
        }
    }

    private var subtitle: String {
        let working = chat.staff?.workingAgents ?? 0
        if working > 0 { return working == 1 ? "1 session working…" : "\(working) sessions working…" }
        return chat.staff?.stateLabel ?? "Chief of Staff"
    }

    private func count(_ n: Int, _ one: String, _ many: String) -> String {
        "\(n) \(n == 1 ? one : many)"
    }

    private func markRead() {
        session.readMarks.markRead(projectId: chat.projectId)
    }

    private func stopChief() {
        guard let id = chat.chiefAgentId else { return }
        Task {
            do {
                try await session.stopAgent(id: id)
                session.refreshStaff()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// The chat's title: avatar, name, and what it's doing.
struct ChatTitle: View {
    var name: String
    var subtitle: String
    var live: Bool

    var body: some View {
        HStack(spacing: 8) {
            ProjectAvatar(name: name, size: 32)
            VStack(alignment: .leading, spacing: 0) {
                Text(name)
                    .font(.headline)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(live ? Palette.blue : Palette.ink3)
                    .lineLimit(1)
            }
        }
    }
}

/// Before the chief has said anything: what it is, and three suggestions
/// that fill the composer.
struct EmptyChief: View {
    var project: String
    var started: Bool
    var pick: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            ChiefStar(size: 28)
                .padding(.top, 40)
            Text("Chief of Staff for \(project)")
                .font(.title2.weight(.semibold))
            Text(started
                ? "It's ready. Tell it what to work on; it starts agents in workspaces, follows them and reports back."
                : "It starts with your first message. It creates workspaces, starts agents in them, follows them and reports back.")
                .font(.body)
                .foregroundStyle(Palette.ink2)
            VStack(spacing: 8) {
                ForEach(ChiefChat.suggestions, id: \.self) { suggestion in
                    Button {
                        pick(suggestion)
                    } label: {
                        HStack {
                            Text(suggestion)
                                .font(.subheadline)
                                .multilineTextAlignment(.leading)
                            Spacer()
                            Image(systemName: "arrow.up.left")
                                .font(.caption)
                                .foregroundStyle(Palette.ink3)
                        }
                        .foregroundStyle(Palette.ink)
                        .padding(12)
                        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(Palette.line, lineWidth: 0.5))
                    }
                }
            }
            .padding(.top, 6)
        }
    }
}
/// One row of the chat.
struct ChatRowView: View {
    var chat: ChiefChat
    var row: ChatRow
    @State private var expanded = false

    var body: some View {
        switch row {
        case .user(let message):
            Bubble(outgoing: true, time: message.tsMs) {
                Text(message.text)
                    .font(.body)
                    .foregroundStyle(Palette.ink)
                    .textSelection(.enabled)
            }
        case .pending(let pending):
            VStack(alignment: .trailing, spacing: 4) {
                Bubble(outgoing: true, time: nil) {
                    Text(pending.text)
                        .font(.body)
                        .foregroundStyle(Palette.ink)
                }
                pendingFooter(pending)
                    .padding(.trailing, 6)
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        case .reply(let message):
            Bubble(outgoing: false, time: message.tsMs) {
                MarkdownView(markdown: message.text)
                    .foregroundStyle(Palette.ink)
                    .textSelection(.enabled)
            }
        case .tool(let message):
            toolRow(message.tool ?? ToolCall(name: "tool"))
        case .spawned(let message, let tool):
            SpawnedCard(tool: tool, agent: chat.spawnedAgent(tool), time: message.tsMs)
        case .task(let message, let note):
            TaskCard(note: note, time: message.tsMs) { reply in
                chat.draft = reply
            }
        case .updates(let message, let count):
            updatesRow(message, count: count)
        }
    }

    @ViewBuilder
    private func pendingFooter(_ pending: PendingMessage) -> some View {
        switch pending.state {
        case .sending:
            Label("Sending…", systemImage: "clock").labelStyle(.tight).font(.caption).foregroundStyle(Palette.ink3)
        case .sent:
            Label("Sent · waiting for the chief", systemImage: "checkmark").labelStyle(.tight).font(.caption).foregroundStyle(Palette.ink3)
        case .failed(let message):
            VStack(alignment: .trailing, spacing: 4) {
                Text(message).font(.caption).foregroundStyle(Palette.red).multilineTextAlignment(.trailing)
                HStack(spacing: 12) {
                    Button("Discard") { chat.discard(pending) }
                    Button("Retry") { Task { await chat.retry(pending) } }.fontWeight(.semibold)
                }
                .font(.caption)
            }
        }
    }

    private func toolRow(_ tool: ToolCall) -> some View {
        Button {
            withAnimation(.snappy) { expanded.toggle() }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 7) {
                    Image(systemName: tool.isCosTool ? "star" : symbol(for: tool.name))
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(tool.isCosTool ? Palette.amber : Palette.ink3)
                    Text(tool.title)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(tool.isError ? Palette.red : Palette.ink2)
                    if !tool.summary.isEmpty, !expanded {
                        Text(tool.summary)
                            .font(.caption.monospaced())
                            .foregroundStyle(Palette.ink3)
                            .lineLimit(1)
                    }
                    if tool.isError {
                        Image(systemName: "exclamationmark.circle").font(.caption).foregroundStyle(Palette.red)
                    }
                }
                if expanded, !tool.summary.isEmpty {
                    CodeBox(text: tool.summary)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Palette.surface.opacity(0.75), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.trailing, 48)
    }

    private func updatesRow(_ message: TranscriptMessage, count: Int) -> some View {
        VStack(spacing: 8) {
            Button {
                withAnimation(.snappy) { expanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Text(count == 1 ? "1 update from sessions" : "\(count) updates from sessions")
                    Text("· \(shortAgo(message.tsMs))").foregroundStyle(Palette.ink3)
                    Image(systemName: "chevron.down")
                        .font(.caption2.weight(.bold))
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                }
                .font(.caption.weight(.medium))
                .foregroundStyle(Palette.ink2)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .shadow(color: .black.opacity(0.06), radius: 0.5, y: 1)
            }
            .buttonStyle(.plain)
            if expanded {
                MarkdownView(markdown: message.updateBody, font: .footnote)
                    .foregroundStyle(Palette.ink2)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 4)
    }

    private func symbol(for tool: String) -> String {
        switch tool {
        case "Bash": "terminal"
        case "Read": "doc.text"
        case "Edit", "Write", "MultiEdit": "pencil"
        case "Grep", "Glob": "magnifyingglass"
        case "WebFetch", "WebSearch": "globe"
        case "Task", "Agent": "person.2"
        default: "wrench.and.screwdriver"
        }
    }
}

/// A chat bubble: yours on the right in the outgoing colour, the chief's on
/// the left, with the time in its corner.
struct Bubble<Content: View>: View {
    var outgoing: Bool
    var time: Int64?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .trailing, spacing: 2) {
            content
            if let time, time > 0 {
                Text(Date(unixMilliseconds: time).formatted(date: .omitted, time: .shortened))
                    .font(.caption2)
                    .foregroundStyle(Palette.ink3)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 11)
        .padding(.top, 7)
        .padding(.bottom, 5)
        .background(outgoing ? Palette.outgoing : Palette.surface, in: shape)
        .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
        .frame(maxWidth: 520, alignment: outgoing ? .trailing : .leading)
        .padding(outgoing ? Edge.Set.leading : Edge.Set.trailing, 44)
        .frame(maxWidth: .infinity, alignment: outgoing ? .trailing : .leading)
    }

    private var shape: UnevenRoundedRectangle {
        outgoing
            ? UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16, bottomTrailingRadius: 16, topTrailingRadius: 4, style: .continuous)
            : UnevenRoundedRectangle(topLeadingRadius: 4, bottomLeadingRadius: 16, bottomTrailingRadius: 16, topTrailingRadius: 16, style: .continuous)
    }
}

/// A session the chief started that waits on you, as a message with its
/// buttons in it: Allow and Deny for an approval, Trust folder for the trust
/// dialog, Open for a login.
struct WaitingMessage: View {
    var session: HostSession
    var agent: Agent
    var approval: Approval?
    @Environment(Navigator.self) private var navigator
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text([agent.title, session.workspace(id: agent.workspaceId).map(\.displayName)].compactMap { $0 }.joined(separator: " · "))
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Palette.claude)
                    .lineLimit(1)
                if let approval {
                    Text(approval.toolName == "Bash" ? "Can I run this?" : "Can I use \(approval.toolName)?")
                        .font(.body)
                        .foregroundStyle(Palette.ink)
                    CodeBox(text: approval.detail, lineLimit: 6)
                    if let reason = approval.reason, !reason.isEmpty {
                        Text("cos asks you because \(reason)")
                            .font(.footnote)
                            .foregroundStyle(Palette.ink2)
                    }
                } else if agent.state == .needsTrust {
                    Text("Claude asks whether to trust this workspace's folder before it starts.")
                        .font(.body)
                        .foregroundStyle(Palette.ink)
                } else {
                    Text("It's on Claude's login screen. Log in on the host: `cos agent attach \(agent.id)`.")
                        .font(.body)
                        .foregroundStyle(Palette.ink)
                }
                if let error {
                    Text(error).font(.footnote).foregroundStyle(Palette.red)
                }
            }
            .padding(.horizontal, 11)
            .padding(.vertical, 9)
            if let approval {
                action("Allow", bold: true) { try await session.decide(approval, .allow) }
                action("Deny") { try await session.decide(approval, .deny) }
            } else if agent.state == .needsTrust {
                action("Trust folder", bold: true) { try await session.trust(agentId: agent.id) }
            }
            Divider()
            Button {
                navigator.open(.terminal(agentId: agent.id))
            } label: {
                Text("Open its terminal")
                    .font(.body)
                    .foregroundStyle(Palette.claude)
                    .frame(maxWidth: .infinity, minHeight: 42)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .background(Palette.surface, in: UnevenRoundedRectangle(topLeadingRadius: 4, bottomLeadingRadius: 16, bottomTrailingRadius: 16, topTrailingRadius: 16, style: .continuous))
        .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
        .disabled(busy)
        .padding(.trailing, 44)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func action(_ title: String, bold: Bool = false, _ work: @escaping @MainActor () async throws -> Void) -> some View {
        VStack(spacing: 0) {
            Divider()
            Button {
                busy = true
                Task {
                    defer { busy = false }
                    do {
                        try await work()
                        error = nil
                    } catch {
                        self.error = error.localizedDescription
                    }
                }
            } label: {
                Text(title)
                    .font(.body.weight(bold ? .semibold : .regular))
                    .foregroundStyle(Palette.claude)
                    .frame(maxWidth: .infinity, minHeight: 42)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// The message box: a capsule field and a round send button.
struct Composer: View {
    @Bindable var chat: ChiefChat
    var focused: FocusState<Bool>.Binding

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Message", text: $chat.draft, axis: .vertical)
                .lineLimit(1...6)
                .focused(focused)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 19, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 19, style: .continuous).stroke(Palette.line, lineWidth: 0.5))
            Button {
                Task { await chat.send() }
            } label: {
                Image(systemName: "arrow.up")
                    .font(.system(size: 16, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 36, height: 36)
                    .background(canSend ? Palette.claude : Palette.ink3.opacity(0.5), in: Circle())
            }
            .disabled(!canSend)
            .accessibilityLabel("Send")
        }
        .padding(.horizontal, 10)
        .padding(.top, 7)
        .padding(.bottom, 7)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 0.5) }
    }

    private var canSend: Bool {
        !chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
