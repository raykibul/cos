import AppKit
import CosKit
import SwiftUI

// Renders the app's views to PNGs against an in-process fake daemon:
//   CosSnapshots <output folder>
// The views are the app's own (this target compiles Cos/ without CosApp),
// talking to the fake over a real Unix socket. Windows are drawn offscreen
// and transparent, so nothing shows up or takes focus.

@MainActor
enum Snapshots {
    static func run(output: URL) async throws {
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let server = try FakeServer(handler: route)
        defer { server.stop() }

        let monitor = DaemonMonitor(client: CosClient(socketPath: server.socketPath))
        monitor.start()
        try await waitFor {
            monitor.agents.count == 14 && monitor.projects.count == 2 && monitor.approvals.count == 3
                && monitor.runs.count == RunFixtures.runs.count && monitor.questions.count == 2
        }

        let scenes: [(String, SidebarItem, CGSize)] = [
            ("agent-approval", .agent(8), CGSize(width: 1180, height: 760)),
            ("agent-working", .agent(7), CGSize(width: 1180, height: 760)),
            ("agent-trust", .agent(9), CGSize(width: 1180, height: 560)),
            ("agent-exited", .agent(5), CGSize(width: 1180, height: 560)),
            ("agent-chief-routed", .agent(23), CGSize(width: 1180, height: 640)),
            ("workspace", .workspace(3), CGSize(width: 1180, height: 1250)),
            ("waiting", .waiting, CGSize(width: 1000, height: 900)),
            ("run-running", .run(10), CGSize(width: 1180, height: 2300)),
            ("run-plan", .run(11), CGSize(width: 1180, height: 1250)),
            ("run-planning", .run(12), CGSize(width: 1180, height: 760)),
            ("run-reporting", .run(13), CGSize(width: 1180, height: 1000)),
            ("run-done", .run(7), CGSize(width: 1180, height: 1500)),
            ("run-failed", .run(6), CGSize(width: 1180, height: 760)),
            ("run-cancelled", .run(5), CGSize(width: 1180, height: 600)),
        ]
        for (name, selection, size) in scenes {
            for appearance in [NSAppearance.Name.darkAqua, .aqua] {
                let window = WindowState()
                window.selection = selection
                try await render(MainWindow(monitor: monitor, window: window), size: size, appearance: appearance, to: output, name: name)
            }
        }

        let lisbon = monitor.workspace(id: 3)!
        for appearance in [NSAppearance.Name.darkAqua, .aqua] {
            try await render(NewAgentSheet(workspace: lisbon, monitor: monitor) { _ in }, size: CGSize(width: 480, height: 470), appearance: appearance, to: output, name: "new-agent", settle: .milliseconds(300))
            try await render(NewRunSheet(workspace: lisbon, monitor: monitor) { _ in }, size: CGSize(width: 500, height: 560), appearance: appearance, to: output, name: "new-run", settle: .milliseconds(300))
            let board = monitor.boards[10]!
            try await render(TaskActionSheet(action: TaskAction(task: board.task(id: 103)!, kind: .fail), monitor: monitor), size: CGSize(width: 440, height: 260), appearance: appearance, to: output, name: "task-fail", settle: .milliseconds(300))
            try await render(LimitsSheet(run: board.run, limits: monitor.limits[10]!, monitor: monitor), size: CGSize(width: 460, height: 380), appearance: appearance, to: output, name: "limits", settle: .milliseconds(300))
            try await render(
                MenuBarContent(monitor: monitor, window: WindowState()),
                size: CGSize(width: 360, height: 760), appearance: appearance, to: output, name: "menu-bar", settle: .milliseconds(300)
            )
        }

        // What the terminals sent back: each page sized its agent to fit.
        for request in server.requests where request.method == "POST" {
            print("POST \(request.path) \(String(decoding: request.body, as: UTF8.self))")
        }
        monitor.stop()
    }

