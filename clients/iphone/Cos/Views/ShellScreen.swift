import CosKit
import SwiftUI

/// A login shell on the host, full screen: the Mac's Ctrl-J terminal, on
/// the phone. It runs on the machine in use, as its daemon's user, in the
/// folder it was opened in, with that project's environment. Drawn by
/// Ghostty like an agent's terminal, with the same key bar. Leaving the
/// screen leaves it running (the info page lists it); Close ends it.
struct ShellScreen: View {
    var session: HostSession
    let shellId: Int
    @State private var terminal: TerminalSession?
    @State private var keyboard = false
    @State private var confirmClose = false
    @State private var error: String?
    @AppStorage("cos.terminal.fontSize") private var fontSize = 12.0
    @Environment(\.dismiss) private var dismiss

    init(session: HostSession, shellId: Int) {
        self.session = session
        self.shellId = shellId
        if let shells = session.service as? any ShellTerminalSource {
            let plumbing = ShellTerminal(shells: shells)
            _terminal = State(initialValue: TerminalSession(agentId: shellId, source: plumbing, control: plumbing))
        } else {
            _terminal = State(initialValue: nil)
        }
    }

    var body: some View {
        let shell = session.shells.first { $0.id == shellId }
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
                    Text(shell.map { session.shellTitle($0) } ?? "Shell")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(subtitle(shell))
                        .font(.caption2)
                        .foregroundStyle(Color.white.opacity(0.6))
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .accessibilityElement(children: .combine)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button("Larger text", systemImage: "textformat.size.larger") {
                        fontSize = min(fontSize + 1, TerminalScreen.fontSizes.upperBound)
                    }
                    .disabled(fontSize >= TerminalScreen.fontSizes.upperBound)
                    Button("Smaller text", systemImage: "textformat.size.smaller") {
                        fontSize = max(fontSize - 1, TerminalScreen.fontSizes.lowerBound)
                    }
                    .disabled(fontSize <= TerminalScreen.fontSizes.lowerBound)
                    Divider()
                    Button("Close shell", systemImage: "xmark.circle", role: .destructive) { confirmClose = true }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .confirmationDialog("Close this shell?", isPresented: $confirmClose, titleVisibility: .visible) {
            Button("Close", role: .destructive) { close() }
        } message: {
            Text("It ends on \(session.host.name), with whatever runs in it.")
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

    /// "on studio · ~/code/atlas".
    private func subtitle(_ shell: Shell?) -> String {
        guard let shell else { return "on \(session.host.name)" }
        return "on \(session.host.name) · \(shell.cwd)"
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
                Text(code.map { "The shell exited (exit \($0))." } ?? "The shell was closed.")
                    .font(.subheadline)
                    .foregroundStyle(.white)
                Button("Close") { close() }
                    .buttonStyle(.borderedProminent)
                    .tint(Palette.claude)
            }
            .padding(16)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .frame(maxHeight: .infinity, alignment: .bottom)
            .padding(.bottom, 16)
        case .gone:
            ContentUnavailableView("This shell is gone", systemImage: "terminal", description: Text("It was closed, or the daemon restarted."))
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

    private func close() {
        Task {
            do {
                try await session.closeShell(id: shellId)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

extension Navigator {
    /// Opens a login shell on the host in `cwd` and shows it. The error to
    /// show when it couldn't.
    func openShell(session: HostSession, cwd: String?, projectId: Int?) async -> String? {
        guard let cwd else { return "cos doesn't know which folder that is." }
        do {
            let shell = try await session.openShell(cwd: cwd, projectId: projectId)
            open(.shell(id: shell.id))
            return nil
        } catch {
            return error.localizedDescription
        }
    }
}

/// A shell in a list: its name, its folder, and since when.
struct ShellRow: View {
    var title: String
    var shell: Shell

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "apple.terminal")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(shell.exited ? Palette.ink3 : Palette.ink2)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.body)
                    .foregroundStyle(shell.exited ? Palette.ink2 : Palette.ink)
                Text(shell.exited ? "exited · \(shell.cwd)" : shell.cwd)
                    .font(.caption.monospaced())
                    .foregroundStyle(Palette.ink3)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .padding(.vertical, 1)
    }
}
