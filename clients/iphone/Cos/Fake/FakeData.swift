import CosKit
import Foundation
import UIKit

/// The invented data `FakeCos` starts with. Times are relative to `now`.
struct FakeData {
    let now: Int64

    func ago(_ seconds: Int64) -> Int64 { now - seconds * 1000 }
    func date(_ seconds: Int64) -> Date { Date(unixMilliseconds: ago(seconds)) }

    // MARK: Projects and workspaces

    var projects: [Project] {
        func workspace(_ id: Int, _ project: Int, _ slug: String, _ city: String, _ title: String?, _ base: WorkspaceBase, _ ago: Int64) -> Workspace {
            Workspace(
                id: id, projectId: project, name: city, title: title, city: city, branch: "\(slug)/\(city)",
                path: "/Users/me/.cos/worktrees/\(slug)-\(project)/\(city)", base: base,
                baseRef: base == .origin ? "origin/main" : "HEAD", baseCommit: "3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f",
                createdAt: date(ago)
            )
        }
        return [
            Project(
                id: 1, name: "atlas", root: "/Users/me/code/atlas", baseBranch: "main", hasOrigin: true, createdAt: date(900_000),
                workspaces: [
                    workspace(5, 1, "atlas", "madrid", "Faster search", .origin, 20_000),
                    workspace(4, 1, "atlas", "porto", nil, .head, 40_000),
                    workspace(3, 1, "atlas", "lisbon", "Fix the session timeout", .origin, 86_000),
                ]
            ),
            Project(
                id: 2, name: "harbor", root: "/Users/me/code/harbor", baseBranch: "main", hasOrigin: true, createdAt: date(800_000),
                workspaces: [
                    workspace(9, 2, "harbor", "nairobi", "Billing export", .origin, 30_000),
                    workspace(8, 2, "harbor", "kyoto", nil, .origin, 60_000),
                ]
            ),
            Project(
                id: 3, name: "opal", root: "/Users/me/code/opal", baseBranch: "main", hasOrigin: true, createdAt: date(700_000),
                workspaces: [
                    workspace(10, 3, "opal", "oslo", "Nightly dependency check", .origin, 50_000),
                ]
            ),
        ]
    }

    // MARK: Agents and approvals

    var agents: [Agent] {
        [
            Agent(
                id: 40, workspaceId: 901, role: .staff, title: "Chief of Staff", sessionId: "c0ffee40", pid: 51040, state: .idle, model: "sonnet",
                startedMs: ago(7_200), lastActivityMs: ago(240),
                lastMessage: "Good news from **lisbon**: the timeout was a unit mix-up. The fix is in and the full suite is running.\n\n**porto** is still waiting on your approval for `left-pad`."
            ),
            Agent(
                id: 41, workspaceId: 903, role: .staff, title: "Chief of Staff", state: .idle, model: "haiku", startedMs: ago(60_000), lastActivityMs: ago(30_000),
                lastMessage: "The nightly dependency check passed. Nothing to update."
            ),
            Agent(
                id: 7, workspaceId: 3, title: "Fix the session timeout", pid: 51007, state: .working, model: "sonnet", startedMs: ago(1_500),
                lastActivityMs: ago(20), lastMessage: "Found it: `SESSION_TTL` was in minutes where the code expected hours. Fixed, and running the whole suite now.",
                spawnedBy: 40
            ),
            Agent(
                id: 8, workspaceId: 4, title: "Add left-pad to the formatter", pid: 51008, state: .waitingApproval, model: "haiku", startedMs: ago(1_400),
                lastActivityMs: ago(90), pendingApprovalId: 12, spawnedBy: 40
            ),
            Agent(
                id: 13, workspaceId: 3, title: "Draft the 0.2 release notes", pid: 51013, state: .idle, startedMs: ago(9_000), lastActivityMs: ago(3_000),
                lastMessage: "Drafted the notes in CHANGELOG.md under 0.2.0. They don't mention the session fix yet."
            ),
            Agent(
                id: 14, workspaceId: 4, title: "Spike: streaming tokenizer", state: .exited, startedMs: ago(30_000), exitedMs: ago(26_000), exitCode: 0,
                lastMessage: "The streaming reader halves peak memory on the 2 GB fixture. Notes are in docs/tokenizer.md."
            ),
            Agent(
                id: 10, workspaceId: 5, title: "Build the trigram index", pid: 51010, state: .working, model: "sonnet", startedMs: ago(1_200),
                lastActivityMs: ago(15)
            ),
            Agent(
                id: 11, workspaceId: 8, title: "Upgrade the payments SDK", state: .needsLogin, startedMs: ago(600), lastActivityMs: ago(600),
                envNames: ["CLAUDE_CONFIG_DIR"]
            ),
            Agent(id: 12, workspaceId: 9, title: "Export invoices as CSV", state: .needsTrust, startedMs: ago(300), lastActivityMs: ago(300)),
        ]
    }