    private static func render(
        _ view: some View, size: CGSize, appearance: NSAppearance.Name, to folder: URL, name: String,
        settle: Duration = .milliseconds(1200)
    ) async throws {
        // The window's own background isn't part of a cached view drawing.
        let hosting = NSHostingView(rootView: view
            .frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor)))
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: appearance)
        window.contentView = hosting
        window.alphaValue = 0
        window.setFrameOrigin(NSPoint(x: -20000, y: -20000))
        window.orderFront(nil)
        // Let the terminal connect, get its snapshot and resize.
        try await Task.sleep(for: settle)
        hosting.layoutSubtreeIfNeeded()
        let file = folder.appendingPathComponent("\(name)-\(appearance == .darkAqua ? "dark" : "light").png")
        try image(of: hosting, appearance: appearance).write(to: file)
        // The sidebar's table doesn't draw as part of the window offscreen,
        // but it does on its own.
        if let table = firstTable(in: hosting) {
            let sidebar = folder.appendingPathComponent("\(name)-\(appearance == .darkAqua ? "dark" : "light")-sidebar.png")
            try image(of: table, appearance: appearance).write(to: sidebar)
        }
        print("wrote \(file.path)")
        window.orderOut(nil)
        window.close()
    }

    /// AppKit's cached drawing of the view. The sidebar's List (a table in a
    /// vibrancy view) comes out blank this way; everything else draws.
    private static func image(of view: NSView, appearance: NSAppearance.Name) throws -> Data {
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { throw CocoaError(.fileWriteUnknown) }
        view.cacheDisplay(in: view.bounds, to: rep)
        guard let png = rep.representation(using: .png, properties: [:]) else { throw CocoaError(.fileWriteUnknown) }
        return png
    }

    private static func firstTable(in view: NSView) -> NSTableView? {
        if let table = view as? NSTableView { return table }
        for child in view.subviews {
            if let table = firstTable(in: child) { return table }
        }
        return nil
    }

    private static func waitFor(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            if ContinuousClock.now > deadline { throw CocoaError(.featureUnsupported) }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    /// The fake daemon's routes.
    nonisolated static func route(_ request: FakeServer.Request) -> FakeServer.Reply {
        let path = request.path
        switch (request.method, path) {
        case ("GET", "/v1/status"): return .json(200, Fixtures.status)
        case ("GET", _) where path.hasPrefix("/v1/events/stream"): return .stream("")
        case ("GET", _) where path.hasPrefix("/v1/events"): return .json(200, #"{"events":[]}"#)
        case ("GET", "/v1/projects"): return .json(200, Fixtures.projects)
        case ("GET", "/v1/agents"): return .json(200, Fixtures.agents)
        case ("GET", "/v1/approvals?state=pending"): return .json(200, Fixtures.approvals)
        case ("GET", "/v1/approvals"): return .json(200, Fixtures.allApprovals)
        case ("GET", "/v1/runs"): return .json(200, RunFixtures.list)
        case ("GET", "/v1/questions?state=open"): return .json(200, RunFixtures.questions)
        case ("GET", _) where path.hasPrefix("/v1/runs/"):
            // /v1/runs/{id}, …/limits, …/report
            let parts = path.split(separator: "/")
            let id = Int(parts[2]) ?? 0
            let found: String? = switch parts.count {
            case 3: RunFixtures.board(id)
            case 4 where parts[3] == "limits": RunFixtures.limits(id)
            case 4 where parts[3] == "report": RunFixtures.report(id)
            default: nil
            }
            return found.map { .json(200, $0) } ?? .json(404, #"{"error":"run \#(id) has no report yet"}"#)
        case ("GET", _) where path.hasSuffix("/terminal"):
            let id = Int(path.split(separator: "/")[2]) ?? 0
            return terminal(agent: id)
        case ("POST", _) where path.hasSuffix("/keys") || path.hasSuffix("/resize"):
            return .empty(204)
        default:
            return .json(404, #"{"error":"the snapshot daemon has no \#(request.method) \#(path)"}"#)
        }
    }

    nonisolated private static func terminal(agent id: Int) -> FakeServer.Reply {
        func snapshot(_ text: String, rows: Int = 32, cols: Int = 100) -> String {
            "event: snapshot\ndata: {\"rows\":\(rows),\"cols\":\(cols),\"data\":\"\(Data(text.utf8).base64EncodedString())\"}\n\n"
        }
        let lisbon = "/Users/me/.cos/worktrees/cos-1/lisbon"
        switch id {
        case 8:
            return .stream(snapshot(ClaudeScreen.screen(cwd: lisbon, prompt: "use left-pad in the formatter", dialog: true)))
        case 23:
            return .stream(snapshot(ClaudeScreen.screen(cwd: "/Users/me/.cos/tasks/10-103", prompt: "Task: Rate-limit the API routes", dialog: true)))
        case 7, 6:
            return .stream(snapshot(ClaudeScreen.screen(cwd: lisbon, prompt: "fix the login bug: sessions expire after 5 minutes", dialog: false)))
        case 9:
            let trust = "\u{1B}[2J\u{1B}[H\u{1B}[1mDo you trust the files in this folder?\u{1B}[0m\r\n\r\n/Users/me/.cos/worktrees/cos-1/porto\r\n\r\n"
                + "Claude Code may read files in this folder and run commands with your permission.\r\n\r\n"
                + "\u{1B}[38;2;177;185;249m❯ 1. Yes, proceed\u{1B}[0m\r\n  2. No, exit\r\n"
            return .stream(snapshot(trust))
        case 5:
            return .streamThenEnd(
                snapshot(ClaudeScreen.screen(cwd: "/Users/me/.cos/worktrees/cos-1/porto", prompt: "run the tests and fix what fails", dialog: false))
                    + "event: exit\ndata: {\"exit_code\":0}\n\n"
            )
        default:
            return .json(404, #"{"error":"no agent with id \#(id)"}"#)
        }
    }
}

let output = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".sandbox/snapshots")
NSApplication.shared.setActivationPolicy(.accessory)
Task { @MainActor in
    do {
        try await Snapshots.run(output: output)
        exit(0)
    } catch {
        print("snapshots failed: \(error)")
        exit(1)
    }
}
NSApplication.shared.run()
