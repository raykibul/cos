import CosKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// A project's Chief of Staff as a chat, like a messaging app: your
/// messages in bubbles on the right, its replies (markdown) on the left,
/// tool calls as slim rows, news from its sessions as an "N updates" chip,
/// and what its sessions need from you as messages with buttons in them.
/// The ⋯ menu leads to its workspaces and sessions. Live from
/// `agent.transcript`.
struct ChiefChatView: View {
    var session: HostSession
    let projectId: Int
    @State private var chat: Chat
    @FocusState private var composing: Bool
    @Environment(Navigator.self) private var navigator
    @State private var confirmStop = false
    @State private var confirmNewSession = false
    @State private var error: String?

    init(session: HostSession, projectId: Int) {
        self.session = session
        self.projectId = projectId
        _chat = State(initialValue: Chat(projectId: projectId, session: session))
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    if chat.rows.isEmpty && !chat.isLoading {
                        EmptyChief(
                            project: chat.project?.name ?? "this project", started: chat.staff?.hasStarted ?? false,
                            briefed: !(chat.staff?.brief.isEmpty ?? true) || !(chat.staff?.identity.isEmpty ?? true)
                        ) { suggestion in
                            chat.draft = suggestion
                            composing = true
                        } openMemory: {
                            navigator.open(.chiefMemory(projectId: projectId))
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
                    if chat.isTyping {
                        TypingBubble(activity: chat.activity) {
                            if let id = chat.agentId { navigator.open(.terminal(agentId: id)) }
                        }
                        .id("typing")
                        .transition(.opacity)
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
            .animation(.easeOut(duration: 0.2), value: chat.isTyping)
            .onChange(of: chat.rows.count + chat.agentsWaiting.count + (chat.isTyping ? 1 : 0)) { old, _ in
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
                    navigator.open(.chiefInfo(projectId: projectId))
                } label: {
                    ChatTitle(name: chat.project?.name ?? "Chief of Staff", subtitle: subtitle, live: chat.isTyping || (chat.staff?.workingAgents ?? 0) > 0)
                }
                .buttonStyle(.plain)
                .accessibilityHint("Shows its workspaces and sessions")
            }
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button("Chief's terminal", systemImage: "terminal") {
                    if let id = chat.agentId { navigator.open(.terminal(agentId: id)) }
                }
                .disabled(chat.agentId == nil)
                menu
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .confirmationDialog("Stop the Chief of Staff?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop", role: .destructive) { stopChief() }
        } message: {
            Text("The sessions it started keep running. Your next message starts it again.")
        }
        .confirmationDialog("Start a new session?", isPresented: $confirmNewSession, titleVisibility: .visible) {
            Button("New session") { newSession() }
            Button("Identity & memory…") { navigator.open(.chiefMemory(projectId: projectId)) }
        } message: {
            Text("The chief drops this conversation and starts afresh with its identity, brief and memory notes. The sessions it started stay its own.")
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
        .task { await chat.load() }
        .onAppear { markRead() }
        .onDisappear { markRead() }
        .onChange(of: session.transcriptTicks[chat.agentId ?? -1]) {
            markRead()
            Task { await chat.read() }
        }
        .onChange(of: chat.agentId) {
            Task { await chat.read() }
        }
        .onChange(of: chat.replyingTo) { _, reply in
            if reply != nil { composing = true }
        }
    }

    /// The three dots: where its workspaces and sessions are.
    private var menu: some View {
        let sessions = session.agents(inProject: projectId)
        let working = sessions.filter { $0.state == .working || $0.state == .starting }.count
        let needYou = chat.agentsWaiting.count
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
            Button("Identity & memory", systemImage: "book.closed") { navigator.open(.chiefMemory(projectId: projectId)) }
            if let id = chat.agentId {
                Button("Chief's terminal", systemImage: "terminal") { navigator.open(.terminal(agentId: id)) }
            }
            if chat.staff?.hasStarted == true {
                Divider()
                Button("New session", systemImage: "plus.bubble") { confirmNewSession = true }
            }
            if let agent = session.agent(id: chat.agentId), agent.state.isAlive {
                Button("Stop Chief of Staff", systemImage: "stop.circle", role: .destructive) { confirmStop = true }
            }
        } label: {
            Label("More", systemImage: "ellipsis.circle")
        }
    }

    /// "typing…" while the chief works, as a chat app shows it, and how
    /// many of its sessions work.
    private var subtitle: String {
        let working = chat.staff?.workingAgents ?? 0
        let sessions = working == 1 ? "1 session working" : "\(working) sessions working"
        if chat.isTyping { return working > 0 ? "typing… · \(sessions)" : "typing…" }
        if working > 0 { return sessions + "…" }
        return chat.staff?.stateLabel ?? "Chief of Staff"
    }

    private func count(_ n: Int, _ one: String, _ many: String) -> String {
        "\(n) \(n == 1 ? one : many)"
    }

    private func markRead() {
        session.readMarks.markRead(projectId: projectId)
    }

    private func newSession() {
        Task {
            do {
                let staff = try await session.service.newStaffSession(projectId: projectId)
                session.setStaff(staff)
                session.refreshStaff()
                await chat.read()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func stopChief() {
        guard let id = chat.agentId else { return }
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
    /// It has a brief or an identity already.
    var briefed: Bool
    var pick: (String) -> Void
    var openMemory: () -> Void

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
            if !started, !briefed {
                Button(action: openMemory) {
                    HStack(alignment: .top, spacing: 10) {
                        CardIcon(symbol: "book.closed", color: Palette.amber)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Give it a brief first")
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(Palette.ink)
                            Text("Who it is, what it's working on, and what it should remember. It reads them when it starts.")
                                .font(.footnote)
                                .foregroundStyle(Palette.ink2)
                                .multilineTextAlignment(.leading)
                        }
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(Palette.ink3)
                            .padding(.top, 10)
                    }
                    .padding(12)
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(Palette.line, lineWidth: 0.5))
                }
                .buttonStyle(.plain)
            }
            VStack(spacing: 8) {
                ForEach(Chat.suggestions, id: \.self) { suggestion in
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
    var chat: Chat
    var row: ChatRow
    @State private var expanded = false
    @State private var info: TranscriptMessage?

    var body: some View {
        switch row {
        case .user(let message, let receipt):
            let parsed = ChatReply.parse(message.text)
            let shown = parsed?.body ?? message.text
            Bubble(outgoing: true, time: message.tsMs, receipt: receipt) {
                if let parsed { QuotedReply(reply: parsed.reply, theirName: chat.theirName) }
                SentFiles(chat: chat, files: message.attachments)
                if !shown.isEmpty {
                    Text(shown)
                        .font(.body)
                        .foregroundStyle(Palette.ink)
                }
            }
            .contextMenu {
                Button("Reply", systemImage: "arrowshape.turn.up.left") { chat.reply(to: message) }
                Button("Info", systemImage: "info.circle") { info = message }
                Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = shown }
            }
            .sheet(item: $info) { message in
                MessageInfo(message: message, times: chat.receiptTimes(for: message))
            }
        case .fromChief(let message, let prompt):
            Bubble(outgoing: true, time: message.tsMs) {
                Label("From the Chief of Staff", systemImage: "star.fill")
                    .labelStyle(.tight)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Palette.amber)
                    .frame(maxWidth: .infinity, alignment: .leading)
                SentFiles(chat: chat, files: message.attachments)
                MarkdownView(markdown: prompt)
                    .foregroundStyle(Palette.ink)
                    .textSelection(.enabled)
            }
            .contextMenu {
                Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = prompt }
            }
        case .pending(let pending):
            VStack(alignment: .trailing, spacing: 4) {
                let parsed = ChatReply.parse(pending.text)
                let shown = parsed?.body ?? pending.text
                Bubble(outgoing: true, time: pending.createdMs, receipt: pending.receipt) {
                    if let parsed { QuotedReply(reply: parsed.reply, theirName: chat.theirName) }
                    SentFiles(chat: chat, files: pending.attachments)
                    if !shown.isEmpty {
                        Text(shown)
                            .font(.body)
                            .foregroundStyle(Palette.ink)
                    }
                }
                if case .failed(let message) = pending.state {
                    failedFooter(pending, message)
                        .padding(.trailing, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        case .reply(let message):
            Bubble(outgoing: false, time: message.tsMs) {
                MarkdownView(markdown: message.text)
                    .foregroundStyle(Palette.ink)
                    .textSelection(.enabled)
            }
            .contextMenu {
                Button("Reply", systemImage: "arrowshape.turn.up.left") { chat.reply(to: message) }
                Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.text }
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

    /// Why it failed, with Retry and Discard. Sending and sent are ticks.
    private func failedFooter(_ pending: PendingMessage, _ message: String) -> some View {
        VStack(alignment: .trailing, spacing: 4) {
            Text(message).font(.caption).foregroundStyle(Palette.red).multilineTextAlignment(.trailing)
            HStack(spacing: 12) {
                Button("Discard") { chat.discard(pending) }
                Button("Retry") { Task { await chat.retry(pending) } }.fontWeight(.semibold)
            }
            .font(.caption)
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
    /// The ticks on what you sent.
    var receipt: Receipt?
    @ViewBuilder var content: Content

    init(outgoing: Bool, time: Int64?, receipt: Receipt? = nil, @ViewBuilder content: () -> Content) {
        self.outgoing = outgoing
        self.time = time
        self.receipt = receipt
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .trailing, spacing: 2) {
            content
            if (time ?? 0) > 0 || receipt != nil {
                HStack(spacing: 3) {
                    if let time, time > 0 {
                        Text(Date(unixMilliseconds: time).formatted(date: .omitted, time: .shortened))
                    }
                    if let receipt { ReceiptTicks(receipt: receipt) }
                }
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

/// The message box: a paperclip (Photos or Files), the files you picked as
/// chips, a capsule field and a round send button. Files upload as soon as
/// you pick them; Send waits until they're up. For a session, a line over
/// it says why it can't send while a prompt would catch the text.
struct Composer: View {
    @Bindable var chat: Chat
    var focused: FocusState<Bool>.Binding
    @State private var choosingPhotos = false
    @State private var choosingFiles = false
    @State private var photos: [PhotosPickerItem] = []

    var body: some View {
        VStack(spacing: 6) {
            if let blocked = chat.blocked {
                Label(blocked, systemImage: "hand.raised")
                    .labelStyle(.tight)
                    .font(.footnote)
                    .foregroundStyle(Palette.claude)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14)
            }
            if let reply = chat.replyingTo, let label = chat.replyLabel {
                ReplyStrip(label: label, text: ChatReply.preview(reply.text)) { chat.replyingTo = nil }
            }
            if !chat.files.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(chat.files) { file in
                            DraftChip(file: file) { chat.removeFile(file.id) }
                        }
                    }
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                }
                .accessibilityLabel("Attached files")
            }
            HStack(alignment: .bottom, spacing: 8) {
                Menu {
                    Button("Photos", systemImage: "photo.on.rectangle") { choosingPhotos = true }
                    Button("Files", systemImage: "folder") { choosingFiles = true }
                } label: {
                    Image(systemName: "paperclip")
                        .font(.system(size: 18, weight: .medium))
                        .foregroundStyle(Palette.ink2)
                        .frame(width: 36, height: 36)
                }
                .accessibilityLabel("Attach")
                TextField(chat.placeholder, text: $chat.draft, axis: .vertical)
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
                        .background(chat.canSend ? Palette.claude : Palette.ink3.opacity(0.5), in: Circle())
                }
                .disabled(!chat.canSend)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 10)
        }
        .padding(.top, 7)
        .padding(.bottom, 7)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 0.5) }
        .photosPicker(isPresented: $choosingPhotos, selection: $photos, maxSelectionCount: Attachment.maxPerMessage, matching: .images)
        .onChange(of: photos) {
            let picked = photos
            photos = []
            Task { for item in picked { await attachPhoto(item) } }
        }
        .fileImporter(isPresented: $choosingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            Task { for url in urls { await attachFile(url) } }
        }
    }

    /// A photo from the library. HEIC and other kinds become JPEG, which
    /// Claude reads; PNG, JPEG, GIF and WebP go as they are.
    private func attachPhoto(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            chat.refuse(name: "photo", contentType: "image/jpeg", size: 0, reason: "Couldn't read the photo")
            return
        }
        let kind = item.supportedContentTypes.first { $0.conforms(to: .image) }
        if let kind, let type = kind.preferredMIMEType, ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(type) {
            await chat.attach(name: "photo.\(kind.preferredFilenameExtension ?? "jpg")", contentType: type, data: data)
        } else if let jpeg = UIImage(data: data)?.jpegData(compressionQuality: 0.85) {
            await chat.attach(name: "photo.jpg", contentType: "image/jpeg", data: jpeg)
        } else {
            chat.refuse(name: "photo", contentType: "image/jpeg", size: data.count, reason: "Couldn't read the photo")
        }
    }

    /// A file from Files (iCloud Drive, On My iPhone, a file provider).
    private func attachFile(_ url: URL) async {
        let name = url.lastPathComponent
        let type = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        if size > Attachment.maxBytes {
            chat.refuse(name: name, contentType: type, size: size, reason: "Over 32 MB, the most cos takes")
            return
        }
        guard let data = try? Data(contentsOf: url) else {
            chat.refuse(name: name, contentType: type, size: size, reason: "Couldn't read the file")
            return
        }
        await chat.attach(name: name, contentType: type, data: data)
    }
}

/// A file on the composer: a thumbnail or an icon, its name, and how its
/// upload goes, with an × to take it off.
struct DraftChip: View {
    var file: DraftFile
    var remove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            FileThumb(contentType: file.contentType, data: file.preview, size: 34)
            VStack(alignment: .leading, spacing: 1) {
                Text(file.name)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                HStack(spacing: 4) {
                    if file.isUploading { ProgressView().controlSize(.mini) }
                    Text(detail)
                        .lineLimit(1)
                }
                .font(.caption2)
                .foregroundStyle(failed ? Palette.red : Palette.ink3)
            }
            .frame(maxWidth: 150, alignment: .leading)
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(Palette.ink3)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove \(file.name)")
        }
        .padding(.leading, 5)
        .padding(.trailing, 8)
        .padding(.vertical, 5)
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(failed ? Palette.red.opacity(0.5) : Palette.line, lineWidth: 0.5))
    }

    private var failed: Bool {
        if case .failed = file.state { true } else { false }
    }

    private var detail: String {
        switch file.state {
        case .uploading: "Uploading…"
        case .ready: Attachment.sizeLabel(file.size)
        case .failed(let reason): reason
        }
    }
}

/// A square thumbnail for a picture, else an icon for its kind.
struct FileThumb: View {
    var contentType: String
    var data: Data?
    var size: CGFloat

