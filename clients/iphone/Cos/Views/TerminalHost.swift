import CosKit
import SwiftTerm
import SwiftUI
import UIKit

/// SwiftTerm's terminal view, fed by a `TerminalSession`: the daemon's
/// terminal stream goes in, keystrokes go back, and the agent's PTY is
/// resized to fit the view. CosKit and SwiftTerm both have terminal
/// types, so this file names CosKit's with their module.
struct TerminalHost: UIViewRepresentable {
    var terminal: TerminalSession
    var fontSize: CGFloat
    /// The keyboard is up: the terminal view is first responder.
    @Binding var keyboard: Bool

    /// Claude Code's dark terminal colours.
    static let background = UIColor(red: 20.0 / 255, green: 20.0 / 255, blue: 19.0 / 255, alpha: 1)
    static let foreground = UIColor(red: 232.0 / 255, green: 230.0 / 255, blue: 222.0 / 255, alpha: 1)

    func makeCoordinator() -> Coordinator {
        Coordinator(terminal: terminal)
    }

    func makeUIView(context: Context) -> SwiftTerm.TerminalView {
        let view = SwiftTerm.TerminalView(frame: .zero, font: Self.font(fontSize))
        view.nativeBackgroundColor = Self.background
        view.nativeForegroundColor = Self.foreground
        view.terminalDelegate = context.coordinator
        context.coordinator.view = view
        terminal.sink = context.coordinator
        terminal.start()
        return view
    }

    func updateUIView(_ view: SwiftTerm.TerminalView, context: Context) {
        if abs(view.font.pointSize - fontSize) > 0.1 {
            view.font = Self.font(fontSize)
        }
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

    static func dismantleUIView(_ view: SwiftTerm.TerminalView, coordinator: Coordinator) {
        coordinator.terminal.stop()
    }

    static func font(_ size: CGFloat) -> UIFont {
        UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }

    @MainActor
    final class Coordinator: TerminalSink {
        let terminal: TerminalSession
        weak var view: SwiftTerm.TerminalView?

        init(terminal: TerminalSession) {
            self.terminal = terminal
        }

        func terminalReset(size: CosKit.TerminalSize) {
            guard let view else { return }
            // ESC c: a full reset, then the snapshot redraws the screen.
            let reset: [UInt8] = [0x1B, 0x63]
            view.feed(byteArray: reset[...])
            view.resize(cols: size.cols, rows: size.rows)
        }

        func terminalFeed(_ bytes: [UInt8]) {
            view?.feed(byteArray: bytes[...])
        }

        func terminalResize(size: CosKit.TerminalSize) {
            guard let view else { return }
            let emulator = view.getTerminal()
            guard emulator.cols != size.cols || emulator.rows != size.rows else { return }
            view.resize(cols: size.cols, rows: size.rows)
        }
    }
}

extension TerminalHost.Coordinator: @preconcurrency TerminalViewDelegate {
    func sizeChanged(source: SwiftTerm.TerminalView, newCols: Int, newRows: Int) {
        let size = CosKit.TerminalSize(rows: newRows, cols: newCols)
        // Resizing the emulator to the PTY's size reports here too; only
        // the size the view fits is the view's.
        guard size != terminal.ptySize else { return }
        terminal.viewSizeChanged(size)
    }

    func send(source: SwiftTerm.TerminalView, data: ArraySlice<UInt8>) {
        terminal.send(Array(data))
    }

    func requestOpenLink(source: SwiftTerm.TerminalView, link: String, params: [String: String]) {
        guard let url = URL(string: link), url.scheme == "https" || url.scheme == "http" else { return }
        UIApplication.shared.open(url)
    }

    func clipboardCopy(source: SwiftTerm.TerminalView, content: Data) {
        if let text = String(data: content, encoding: .utf8) { UIPasteboard.general.string = text }
    }

    func setTerminalTitle(source: SwiftTerm.TerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: SwiftTerm.TerminalView, directory: String?) {}
    func scrolled(source: SwiftTerm.TerminalView, position: Double) {}
    func bell(source: SwiftTerm.TerminalView) {}
    func iTermContent(source: SwiftTerm.TerminalView, content: ArraySlice<UInt8>) {}
    func rangeChanged(source: SwiftTerm.TerminalView, startY: Int, endY: Int) {}
}
