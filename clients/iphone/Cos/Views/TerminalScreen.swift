import CosKit
import SwiftUI

/// An agent's live terminal, full screen: the Chief of Staff's or one of
/// its sessions'. Chips along the top switch between the chief and the
/// sessions of its project without leaving the terminal; a strip shows
/// what the agent waits on you for; a key bar has the keys a phone
/// keyboard lacks. Typing goes straight to Claude, as on the Mac. Turned
/// sideways, the terminal gets about twice the columns.
struct TerminalScreen: View {
    var session: HostSession
    @State private var agentId: Int
    @State private var keyboard = false
    @State private var confirmStop = false
    @State private var error: String?
    @AppStorage("cos.terminal.fontSize") private var fontSize = 12.0
    @Environment(Navigator.self) private var navigator

    init(session: HostSession, agentId: Int) {
        self.session = session
        _agentId = State(initialValue: agentId)
    }

    static let fontSizes = 8.0...20.0

    var body: some View {
        let agent = session.agent(id: agentId)
        let peers = session.terminalPeers(of: agentId)
        VStack(spacing: 0) {
            if peers.count > 1 {
                TerminalChips(session: session, peers: peers, selection: $agentId)
            }
            TerminalAttention(session: session, agentId: agentId, peers: peers, selection: $agentId, error: $error)
            TerminalPane(session: session, agentId: agentId, fontSize: fontSize, keyboard: $keyboard)
                .id(agentId)
        }
        .background(Color(TerminalHost.background).ignoresSafeArea())
        .environment(\.colorScheme, .dark)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(Color(TerminalHost.background), for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(title(agent))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(subtitle(agent))
                        .font(.caption2)
                        .foregroundStyle(agent?.state == .working ? Palette.blue : Color.white.opacity(0.6))
                        .lineLimit(1)
                }
                .accessibilityElement(children: .combine)
            }
            ToolbarItem(placement: .topBarTrailing) {
                menu(agent)
            }
        }
        .confirmationDialog("Stop this session?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop", role: .destructive) { stop() }
        } message: {
            Text("Claude stops. A message from its page resumes it.")
        }
        .alert("Couldn't do that", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            keyboard = true
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
            keyboard = false
        }
    }

    private func menu(_ agent: Agent?) -> some View {
        Menu {
            Button("Larger text", systemImage: "textformat.size.larger") {
                fontSize = min(fontSize + 1, Self.fontSizes.upperBound)
            }
            .disabled(fontSize >= Self.fontSizes.upperBound)
            Button("Smaller text", systemImage: "textformat.size.smaller") {
                fontSize = max(fontSize - 1, Self.fontSizes.lowerBound)
            }
            .disabled(fontSize <= Self.fontSizes.lowerBound)
            Divider()
            if let projectId = session.projectId(ofAgent: agentId) {
                Button("Chat with the chief", systemImage: "bubble.left") { navigator.open(.chief(projectId: projectId)) }
            }
            Button("Session details", systemImage: "info.circle") { navigator.open(.agent(id: agentId)) }
            if let agent, agent.state.isAlive {
                Divider()
                Button("Stop session", systemImage: "stop.circle", role: .destructive) { confirmStop = true }
            }
        } label: {
            Label("More", systemImage: "ellipsis.circle")
        }
    }

    /// "Fix the flaky test", or "quizclub · chief" for a Chief of Staff.
    private func title(_ agent: Agent?) -> String {
        guard let agent else { return "Agent \(agentId)" }
        if agent.role == .staff, let projectId = session.projectId(ofAgent: agent.id), let project = session.project(id: projectId) {
            return "\(project.name) · chief"
        }
        return agent.title
    }

    /// "quizclub/kyoto · sonnet · working".
    private func subtitle(_ agent: Agent?) -> String {
        guard let agent else { return "" }
        var parts: [String] = []
        if agent.role != .staff, let workspace = session.workspace(id: agent.workspaceId) { parts.append(workspace.branch) }
        if let model = agent.model { parts.append(model) }
        parts.append(agent.state.style.label.lowercased())
        return parts.joined(separator: " · ")
    }

    private func stop() {
        let id = agentId
        Task {
            do {
                try await session.stopAgent(id: id)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// The chief and its project's sessions, one chip each, to switch terminals.
struct TerminalChips: View {
    var session: HostSession
    var peers: [Agent]
    @Binding var selection: Int

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(peers) { agent in
                        chip(agent)
                            .id(agent.id)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
            }
            .onAppear { proxy.scrollTo(selection, anchor: .center) }
        }
        .overlay(alignment: .bottom) { Rectangle().fill(Color.white.opacity(0.12)).frame(height: 0.5) }
    }

    private func chip(_ agent: Agent) -> some View {
        let selected = agent.id == selection
        return Button {
            selection = agent.id
        } label: {
            HStack(spacing: 6) {
                if agent.role == .staff {
                    Image(systemName: "star.fill")
                        .font(.caption2)
                        .foregroundStyle(Palette.amber)
                } else {
                    StateDot(color: waits(agent) ? Palette.claude : agent.state.style.color)
                }
                Text(name(agent))
                    .lineLimit(1)
            }
            .font(.footnote.weight(selected ? .semibold : .medium))
            .foregroundStyle(selected ? Color.black : Color.white.opacity(agent.state.isAlive ? 0.8 : 0.45))
            .padding(.horizontal, 11)
            .frame(height: 30)
            .frame(maxWidth: 170)
            .background(selected ? Color.white.opacity(0.92) : Color.white.opacity(0.1), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    /// The workspace's name, or the session's title when its workspace has
    /// more than one session here.
    private func name(_ agent: Agent) -> String {
        if agent.role == .staff { return "chief" }
        let shared = peers.filter { $0.workspaceId == agent.workspaceId }.count > 1
        if !shared, let workspace = session.workspace(id: agent.workspaceId) { return workspace.displayName }
        return agent.title
    }

    private func waits(_ agent: Agent) -> Bool {
        agent.state == .needsTrust || agent.state == .needsLogin || session.pendingApproval(forAgent: agent.id)?.isForYou == true
    }
}

/// One line for what waits on you: this agent's approval (Allow, Deny),
/// trust dialog or login; else another session's, with Show.
struct TerminalAttention: View {
    var session: HostSession
    var agentId: Int
    var peers: [Agent]
    @Binding var selection: Int
    @Binding var error: String?
    @State private var busy = false

    var body: some View {
        if let agent = session.agent(id: agentId), let attention = attention(for: agent) {
            HStack(spacing: 8) {
                Circle().fill(Palette.claude).frame(width: 7, height: 7)
                VStack(alignment: .leading, spacing: 1) {
                    Text(attention.title)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(Palette.claude)
                    if !attention.detail.isEmpty {
                        Text(attention.detail)
                            .font(.footnote.monospaced())
                            .foregroundStyle(.white)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 4)
                if let approval = session.pendingApproval(forAgent: agent.id), approval.isForYou {
                    action("Deny", prominent: false) { try await session.decide(approval, .deny) }
                    action("Allow", prominent: true) { try await session.decide(approval, .allow) }
                } else if agent.state == .needsTrust {
                    action("Trust", prominent: true) { try await session.trust(agentId: agent.id) }
                }
            }
            .padding(.leading, 14)
            .padding(.trailing, 8)
            .padding(.vertical, 6)
            .background(Color.white.opacity(0.06))
            .disabled(busy)
        } else if let other = peers.first(where: { $0.id != agentId && waits($0) }) {
            Button {
                selection = other.id
            } label: {
                HStack(spacing: 8) {
                    Circle().fill(Palette.claude).frame(width: 7, height: 7)
                    Text("\(other.title) needs you")
                        .font(.footnote)
                        .foregroundStyle(.white.opacity(0.85))
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Text("Show")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Palette.claude)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.06))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func attention(for agent: Agent) -> (title: String, detail: String)? {
        if let approval = session.pendingApproval(forAgent: agent.id), approval.isForYou {
            return (approval.toolName == "Bash" ? "Can I run this?" : "Can I use \(approval.toolName)?", approval.detail)
        }
        if agent.state == .needsTrust { return ("Trust this workspace's folder?", "") }
        if agent.state == .needsLogin { return ("Claude needs a login", "cos agent attach \(agent.id)") }
        return nil
    }

    private func waits(_ agent: Agent) -> Bool {
        agent.state == .needsTrust || agent.state == .needsLogin || session.pendingApproval(forAgent: agent.id)?.isForYou == true
    }

    private func action(_ title: String, prominent: Bool, _ work: @escaping @MainActor () async throws -> Void) -> some View {
        Button {
            busy = true
            Task {
                defer { busy = false }
                do {
                    try await work()
                } catch {
                    self.error = error.localizedDescription
                }
            }
        } label: {
            Text(title)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 12)
                .frame(height: 30)
                .background(prominent ? Palette.claude : Color.clear, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .buttonStyle(.plain)
    }
}

/// The terminal itself, what state its stream is in, and the key bar.
struct TerminalPane: View {
    var session: HostSession
    var agentId: Int
    var fontSize: Double
    @Binding var keyboard: Bool
    @State private var terminal: TerminalSession?
    @State private var resuming = false

    init(session: HostSession, agentId: Int, fontSize: Double, keyboard: Binding<Bool>) {
        self.session = session
        self.agentId = agentId
        self.fontSize = fontSize
        _keyboard = keyboard
        let service = session.service
        if let source = service as? any TerminalSource, let control = service as? any TerminalControl {
            _terminal = State(initialValue: TerminalSession(agentId: agentId, source: source, control: control))
        } else {
            _terminal = State(initialValue: nil)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                if let terminal {
                    TerminalHost(terminal: terminal, fontSize: CGFloat(fontSize), keyboard: $keyboard)
                        .padding(.horizontal, 4)
                    overlay(terminal)
                } else {
                    ContentUnavailableView("No live terminal", systemImage: "terminal", description: Text("This host can't stream terminals."))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if !keyboard, let terminal {
                KeyBar(terminal: terminal) { keyboard = true }
            }
        }
    }

    @ViewBuilder
    private func overlay(_ terminal: TerminalSession) -> some View {
        switch terminal.phase {
        case .connecting:
            ProgressView("Connecting…")
                .tint(.white)
                .foregroundStyle(.white.opacity(0.7))
        case .live:
            if let error = terminal.lastError {
                notice(error, color: Palette.red)
            }
        case .reconnecting:
            notice("Reconnecting…", color: Color.white.opacity(0.7))
        case .exited(let code):
            VStack(spacing: 10) {
                Text(code.map { "The session ended (exit \($0))." } ?? "The session was stopped.")
                    .font(.subheadline)
                    .foregroundStyle(.white)
                Button(resuming ? "Resuming…" : "Resume it") { resume(terminal) }
                    .buttonStyle(.borderedProminent)
                    .tint(Palette.claude)
                    .disabled(resuming)
            }
            .padding(16)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .frame(maxHeight: .infinity, alignment: .bottom)
            .padding(.bottom, 16)
        case .gone:
            ContentUnavailableView("This session is gone", systemImage: "terminal")
                .background(Color(TerminalHost.background))
        }
    }

    private func notice(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.caption.weight(.medium))
            .foregroundStyle(color)
            .lineLimit(2)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(.ultraThinMaterial, in: Capsule())
            .frame(maxHeight: .infinity, alignment: .top)
            .padding(.top, 8)
    }

    private func resume(_ terminal: TerminalSession) {
        resuming = true
        Task {
            defer { resuming = false }
            if (try? await session.resume(agentId: agentId)) != nil {
                terminal.restart()
            }
        }
    }
}

/// The keys a phone keyboard lacks, and the keyboard.
struct KeyBar: View {
    var terminal: TerminalSession
    var showKeyboard: () -> Void

    private struct Key: Identifiable {
        var label: String
        var name: String
        var bytes: [UInt8]
        var id: String { name }
    }

    private static let keys = [
        Key(label: "esc", name: "Escape", bytes: [0x1B]),
        Key(label: "^C", name: "Control C", bytes: [0x03]),
        Key(label: "tab", name: "Tab", bytes: [0x09]),
        Key(label: "⇧⇥", name: "Shift tab", bytes: [0x1B, 0x5B, 0x5A]),
        Key(label: "↑", name: "Up", bytes: [0x1B, 0x5B, 0x41]),
        Key(label: "↓", name: "Down", bytes: [0x1B, 0x5B, 0x42]),
        Key(label: "←", name: "Left", bytes: [0x1B, 0x5B, 0x44]),
        Key(label: "→", name: "Right", bytes: [0x1B, 0x5B, 0x43]),
        Key(label: "⏎", name: "Return", bytes: [0x0D]),
    ]

    var body: some View {
        HStack(spacing: 6) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(Self.keys) { key in
                        Button {
                            terminal.send(key.bytes)
                        } label: {
                            keyLabel(Text(key.label))
                        }
                        .accessibilityLabel(key.name)
                    }
                    Button {
                        paste()
                    } label: {
                        keyLabel(Image(systemName: "doc.on.clipboard"))
                    }
                    .accessibilityLabel("Paste")
                }
            }
            Button(action: showKeyboard) {
                Image(systemName: "keyboard")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(.white)
                    .frame(width: 50, height: 36)
                    .background(Palette.claude, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
            .accessibilityLabel("Type")
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .overlay(alignment: .top) { Rectangle().fill(Color.white.opacity(0.12)).frame(height: 0.5) }
        .disabled(terminal.phase != .live)
    }

    private func keyLabel(_ content: some View) -> some View {
        content
            .font(.system(size: 14, weight: .medium, design: .monospaced))
            .foregroundStyle(.white)
            .frame(minWidth: 40, minHeight: 36)
            .padding(.horizontal, 2)
            .background(Color.white.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    /// The clipboard as a bracketed paste, as a terminal pastes.
    private func paste() {
        guard let text = UIPasteboard.general.string, !text.isEmpty else { return }
        terminal.send([0x1B, 0x5B, 0x32, 0x30, 0x30, 0x7E] + Array(text.utf8) + [0x1B, 0x5B, 0x32, 0x30, 0x31, 0x7E])
    }
}