    var approvals: [Approval] {
        [
            Approval(
                id: 12, agentId: 8, workspaceId: 4, toolName: "Bash", toolInput: .object(["command": .string("npm install left-pad --save")]),
                summary: "Bash npm install left-pad --save", createdMs: ago(90), tier: .askUser,
                reason: "changes or downloads dependencies (npm install)"
            ),
        ]
    }

    // MARK: The Chief of Staff

    var staffs: [Int: Staff] {
        [
            1: Staff(projectId: 1, agentId: 40, state: .idle, model: "sonnet", brief: "Ship 0.2: the session fix, left-pad, release notes.", identity: Self.identity, worktree: "/Users/me/.cos/worktrees/atlas-1/chief", branch: "atlas/chief", workingAgents: 1, waitingOnYou: 1),
            2: Staff(projectId: 2),
            3: Staff(projectId: 3, agentId: 41, state: .idle, model: "haiku", brief: "Keep dependencies current.", branch: "opal/chief"),
        ]
    }

    static let identity = """
    You are Ada, the Chief of Staff for atlas. You're calm, direct and careful.

    Talk to me like a senior engineer: short answers, no fluff, the result first. Before starting agents, say in one line what each will do. Prefer small PRs, one subject each. Never push to main. I work in Dhaka time and review PRs the same day.
    """

    var notes: [Int: [StaffNote]] {
        [
            1: [
                StaffNote(
                    name: "user-preferences", description: "How I like PRs and replies",
                    body: "- Keep replies short and lead with the result.\n- One subject per PR.\n- Ask before pushing a tag.",
                    createdMs: ago(600_000), updatedMs: ago(86_000)
                ),
                StaffNote(
                    name: "release-process", description: "How atlas cuts a release",
                    body: "Tag from main as `vX.Y.Z` once CI is green. The release notes live in CHANGELOG.md, newest first.",
                    createdMs: ago(500_000), updatedMs: ago(400_000)
                ),
                StaffNote(
                    name: "test-suite", description: "Which tests are slow or flaky",
                    body: "The full suite takes about 6 minutes. `auth/session.test.ts` is flaky under load; run it twice before calling it broken.",
                    createdMs: ago(90_000), updatedMs: ago(3_000)
                ),
            ],
        ]
    }