    var body: some View {
        Group {
            if let data, let image = UIImage(data: data) {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.42))
                    .foregroundStyle(Palette.ink3)
            }
        }
        .frame(width: size, height: size)
        .background(Palette.subtle)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private var symbol: String {
        if contentType == "application/pdf" { return "doc.richtext" }
        if contentType.hasPrefix("image/") { return "photo" }
        if contentType.hasPrefix("text/") || contentType == "application/json" { return "doc.text" }
        return "doc"
    }
}

/// The files one of your messages names, inside its bubble: pictures as
/// pictures (tap for the whole one), other files as a row with their size.
struct SentFiles: View {
    var chat: Chat
    var files: [AttachedFile]

    var body: some View {
        if !files.isEmpty {
            VStack(alignment: .trailing, spacing: 6) {
                ForEach(files) { file in
                    if file.isImage {
                        SentImage(chat: chat, file: file)
                    } else {
                        HStack(spacing: 8) {
                            FileThumb(contentType: file.contentType, data: nil, size: 34)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(file.name).font(.subheadline.weight(.medium)).lineLimit(1)
                                Text(file.sizeLabel).font(.caption).foregroundStyle(Palette.ink3)
                            }
                            Spacer(minLength: 0)
                        }
                        .foregroundStyle(Palette.ink)
                        .padding(6)
                        .frame(minWidth: 180, maxWidth: 240)
                        .background(Palette.page.opacity(0.55), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            .padding(.top, 2)
        }
    }
}

/// A picture you sent: the one you picked, or fetched from the chief's folder.
struct SentImage: View {
    var chat: Chat
    var file: AttachedFile
    @State private var image: UIImage?
    @State private var showing = false

    /// Its own shape, between a wide strip and a short portrait.
    static func height(of image: UIImage, width: CGFloat) -> CGFloat {
        guard image.size.width > 0 else { return width * 0.6 }
        return min(max(width * image.size.height / image.size.width, 90), 260)
    }

    var body: some View {
        Group {
            if let image {
                Button {
                    showing = true
                } label: {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                        .frame(width: 240, height: Self.height(of: image, width: 240))
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                }
                .buttonStyle(.plain)
            } else {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Palette.subtle)
                    .frame(width: 200, height: 130)
                    .overlay { Image(systemName: "photo").foregroundStyle(Palette.ink3) }
            }
        }
        .accessibilityLabel(file.name)
        .accessibilityAddTraits(.isImage)
        .task(id: file.name) {
            if let data = await chat.image(file) { image = UIImage(data: data) }
        }
        .sheet(isPresented: $showing) {
            if let image {
                NavigationStack {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(Color.black)
                        .navigationTitle(file.name)
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) { Button("Done") { showing = false } }
                        }
                }
            }
        }
    }
}

