import AppKit
import CosKit
import SwiftTerm
import SwiftUI

/// An agent's live terminal: SwiftTerm fed by a `TerminalSession`. The
/// session owns the stream, keystrokes and resizes; this view only draws and
/// reports what you type and how big it is.
struct AgentTerminalView: NSViewRepresentable {
    let session: TerminalSession

    func makeNSView(context: Context) -> CosTerminalNSView {
        let view = CosTerminalNSView(frame: NSRect(x: 0, y: 0, width: 800, height: 500))
        view.session = session
        session.sink = view
        session.start()
        return view
    }

    func updateNSView(_ view: CosTerminalNSView, context: Context) {
        guard view.session !== session else { return }
        // Another agent in the same view: start over.
        view.session?.stop()
        view.session = session
        session.sink = view
        session.start()
    }

    static func dismantleNSView(_ view: CosTerminalNSView, coordinator: ()) {
        view.session?.stop()
    }
}

/// SwiftTerm's view, as a `TerminalSink`, with the app's colours, copy and
/// paste that work without a main menu, and focus when it appears.
final class CosTerminalNSView: TerminalView, TerminalSink {
    weak var session: TerminalSession?
    /// True while we resize the emulator to the PTY's size, so that isn't
    /// mistaken for the view's own size.
    private var applyingPTYSize = false

    override init(frame: CGRect) {
        super.init(frame: frame, font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular))
        terminalDelegate = self
        optionAsMetaKey = true
        applyTheme()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    // MARK: TerminalSink

    func terminalReset(size: TerminalSize) {
        getTerminal().resetToInitialState()
        applyTheme()
        setPTYSize(size, force: true)
    }

    func terminalFeed(_ bytes: [UInt8]) {
        feed(byteArray: bytes[...])
    }

    func terminalResize(size: TerminalSize) {
        setPTYSize(size, force: false)
    }

    private func setPTYSize(_ size: TerminalSize, force: Bool) {
        let terminal = getTerminal()
        guard force || terminal.cols != size.cols || terminal.rows != size.rows else { return }
        applyingPTYSize = true
        resize(cols: size.cols, rows: size.rows)
        applyingPTYSize = false
    }

    // MARK: Size

    override func setFrameSize(_ newSize: NSSize) {
        // SwiftUI sets the same size again on many updates; re-fitting then
        // would undo a size the PTY was given by someone else.
        guard newSize != frame.size else { return }
        super.setFrameSize(newSize)
        guard newSize.width > 0, newSize.height > 0 else { return }
        // SwiftTerm has just fitted its grid to the frame.
        let terminal = getTerminal()
        session?.viewSizeChanged(TerminalSize(rows: terminal.rows, cols: terminal.cols))
    }

    // MARK: Focus

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.window === window else { return }
            window.makeFirstResponder(self)
            // In case the first layout matched the initial frame, which
            // setFrameSize skips.
            if self.session?.viewSize == nil, self.frame.width > 0, self.frame.height > 0 {
                let terminal = self.getTerminal()
                self.session?.viewSizeChanged(TerminalSize(rows: terminal.rows, cols: terminal.cols))
            }
        }
    }

    // MARK: Copy and paste

    /// Cmd-C, Cmd-V and Cmd-A are handled here, before the Edit menu, so
    /// they act on the terminal rather than going to Claude as keystrokes.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard window?.firstResponder === self,
              event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
              let key = event.charactersIgnoringModifiers
        else { return super.performKeyEquivalent(with: event) }
        switch key {
        case "c":
            copy(self)
            return true
        case "v":
            paste(self)
            return true
        case "a":
            selectAll(self)
            return true
        default:
            return super.performKeyEquivalent(with: event)
        }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        menu.addItem(withTitle: "Copy", action: #selector(copyFromMenu), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(pasteFromMenu), keyEquivalent: "v")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Select All", action: #selector(selectAllFromMenu), keyEquivalent: "a")
        for item in menu.items { item.target = self }
        return menu
    }

    @objc private func copyFromMenu() { copy(self) }
    @objc private func pasteFromMenu() { paste(self) }
    @objc private func selectAllFromMenu() { selectAll(self) }

    // MARK: Theme

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyTheme()
    }

    private func applyTheme() {
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let theme = dark ? TerminalTheme.dark : TerminalTheme.light
        nativeBackgroundColor = theme.background
        nativeForegroundColor = theme.foreground
        caretColor = theme.caret
        selectedTextBackgroundColor = theme.selection
        installColors(theme.ansi)
        layer?.backgroundColor = theme.background.cgColor
        needsDisplay = true
    }
}

extension CosTerminalNSView: @preconcurrency TerminalViewDelegate {
    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        session?.send(Array(data))
    }

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        // Reported from setFrameSize instead, where it's known to be the
        // view's own size rather than the PTY's.
    }

    func setTerminalTitle(source: TerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    func scrolled(source: TerminalView, position: Double) {}
    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}

    /// OSC 52: the program in the terminal put text on the clipboard.
    func clipboardCopy(source: TerminalView, content: Data) {
        guard let text = String(data: content, encoding: .utf8) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// Terminal colours that sit with the app's window: a near-black or
/// near-white page and an ANSI palette that reads well on it. Claude's own
/// colours are mostly 24-bit, so the palette matters for the rest.
@MainActor
struct TerminalTheme {
    var background: NSColor
    var foreground: NSColor
    var caret: NSColor
    var selection: NSColor
    var ansi: [SwiftTerm.Color]

    static let dark = TerminalTheme(
        background: NSColor(srgbRed: 0.110, green: 0.110, blue: 0.118, alpha: 1),
        foreground: NSColor(srgbRed: 0.878, green: 0.878, blue: 0.886, alpha: 1),
        caret: NSColor(srgbRed: 0.85, green: 0.47, blue: 0.34, alpha: 1),
        selection: NSColor(srgbRed: 0.25, green: 0.36, blue: 0.55, alpha: 1),
        ansi: palette([
            0x1C1C1E, 0xFF6B6B, 0x63D297, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xD0D0D4,
            0x6E6E73, 0xFF8787, 0x8CE3B0, 0xF0D49A, 0x8DC8F7, 0xD9A0EA, 0x84D0DA, 0xFFFFFF,
        ])
    )

    static let light = TerminalTheme(
        background: NSColor(srgbRed: 0.992, green: 0.992, blue: 0.996, alpha: 1),
        foreground: NSColor(srgbRed: 0.114, green: 0.114, blue: 0.122, alpha: 1),
        caret: NSColor(srgbRed: 0.78, green: 0.36, blue: 0.22, alpha: 1),
        selection: NSColor(srgbRed: 0.70, green: 0.82, blue: 1.0, alpha: 1),
        // Yellow and white are darkened so they stay readable on white.
        ansi: palette([
            0x1D1D1F, 0xC4282D, 0x1E8A4C, 0x9A6700, 0x0B5CD5, 0x8E3FB8, 0x0F7C8C, 0x6E6E73,
            0x55555A, 0xE0353B, 0x24A35A, 0xB07A00, 0x2F7BEA, 0xA54FD0, 0x139AAD, 0x3A3A3C,
        ])
    )

    private static func palette(_ colors: [UInt32]) -> [SwiftTerm.Color] {
        colors.map { rgb in
            SwiftTerm.Color(
                red: UInt16((rgb >> 16) & 0xFF) * 257,
                green: UInt16((rgb >> 8) & 0xFF) * 257,
                blue: UInt16(rgb & 0xFF) * 257
            )
        }
    }
}