    var transcripts: [Int: [TranscriptMessage]] {
        let ask = "Sessions keep expiring after five minutes, and the formatter needs left-pad. Get agents on both, and tell me what's left before 0.2."
        let answer = """
        Here's where things stand.

        **The session bug has an agent on it.** It's in *lisbon*, reading the auth middleware first. I'll tell you when it has a fix.

        **left-pad needs you.** The formatter agent in *porto* wants to run `npm install left-pad --save`. Allow it, or tell me to have it write the helper by hand.

        Before 0.2, three things are left:

        1. **Merge lisbon** once its tests pass.
        2. **Read the tokenizer spike in porto**: its notes are in `docs/tokenizer.md`.
        3. **Finish the release notes**, which don't mention the session fix yet.
        """
        let update = """
        [cos · 2 updates]
        ### Agent 7 "Fix the session timeout" finished a turn
        Found it: SESSION_TTL was in minutes where the code expected hours. Fixed, and running the whole suite now.
        ### Agent 8 "Add left-pad to the formatter" is waiting for your approval
        Bash npm install left-pad --save
        """
        let after = "Good news from **lisbon**: the timeout was a unit mix-up (`SESSION_TTL` in minutes, read as hours). The fix is in and the full suite is running.\n\n**porto** is still waiting on your approval for `left-pad`."
        return [
            40: [
                TranscriptMessage(n: 1, role: .user, kind: .text, text: ask, tsMs: ago(1_600), attachments: firstAsk),
                TranscriptMessage(n: 2, role: .assistant, kind: .text, text: "I'll check the workspaces, then start one agent on each.", tsMs: ago(1_590)),
                TranscriptMessage(n: 3, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__workspaces_list"), tsMs: ago(1_585)),
                TranscriptMessage(n: 4, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_spawn", summary: "Fix the session timeout", agentId: 7), tsMs: ago(1_500)),
                TranscriptMessage(n: 5, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_spawn", summary: "Add left-pad to the formatter", agentId: 8), tsMs: ago(1_400)),
                TranscriptMessage(n: 6, role: .assistant, kind: .tool, tool: ToolCall(name: "Bash", summary: "git log --oneline -5 origin/main"), tsMs: ago(1_390)),
                TranscriptMessage(n: 7, role: .assistant, kind: .text, text: answer, tsMs: ago(1_380)),
                TranscriptMessage(n: 8, role: .user, kind: .task, text: Self.milestoneCheck, tsMs: ago(900)),
                TranscriptMessage(n: 9, role: .user, kind: .update, text: update, tsMs: ago(260)),
                TranscriptMessage(n: 10, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_read", summary: "{\"agent_id\":7}", agentId: 7), tsMs: ago(250)),
                TranscriptMessage(n: 11, role: .assistant, kind: .text, text: after, tsMs: ago(240)),
            ],
        ]
    }

    /// What Claude types into the chief when a subagent it ran finishes.
    static let milestoneCheck = """
    <task-notification>
    <task-id>a066345f791fd4714</task-id>
    <tool-use-id>toolu_01NzzivZP49fSMRjtJ3AyN7V</tool-use-id>
    <output-file>/tmp/claude/tasks/a066345f791fd4714.output</output-file>
    <status>completed</status>
    <summary>Agent "Check the 0.2 milestone" finished</summary>
    <note>A task-notification fires each time this agent stops.</note>
    <result>**0.2 milestone: 9 of 12 issues closed.**

    Three are open:

    - **#41 Session expires after five minutes**: lisbon is on it.
    - **#44 Formatter pads with spaces**: waits on left-pad in porto.
    - **#47 Release notes**: drafted, missing the session fix.</result>
    <usage>total_tokens: 18234
    tool_uses: 6
    duration_ms: 94000</usage>
    </task-notification>
    """

    // MARK: Attachments

    /// What you attached to atlas's first ask: a screenshot and a log.
    var files: [Int: [(name: String, contentType: String, data: Data)]] {
        [1: [("login-expired.png", "image/png", Self.loginShot), ("auth.log", "text/plain", Self.authLog)]]
    }

    private var firstAsk: [AttachedFile] {
        files[1, default: []].map { file in
            AttachedFile(
                name: file.name, path: "/Users/me/.cos/worktrees/project-1/chief/.cos/attachments/\(file.name)",
                contentType: file.contentType, sizeLabel: Attachment.sizeLabel(file.data.count)
            )
        }
    }

    static let authLog = Data(String(repeating: "ERROR session expired after 300s\n", count: 40).utf8)

    /// A drawn "Session expired" sign-in screen, so the fake needs no image files.
    static let loginShot: Data = {
        let size = CGSize(width: 480, height: 300)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        let image = UIGraphicsImageRenderer(size: size, format: format).image { context in
            UIColor(red: 0.96, green: 0.95, blue: 0.92, alpha: 1).setFill()
            context.fill(CGRect(origin: .zero, size: size))
            UIColor(red: 0.90, green: 0.88, blue: 0.84, alpha: 1).setFill()
            context.fill(CGRect(x: 0, y: 0, width: 480, height: 34))
            for (i, color) in [UIColor.systemRed, .systemYellow, .systemGreen].enumerated() {
                color.setFill()
                UIBezierPath(ovalIn: CGRect(x: 15 + CGFloat(i) * 18, y: 12, width: 10, height: 10)).fill()
            }
            UIColor.white.setFill()
            UIBezierPath(roundedRect: CGRect(x: 140, y: 70, width: 200, height: 160), cornerRadius: 12).fill()
            let title = NSAttributedString(string: "Sign in", attributes: [.font: UIFont.boldSystemFont(ofSize: 16), .foregroundColor: UIColor.darkText])
            title.draw(at: CGPoint(x: 240 - title.size().width / 2, y: 88))
            UIColor(red: 0.96, green: 0.95, blue: 0.92, alpha: 1).setFill()
            UIBezierPath(roundedRect: CGRect(x: 164, y: 120, width: 152, height: 24), cornerRadius: 6).fill()
            UIBezierPath(roundedRect: CGRect(x: 164, y: 152, width: 152, height: 24), cornerRadius: 6).fill()
            UIColor(red: 0.85, green: 0.47, blue: 0.34, alpha: 1).setFill()
            UIBezierPath(roundedRect: CGRect(x: 164, y: 188, width: 152, height: 28), cornerRadius: 7).fill()
            UIColor(red: 0.98, green: 0.89, blue: 0.87, alpha: 1).setFill()
            UIBezierPath(roundedRect: CGRect(x: 110, y: 246, width: 260, height: 30), cornerRadius: 8).fill()
            let warning = NSAttributedString(
                string: "Session expired. Please sign in again.",
                attributes: [.font: UIFont.systemFont(ofSize: 12), .foregroundColor: UIColor(red: 0.70, green: 0.23, blue: 0.18, alpha: 1)]
            )
            warning.draw(at: CGPoint(x: 240 - warning.size().width / 2, y: 254))
        }
        return image.pngData() ?? Data()
    }()

    var inboxes: [Int: [StaffItem]] {
        [
            1: [
                StaffItem(id: 2, projectId: 1, kind: "agent", agentId: 8, text: "Agent 8 \"Add left-pad to the formatter\" is waiting for your approval\nBash npm install left-pad --save", createdMs: ago(270), deliveredMs: ago(260)),
                StaffItem(id: 1, projectId: 1, kind: "agent", agentId: 7, text: "Agent 7 \"Fix the session timeout\" finished a turn\nFound it: SESSION_TTL was in minutes where the code expected hours.", createdMs: ago(280), deliveredMs: ago(260)),
            ],
        ]
    }

    // MARK: Screens

    /// A screen as the terminal stream's first event: clear, draw each
    /// line, put the cursor back.
    static func snapshot(of screen: AgentScreen) -> TerminalEvent {
        var text = "\u{1B}[2J\u{1B}[H"
        text += screen.text.split(separator: "\n", omittingEmptySubsequences: false).joined(separator: "\r\n")
        let row = (screen.cursor.first ?? 0) + 1
        let col = (screen.cursor.dropFirst().first ?? 0) + 1
        text += "\u{1B}[\(row);\(col)H"
        return .snapshot(rows: max(screen.rows, 1), cols: max(screen.cols, 1), data: Array(text.utf8))
    }

    static func screen(for agent: Agent) -> AgentScreen {
        let lines = [
            "╭──────────────────────────────────────────────╮",
            "│ ✻ Claude Code                                │",
            "│   cwd: ~/.cos/worktrees/atlas-1/lisbon        │",
            "╰──────────────────────────────────────────────╯",
            "",
            "> \(agent.title)",
            "",
            "⏺ Read(src/auth/session.ts)",
            "  ⎿  Read 84 lines",
            "",
            "⏺ Update(src/auth/session.ts)",
            "  ⎿  Updated with 1 addition and 1 removal",
            "       12 - const SESSION_TTL = 12;",
            "       12 + const SESSION_TTL = 12 * 60;",
            "",
            "⏺ Bash(npm test)",
            "  ⎿  Running… 212 tests",
            "",
            "✻ Working… (esc to interrupt)",
            "",
            "────────────────────────────────────────────────",
            "> ",
        ]
        return AgentScreen(rows: lines.count, cols: 48, text: lines.joined(separator: "\n"), cursor: [lines.count - 1, 2])
    }
}

// MARK: The tailnet

extension FakeData {
    /// The machines on the invented tailnet, as `answering` lists them: the
    /// two saved hosts (build-box and studio, both running cos), a Linux
    /// server running cos that isn't saved yet (gpu-box), a Mac without cos
    /// (mini), this iPhone, an iPad that went offline 3 hours ago, and a
    /// tagged CI server. The answering host is never in its own list.
    func peers(answering: String?) -> TailnetPeers {
        func peer(
            _ name: String, _ os: String, _ ip: String, cos: PeerCos = .notRunning, online: Bool = true, hostname: String? = nil,
            lastSeen: Int64? = nil, mine: Bool = true, tagged: Bool = false, isYou: Bool = false
        ) -> TailnetPeer {
            let last = ip.split(separator: ".").last.map(String.init) ?? "1"
            return TailnetPeer(
                id: "n" + name.replacingOccurrences(of: "-", with: "") + "CNTRL", name: name, hostname: hostname ?? name,
                dnsName: name + ".tail0000.ts.net", os: os, ips: [ip, "fd7a:115c:a1e0::" + last],
                online: online, lastSeen: lastSeen.map(date), mine: mine, tagged: tagged, cos: online && mine ? cos : .unknown,
                address: ip + ":7700", isYou: isYou
            )
        }
        let all = [
            peer("build-box", "linux", "100.101.102.110", cos: .running),
            peer("studio", "macOS", "100.101.102.111", cos: .running),
            peer("gpu-box", "linux", "100.101.102.120", cos: .running),
            peer("mini", "macOS", "100.101.102.121"),
            peer("iphone", "iOS", "100.101.102.130", hostname: "My iPhone", isYou: true),
            peer("ipad", "iOS", "100.101.102.131", online: false, hostname: "My iPad", lastSeen: 3 * 3600 + 600),
            peer("ci-runner", "linux", "100.101.102.140", mine: false, tagged: true),
        ]
        let me = all.first { $0.name == answering }
        let host = me.map { TailnetHost(name: $0.name, dnsName: $0.dnsName, os: $0.os, address: $0.address) }
        return TailnetPeers(peers: all.filter { $0.name != answering }, selfName: answering, host: host)
    }
}