/// The quote a message of yours replies to, muted, above its text:
/// "↩ You replied to the Chief of Staff".
struct QuotedReply: View {
    var reply: ChatReply
    var theirName: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Label(reply.from == .me ? "You replied to yourself" : "You replied to \(theirName)", systemImage: "arrowshape.turn.up.left")
                .labelStyle(.tight)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(Palette.ink3)
            Text(ChatReply.preview(reply.text))
                .font(.footnote)
                .foregroundStyle(Palette.ink2)
                .lineLimit(3)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.page.opacity(0.55), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(alignment: .leading) {
            Rectangle().fill(Palette.claude.opacity(0.6)).frame(width: 3)
                .clipShape(RoundedRectangle(cornerRadius: 1.5))
        }
        .accessibilityElement(children: .combine)
    }
}

/// What the composer replies to, on top of it, with × to drop it.
struct ReplyStrip: View {
    var label: String
    var text: String
    var dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Rectangle().fill(Palette.claude).frame(width: 3).clipShape(RoundedRectangle(cornerRadius: 1.5))
            VStack(alignment: .leading, spacing: 1) {
                Text(label)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Palette.claude)
                Text(text)
                    .font(.caption)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(2)
            }
            Spacer(minLength: 0)
            Button(action: dismiss) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 18))
                    .foregroundStyle(Palette.ink3)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Don't reply")
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 14)
        .padding(.top, 2)
    }
}
