import Foundation

/// Canned daemon data in the shape of docs/daemon.md and docs/agents-api.md.
enum Fixtures {
    static let now = Int64(Date().timeIntervalSince1970 * 1000)

    static let status = #"{"version":"0.1.0","pid":4242,"started_at":"2026-10-22T09:00:00Z","home":"/Users/me/.cos","events":42,"last_seq":42}"#

    static let projects = #"""
    {"projects":[{"id":1,"name":"cos","root":"/Users/me/personal/apps/cos","base_branch":"main","has_origin":true,"created_at":"2026-10-22T09:00:00Z","workspaces":[
      {"id":3,"project_id":1,"name":"lisbon","title":"Fix the login flow","branch":"cos/lisbon","path":"/Users/me/.cos/worktrees/cos-1/lisbon","base":"origin","base_ref":"origin/main","base_commit":"3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f","created_at":"2026-10-22T09:01:00Z"},
      \#(workspace(4)),
      \#(workspace(5)),
      \#(workspace(6))]},
     {"id":2,"name":"opal-app","root":"/Users/me/work/opal-app","base_branch":"master","has_origin":true,"created_at":"2026-10-22T09:10:00Z","workspaces":[]}]}
    """#

    /// Workspaces 4 to 6, which runs use too.
    static func workspace(_ id: Int) -> String {
        let (name, title, commit) = switch id {
        case 4: ("porto", "porto", "9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e")
        case 5: ("madrid", "Dashboard speed", "1c3f5a7e9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d")
        case 6: ("oslo", "React 19", "5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f3f2c9a1e")
        default: ("lisbon", "Fix the login flow", "3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f")
        }
        return #"{"id":\#(id),"project_id":1,"name":"\#(name)","title":"\#(title)","branch":"cos/\#(name)","path":"/Users/me/.cos/worktrees/cos-1/\#(name)","base":"head","base_ref":"HEAD","base_commit":"\#(commit)","created_at":"2026-10-22T09:05:00Z"}"#
    }

    static func agent(
        id: Int, workspace: Int, title: String, state: String, role: String = "user", run: Int? = nil, model: String? = "sonnet",
        rows: Int = 32, cols: Int = 100, startedAgo: Int64, activeAgo: Int64, exitCode: Int? = nil,
        lastMessage: String? = nil, approval: Int? = nil
    ) -> String {
        func json(_ value: String?) -> String {
            guard let value else { return "null" }
            let data = try! JSONEncoder().encode(value)
            return String(decoding: data, as: UTF8.self)
        }
        let exited = state == "exited" || state == "interrupted"
        return """
        {"id":\(id),"workspace_id":\(workspace),"run_id":\(run.map(String.init) ?? "null"),"role":"\(role)","title":\(json(title)),\
        "session_id":"5504b543-4897-4751-add5-f81f24a6\(String(format: "%04d", id))","pid":\(exited ? "null" : String(25600 + id)),\
        "state":"\(state)","model":\(json(model)),"permission_mode":"default","rows":\(rows),"cols":\(cols),\
        "started_ms":\(now - startedAgo * 1000),"last_activity_ms":\(now - activeAgo * 1000),\
        "exited_ms":\(exited ? String(now - activeAgo * 1000) : "null"),"exit_code":\(exitCode.map(String.init) ?? "null"),\
        "last_message":\(json(lastMessage)),"pending_approval_id":\(approval.map(String.init) ?? "null")}
        """
    }

    static let agents: String = {
        let list = [
            agent(id: 8, workspace: 3, title: "Add left-pad to the formatter", state: "waiting_approval", startedAgo: 300, activeAgo: 20, approval: 12),
            agent(id: 7, workspace: 3, title: "fix login bug", state: "working", model: "opus", startedAgo: 900, activeAgo: 3,
                  lastMessage: "Sessions were expiring after 5 minutes because TTL_MS used minutes where it meant hours. I fixed the constant and added a test."),
            agent(id: 9, workspace: 4, title: "Try the new parser", state: "needs_trust", model: nil, startedAgo: 40, activeAgo: 40),
            agent(id: 6, workspace: 4, title: "Write the release notes", state: "idle", model: "haiku", startedAgo: 3600, activeAgo: 1200,
                  lastMessage: "Done. The release notes are in CHANGELOG.md under 0.2.0."),
            agent(id: 5, workspace: 4, title: "Agent 5", state: "exited", role: "worker", startedAgo: 7200, activeAgo: 5000, exitCode: 0,
                  lastMessage: "All 214 tests pass."),
        ] + RunFixtures.agents
        return #"{"agents":["# + list.joined(separator: ",") + "]}"
    }()

    /// From a daemon without routes, so it is yours.
    static let leftPad = """
    {"id":12,"agent_id":8,"workspace_id":3,"tool_name":"Bash","tool_input":{"command":"npm install left-pad --save","description":"Install left-pad for the padding helper"},"tool_use_id":"toolu_01ABC","summary":"Bash npm install left-pad --save","state":"pending","decided_by":null,"message":null,"created_ms":\(now - 20_000),"decided_ms":null}
    """

    static let approvals = #"{"approvals":["# + ([leftPad] + RunFixtures.pending).joined(separator: ",") + "]}"
    static let allApprovals = #"{"approvals":["# + ([leftPad] + RunFixtures.pending + RunFixtures.decided).joined(separator: ",") + "]}"
}

/// A Claude Code screen in the style of 2.1.x, as raw terminal bytes:
/// welcome box, a prompt, tool calls with a diff, and a permission dialog.
/// Hand-made to look like a captured one (24-bit colours, box drawing,
/// CRLF line ends), since running the real CLI costs tokens.
enum ClaudeScreen {
    private static let esc = "\u{1B}["
    private static func rgb(_ r: Int, _ g: Int, _ b: Int) -> String { "\(esc)38;2;\(r);\(g);\(b)m" }
    private static func bg(_ r: Int, _ g: Int, _ b: Int) -> String { "\(esc)48;2;\(r);\(g);\(b)m" }
    private static let reset = "\u{1B}[0m"
    private static let bold = "\u{1B}[1m"
    private static let dim = "\u{1B}[2m"
    private static let orange = rgb(215, 119, 87)
    private static let gray = rgb(153, 153, 153)
    private static let green = rgb(78, 186, 101)
    private static let blue = rgb(177, 185, 249)

    private static func box(_ lines: [String], width: Int, color: String) -> [String] {
        let bar = String(repeating: "─", count: width - 2)
        var out = ["\(color)╭\(bar)╮\(reset)"]
        for line in lines {
            let visible = line.replacingOccurrences(of: "\u{1B}\\[[0-9;]*m", with: "", options: .regularExpression)
            let pad = max(0, width - 4 - visible.count)
            out.append("\(color)│\(reset) \(line)\(String(repeating: " ", count: pad)) \(color)│\(reset)")
        }
        out.append("\(color)╰\(bar)╯\(reset)")
        return out
    }

    static func screen(cwd: String, prompt: String, dialog: Bool) -> String {
        var lines: [String] = []
        lines += box([
            "\(orange)✻\(reset) Welcome to \(bold)Claude Code\(reset)!",
            "",
            "  \(gray)/help for help, /status for your current setup\(reset)",
            "",
            "  \(gray)cwd: \(cwd)\(reset)",
        ], width: 58, color: orange)
        lines += [
            "",
            "\(gray)>\(reset) \(prompt)",
            "",
            "\(rgb(255, 255, 255))⏺\(reset) I'll start by looking at how sessions are created.",
            "",
            "\(green)⏺\(reset) \(bold)Read\(reset)(src/auth/session.ts)",
            "  ⎿  Read \(bold)84\(reset) lines (ctrl+r to expand)",
            "",
            "\(green)⏺\(reset) \(bold)Update\(reset)(src/auth/session.ts)",
            "  ⎿  Updated \(bold)src/auth/session.ts\(reset) with \(bold)1\(reset) addition and \(bold)1\(reset) removal",
            "       \(dim)11\(reset)    export const SESSION_COOKIE = \"sid\";",
            "       \(dim)12\(reset) \(bg(92, 34, 41))-  const TTL_MS = 5 * 60 * 1000;      \(reset)",
            "       \(dim)12\(reset) \(bg(34, 92, 50))+  const TTL_MS = 5 * 60 * 60 * 1000; \(reset)",
            "       \(dim)13\(reset)    ",
            "",
        ]
        if dialog {
            lines += [
                "\(green)⏺\(reset) \(bold)Bash\(reset)(npm install left-pad --save)",
            ]
            lines += box([
                "\(bold)Bash command\(reset)",
                "",
                "  npm install left-pad --save",
                "  \(gray)Install left-pad for the padding helper\(reset)",
                "",
                "Do you want to proceed?",
                "\(blue)❯\(reset) \(blue)1. Yes\(reset)",
                "  2. Yes, and don't ask again for \(bold)npm install\(reset) commands",
                "  3. No, and tell Claude what to do differently \(gray)(esc)\(reset)",
            ], width: 70, color: blue)
        } else {
            lines += [
                "\(orange)✻\(reset) \(orange)Running tests…\(reset) \(gray)(42s · ↓ 1.2k tokens · esc to interrupt)\(reset)",
                "",
            ]
            lines += box(["\(gray)>\(reset) "], width: 98, color: gray)
            lines += ["  \(gray)? for shortcuts\(reset)"]
        }
        return "\(esc)2J\(esc)H" + lines.joined(separator: "\r\n")
    }
}
