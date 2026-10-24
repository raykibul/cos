import CosKit
import GhosttyTerminal
import SwiftUI
import UIKit

/// Ghostty's terminal view (libghostty, through libghostty-spm), fed by a
/// `TerminalSession`: the daemon's terminal stream goes in through an
/// in-memory session, keystrokes come back out of it, and the agent's PTY
/// is resized to the grid that fits the view. Ghostty keeps the scrollback,
/// and a drag scrolls it.
struct TerminalHost: UIViewRepresentable {
    var terminal: TerminalSession
    var fontSize: CGFloat
    /// The keyboard is up: the terminal view is first responder.
    @Binding var keyboard: Bool

    /// Claude Code's dark terminal colours.
    static let background = UIColor(red: 20.0 / 255, green: 20.0 / 255, blue: 19.0 / 255, alpha: 1)
    static let foreground = UIColor(red: 232.0 / 255, green: 230.0 / 255, blue: 222.0 / 255, alpha: 1)
    static let backgroundHex = "#141413"
    static let foregroundHex = "#e8e6de"

    func makeCoordinator() -> Coordinator {
        Coordinator(terminal: terminal, fontSize: fontSize)
    }

    func makeUIView(context: Context) -> UITerminalView {
        let coordinator = context.coordinator
        let view = UITerminalView(frame: .zero)
        view.backgroundColor = Self.background
        view.delegate = coordinator
        view.controller = coordinator.controller
        view.configuration = TerminalSurfaceOptions(backend: .inMemory(coordinator.session))
        coordinator.view = view
        terminal.sink = coordinator
        terminal.start()
        return view
    }

    func updateUIView(_ view: UITerminalView, context: Context) {
        context.coordinator.setFontSize(fontSize)
        // Not during the update: showing the keyboard changes state.
        let wanted = keyboard
        Task { @MainActor in
            if wanted, !view.isFirstResponder {
                _ = view.becomeFirstResponder()
            } else if !wanted, view.isFirstResponder {
                _ = view.resignFirstResponder()
            }
        }
    }

    static func dismantleUIView(_ view: UITerminalView, coordinator: Coordinator) {
        coordinator.terminal.stop()
    }

    /// Ghostty's settings: Claude's colours in light and dark mode, at this size.
    static func configuration(fontSize: CGFloat) -> TerminalConfiguration {
        TerminalConfiguration(startingFrom: .default) { builder in
            builder.withFontSize(Float(fontSize))
            builder.withBackground(backgroundHex)
            builder.withForeground(foregroundHex)
            builder.withCursorStyleBlink(false)
        }
    }

    @MainActor
    final class Coordinator: TerminalSink {
        let terminal: TerminalSession
        let session: InMemoryTerminalSession
        let controller: TerminalController
        weak var view: UITerminalView?
        private var fontSize: CGFloat

        init(terminal: TerminalSession, fontSize: CGFloat) {
            self.terminal = terminal
            self.fontSize = fontSize
            let colours = TerminalConfiguration { builder in
                builder.withBackground(TerminalHost.backgroundHex)
                builder.withForeground(TerminalHost.foregroundHex)
            }
            controller = TerminalController(
                configuration: TerminalHost.configuration(fontSize: fontSize),
                theme: TerminalTheme(light: colours, dark: colours)
            )
            // Both callbacks come off the main thread. `DispatchQueue.main`
            // keeps keystrokes in the order they were typed.
            session = InMemoryTerminalSession(
                write: { [weak terminal] data in
                    let bytes = Array(data)
                    DispatchQueue.main.async {
                        MainActor.assumeIsolated { terminal?.send(bytes) }
                    }
                },
                resize: { [weak terminal] viewport in
                    let size = CosKit.TerminalSize(rows: Int(viewport.rows), cols: Int(viewport.columns))
                    DispatchQueue.main.async {
                        MainActor.assumeIsolated { terminal?.viewSizeChanged(size) }
                    }
                },
                suppressesPixelOnlyResizes: true
            )
        }

        func setFontSize(_ size: CGFloat) {
            guard abs(size - fontSize) > 0.1 else { return }
            fontSize = size
            controller.setTerminalConfiguration(TerminalHost.configuration(fontSize: size))
        }

        func terminalReset(size: CosKit.TerminalSize) {
            // ESC c: a full reset, then the snapshot redraws the screen. The
            // grid stays the view's size; the session resizes the PTY to it.
            session.receive(Data([0x1B, 0x63]))
        }

        func terminalFeed(_ bytes: [UInt8]) {
            session.receive(Data(bytes))
        }

        /// Ghostty's grid always fits the view, so a PTY resize from
        /// elsewhere isn't mirrored here; the session takes the PTY back
        /// when this phone types.
        func terminalResize(size: CosKit.TerminalSize) {}
    }
}

extension TerminalHost.Coordinator: TerminalSurfaceOpenURLDelegate {
    func terminalDidRequestOpenURL(_ url: String, kind: TerminalOpenURLKind) {
        guard let url = URL(string: url), url.scheme == "https" || url.scheme == "http" else { return }
        UIApplication.shared.open(url)
    }
}
