import CosKit
import Foundation

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
        ]
    }

    // MARK: Agents and approvals

    var agents: [Agent] {
        [
            Agent(id: 40, workspaceId: 901, role: .staff, title: "Chief of Staff", sessionId: "c0ffee40", pid: 51040, state: .idle, model: "sonnet", startedMs: ago(7_200), lastActivityMs: ago(240)),
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
            Agent(id: 9, workspaceId: 5, runId: "3", role: .chief, title: "Chief · Faster search", state: .idle, model: "sonnet", startedMs: ago(5_000), lastActivityMs: ago(400)),
            Agent(id: 10, workspaceId: 5, runId: "3", taskId: 21, role: .worker, title: "Build the trigram index", state: .working, model: "sonnet", startedMs: ago(1_200), lastActivityMs: ago(15), cwd: "/Users/me/.cos/tasks/madrid-5/t21-build-the-trigram-index"),
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
                summary: "Bash npm install left-pad --save", createdMs: ago(90), tier: .askUser, route: .user,
                reason: "changes or downloads dependencies (npm install)"
            ),
            Approval(
                id: 13, agentId: 10, workspaceId: 5, toolName: "Bash", toolInput: .object(["command": .string("cargo add fst")]),
                summary: "Bash cargo add fst", createdMs: ago(30), tier: .chief, route: .chief, reason: "installs a package into the task's worktree"
            ),
        ]
    }

    // MARK: Runs

    var runs: [Run] {
        [
            Run(id: 3, workspaceId: 5, goal: "Speed up search with a trigram index", state: .running, chiefAgentId: 9, createdAt: date(5_000), updatedAt: date(60)),
            Run(id: 4, workspaceId: 4, goal: "Stream the tokenizer instead of reading whole files", state: .awaitingPlanApproval, createdAt: date(2_400), updatedAt: date(300)),
            Run(id: 2, workspaceId: 3, goal: "Write the 0.2 release notes", state: .done, preset: .strict, reportPath: "runs/2/report.md", createdAt: date(80_000), updatedAt: date(76_000)),
            Run(id: 1, workspaceId: 9, goal: "Try a Rust rewrite of the exporter", state: .cancelled, reason: "Not worth it before the billing launch.", createdAt: date(200_000), updatedAt: date(190_000)),
        ]
    }

    var tasks: [RunTask] {
        [
            RunTask(id: 20, runId: 3, position: 0, title: "Profile the slow queries", instructions: "Find the queries over 200 ms.", files: ["bench/**"], state: .merged, agentId: 15, checks: TaskChecks(scope: ScopeCheck(ok: true), tests: TestCheck(status: "passed")), review: TaskReview(verdict: "approve")),
            RunTask(id: 21, runId: 3, position: 1, title: "Build the trigram index", instructions: "Index titles and bodies.", files: ["src/search/index.rs"], testCommand: "cargo test index", dependsOn: [20], state: .running, agentId: 10),
            RunTask(id: 22, runId: 3, position: 2, title: "Cache the tokenized queries", instructions: "An LRU of 1,000.", files: ["src/search/cache.rs"], dependsOn: [20], state: .review, agentId: 16),
            RunTask(id: 23, runId: 3, position: 3, title: "Highlight matches", instructions: "Bold the matched trigrams.", files: ["web/src/results.tsx"], dependsOn: [20], state: .blocked, retries: 1, lastFailure: "tests failed: 2 of 48 (results.test.tsx)"),
            RunTask(id: 24, runId: 3, position: 4, title: "Wire it into the search API", instructions: "Behind a flag.", files: ["src/api/search.rs"], dependsOn: [21, 22], state: .proposed),
            RunTask(id: 30, runId: 4, position: 0, title: "Add a streaming reader", instructions: "Read 64 KiB at a time.", files: ["src/io/reader.rs"], state: .proposed),
            RunTask(id: 31, runId: 4, position: 1, title: "Port the tokenizer to the reader", instructions: "Keep the public API.", files: ["src/tokenizer/**"], dependsOn: [30], state: .proposed),
            RunTask(id: 32, runId: 4, position: 2, title: "Document the memory limits", instructions: "In docs/tokenizer.md.", files: ["docs/tokenizer.md"], state: .proposed),
            RunTask(id: 10, runId: 2, position: 0, title: "Collect the merged changes", files: ["CHANGELOG.md"], state: .merged),
            RunTask(id: 11, runId: 2, position: 1, title: "Write the upgrade notes", files: ["docs/upgrading.md"], dependsOn: [10], state: .merged),
            RunTask(id: 5, runId: 1, position: 0, title: "Port the CSV writer", files: ["exporter/**"], state: .cancelled),
        ]
    }

    var questions: [Question] {
        [
            Question(
                id: 5, runId: 3, agentId: 9,
                question: "The trigram index adds about 40 MB per million rows. Build it at startup, or lazily on the first search?",
                choices: ["At startup", "Lazily on first search"], createdMs: ago(200)
            ),
        ]
    }

    var reports: [Int: String] {
        [
            2: """
            # Release notes for 0.2

            Both tasks merged on the first try.

            ## What changed
            - **Sessions** now last 12 hours instead of 5 minutes.
            - The formatter pads columns with `left-pad`.
            - Search results show *where* they matched.

            ## Left open
            1. The upgrade notes don't cover Windows yet.
            2. `CHANGELOG.md` needs a date before tagging.

            ```
            git tag v0.2.0 && git push --tags
            ```

            ## Approved on your behalf (2)
            - `cargo test` in the task worktree (rule)
            - `npm run lint` in the task worktree (rule)
            """,
        ]
    }

    // MARK: The Chief of Staff

    var staffs: [Int: Staff] {
        [
            1: Staff(projectId: 1, agentId: 40, state: .idle, model: "sonnet", brief: "Ship 0.2: the session fix, left-pad, release notes.", worktree: "/Users/me/.cos/worktrees/atlas-1/chief", branch: "atlas/chief", workingAgents: 1, waitingOnYou: 1),
            2: Staff(projectId: 2),
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
        2. **Approve the plan in porto** for the streaming tokenizer.
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
                TranscriptMessage(n: 1, role: .user, kind: .text, text: ask, tsMs: ago(1_600)),
                TranscriptMessage(n: 2, role: .assistant, kind: .text, text: "I'll check the workspaces, then start one agent on each.", tsMs: ago(1_590)),
                TranscriptMessage(n: 3, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__workspaces_list"), tsMs: ago(1_585)),
                TranscriptMessage(n: 4, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_spawn", summary: "Fix the session timeout · lisbon"), tsMs: ago(1_500)),
                TranscriptMessage(n: 5, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_spawn", summary: "Add left-pad to the formatter · porto"), tsMs: ago(1_400)),
                TranscriptMessage(n: 6, role: .assistant, kind: .tool, tool: ToolCall(name: "Bash", summary: "git log --oneline -5 origin/main"), tsMs: ago(1_390)),
                TranscriptMessage(n: 7, role: .assistant, kind: .text, text: answer, tsMs: ago(1_380)),
                TranscriptMessage(n: 8, role: .user, kind: .update, text: update, tsMs: ago(260)),
                TranscriptMessage(n: 9, role: .assistant, kind: .tool, tool: ToolCall(name: "mcp__cos__agent_read", summary: "agent 7"), tsMs: ago(250)),
                TranscriptMessage(n: 10, role: .assistant, kind: .text, text: after, tsMs: ago(240)),
            ],
        ]
    }

    var inboxes: [Int: [StaffItem]] {
        [
            1: [
                StaffItem(id: 2, projectId: 1, kind: "agent", agentId: 8, text: "Agent 8 \"Add left-pad to the formatter\" is waiting for your approval\nBash npm install left-pad --save", createdMs: ago(270), deliveredMs: ago(260)),
                StaffItem(id: 1, projectId: 1, kind: "agent", agentId: 7, text: "Agent 7 \"Fix the session timeout\" finished a turn\nFound it: SESSION_TTL was in minutes where the code expected hours.", createdMs: ago(280), deliveredMs: ago(260)),
            ],
        ]
    }

    // MARK: Screens

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
