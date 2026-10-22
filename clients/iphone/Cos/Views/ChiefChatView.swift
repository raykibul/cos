import CosKit
import SwiftUI

/// A project's Chief of Staff as a chat, read from its transcript: your
/// messages as bubbles, its replies as markdown, tool calls as compact rows
/// and agents' news as "N updates" rows. Live from `agent.transcript`.
struct ChiefChatView: View {
    var session: HostSession
    @State private var chat: ChiefChat
    @FocusState private var composing: Bool

    init(session: HostSession, projectId: Int) {
        self.session = session
        _chat = State(initialValue: ChiefChat(projectId: projectId, session: session))
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
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
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
            }
            .scrollDismissesKeyboard(.interactively)
            .defaultScrollAnchor(.bottom)
            .background(Palette.page)
            .refreshable { await chat.load() }
            .onChange(of: chat.rows.count) { old, _ in
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
            VStack(spacing: 0) {
                WaitingStrip(session: session, chat: chat)
                Composer(chat: chat, focused: $composing)
            }
            .background(Palette.page.opacity(0.96).ignoresSafeArea(edges: .bottom))
        }
        .navigationBarTitleDisplayMode(.inline)
        // An opaque bar: the two-line title is unreadable over scrolled text.
        .toolbarBackground(Palette.page, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 1) {
                    HStack(spacing: 5) {
                        ChiefStar(size: 12)
                        Text("Chief of Staff").font(.headline)
                    }
                    Text([chat.project?.name, chat.staff?.stateLabel].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(Palette.ink3)
                        .lineLimit(1)
                }
            }
            ToolbarItem(placement: .topBarTrailing) {
                if let model = chat.staff?.model {
                    Text(model)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(Palette.ink2)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(Palette.subtle, in: Capsule())
                }
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .task { await chat.load() }
        .onChange(of: session.transcriptTicks[chat.chiefAgentId ?? -1]) {
            Task { await chat.read() }
        }
        .onChange(of: chat.chiefAgentId) {
            Task { await chat.read() }
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

struct ChatRowView: View {
    var chat: ChiefChat
    var row: ChatRow
    @State private var expanded = false

    var body: some View {
        switch row {
        case .user(let message):
            bubble(message.text, footer: nil)
        case .pending(let pending):
            bubble(pending.text, footer: pendingFooter(pending))
        case .reply(let message):
            MarkdownView(markdown: message.text)
                .foregroundStyle(Palette.ink)
                .textSelection(.enabled)
        case .tool(let message):
            toolRow(message.tool ?? ToolCall(name: "tool"))
        case .updates(let message, let count):
            updatesRow(message, count: count)
        }
    }

    private func bubble(_ text: String, footer: AnyView?) -> some View {
        VStack(alignment: .trailing, spacing: 4) {
            Text(text)
                .font(.body)
                .foregroundStyle(Palette.ink)
                .textSelection(.enabled)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(Palette.subtle, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            if let footer { footer }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .padding(.leading, 48)
    }

    private func pendingFooter(_ pending: PendingMessage) -> AnyView {
        switch pending.state {
        case .sending:
            AnyView(Text("Sending…").font(.caption).foregroundStyle(Palette.ink3))
        case .sent:
            AnyView(Text("Sent · waiting for the chief").font(.caption).foregroundStyle(Palette.ink3))
        case .failed(let message):
            AnyView(
                VStack(alignment: .trailing, spacing: 4) {
                    Text(message).font(.caption).foregroundStyle(Palette.red).multilineTextAlignment(.trailing)
                    HStack(spacing: 12) {
                        Button("Discard") { chat.discard(pending) }
                        Button("Retry") { Task { await chat.retry(pending) } }.fontWeight(.semibold)
                    }
                    .font(.caption)
                }
            )
        }
    }

    private func toolRow(_ tool: ToolCall) -> some View {
        Button {
            withAnimation(.snappy) { expanded.toggle() }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Image(systemName: tool.isCosTool ? "star" : symbol(for: tool.name))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(tool.isCosTool ? Palette.amber : Palette.ink3)
                        .frame(width: 16)
                    Text(tool.title)
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(tool.isError ? Palette.red : Palette.ink2)
                    if !tool.summary.isEmpty, !expanded {
                        Text(tool.summary)
                            .font(.footnote.monospaced())
                            .foregroundStyle(Palette.ink3)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                    if tool.isError {
                        Image(systemName: "exclamationmark.circle").font(.caption).foregroundStyle(Palette.red)
                    }
                }
                if expanded, !tool.summary.isEmpty {
                    CodeBox(text: tool.summary)
                }
            }
            .padding(.vertical, 2)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func updatesRow(_ message: TranscriptMessage, count: Int) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.snappy) { expanded.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "tray.and.arrow.down")
                        .font(.caption.weight(.semibold))
                    Text(count == 1 ? "1 update from agents" : "\(count) updates from agents")
                        .font(.footnote.weight(.medium))
                    Spacer()
                    Text(shortAgo(message.tsMs))
                        .font(.caption)
                        .foregroundStyle(Palette.ink3)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.bold))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .foregroundStyle(Palette.ink3)
                }
                .foregroundStyle(Palette.ink2)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(Palette.surface, in: Capsule())
                .overlay(Capsule().stroke(Palette.line, lineWidth: 0.5))
            }
            .buttonStyle(.plain)
            if expanded {
                MarkdownView(markdown: message.updateBody, font: .footnote)
                    .foregroundStyle(Palette.ink2)
                    .padding(12)
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
        }
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

/// Agents the chief started that wait on you, above the composer.
struct WaitingStrip: View {
    var session: HostSession
    var chat: ChiefChat
    @Environment(Navigator.self) private var navigator
    @State private var error: String?

    var body: some View {
        let waiting = chat.agentsWaiting
        if !waiting.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text(waiting.count == 1 ? "1 agent waiting on you" : "\(waiting.count) agents waiting on you")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Palette.claude)
                ForEach(waiting, id: \.agent.id) { entry in
                    HStack(spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(entry.agent.title)
                                .font(.footnote.weight(.semibold))
                                .lineLimit(1)
                            Text(entry.approval?.detail ?? entry.agent.state.style.label)
                                .font(.caption.monospaced())
                                .foregroundStyle(Palette.ink2)
                                .lineLimit(1)
                        }
                        Spacer(minLength: 6)
                        if let approval = entry.approval {
                            Button("Deny") { decide(approval, .deny) }
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(Palette.ink2)
                            Button("Allow") { decide(approval, .allow) }
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(Palette.claude)
                        } else {
                            Button("Open") { navigator.open(.agent(id: entry.agent.id)) }
                                .font(.footnote.weight(.semibold))
                        }
                    }
                }
                if let error {
                    Text(error).font(.caption).foregroundStyle(Palette.red)
                }
            }
            .padding(12)
            .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Palette.claude.opacity(0.35), lineWidth: 1))
            .padding(.horizontal, 12)
            .padding(.bottom, 6)
        }
    }

    private func decide(_ approval: Approval, _ decision: ApprovalDecision) {
        Task {
            do {
                try await session.decide(approval, decision)
                error = nil
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// "What should we work on?": sends to the chief, starting it if needed.
struct Composer: View {
    @Bindable var chat: ChiefChat
    var focused: FocusState<Bool>.Binding

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("What should we work on?", text: $chat.draft, axis: .vertical)
                .lineLimit(1...6)
                .focused(focused)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
            Button {
                Task { await chat.send() }
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
            .accessibilityLabel("Send")
        }
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).stroke(Palette.line, lineWidth: 1))
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .padding(.bottom, 8)
    }

    private var canSend: Bool {
        !chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
