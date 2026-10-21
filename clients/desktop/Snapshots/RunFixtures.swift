import Foundation

/// Canned runs for the snapshot daemon, one in each state, in the shape of
/// docs/daemon.md and docs/orchestration.md.
///
/// - 10 running in lisbon (3): tasks merged, in review, blocked, working, proposed
/// - 11 awaiting plan approval in porto (4)
/// - 12 planning in madrid (5), with a question
/// - 13 reporting in oslo (6)
/// - 7 done, 6 failed, 5 cancelled: lisbon's past runs
enum RunFixtures {
    static let now = Fixtures.now

    static func json(_ value: Any) -> String {
        String(decoding: try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), as: UTF8.self)
    }

    static func date(_ agoSeconds: Int64) -> String {
        Date(timeIntervalSince1970: Double(now) / 1000 - Double(agoSeconds)).ISO8601Format()
    }

    struct RunSpec {
        var id: Int
        var workspace: Int
        var goal: String
        var state: String
        var preset = "normal"
        var cap = 20
        var chief: Int?
        var reason: String?
        var reportPath: String?
        var startedAgo: Int64
        var updatedAgo: Int64
        var tasks: [[String: Any]]
    }

    static func task(
        _ id: Int, run: Int, _ position: Int, _ title: String, _ state: String, instructions: String, files: [String],
        test: String? = nil, dependsOn: [Int] = [], retries: Int = 0, agent: Int? = nil, lastFailure: String? = nil,
        checks: [String: Any]? = nil, review: [String: Any]? = nil
    ) -> [String: Any] {
        var task: [String: Any] = [
            "id": id, "run_id": run, "position": position, "title": title, "instructions": instructions, "files": files,
            "depends_on": dependsOn, "state": state, "retries": retries,
            "created_at": date(3000), "updated_at": date(60),
        ]
        if let test { task["test_command"] = test }
        if let agent { task["agent_id"] = agent }
        if let lastFailure { task["last_failure"] = lastFailure }
        if let checks { task["checks"] = checks }
        if let review { task["review"] = review }
        if !["proposed", "ready"].contains(state) {
            task["branch"] = "cos/run-\(run)/task-\(id)"
            task["worktree"] = "/Users/me/.cos/tasks/\(run)-\(id)"
        }
        return task
    }

    nonisolated(unsafe) static let passing: [String: Any] = [
        "scope": ["ok": true, "out_of_scope": [String](), "protected": [String](), "uncommitted": [String]()],
        "tests": ["status": "passed", "exit_code": 0, "output_tail": "test result: ok. 12 passed; 0 failed"],
    ]

    nonisolated(unsafe) static let runs: [RunSpec] = [
        RunSpec(id: 10, workspace: 3, goal: "Add rate limiting to the login and API routes", state: "running", chief: 20, startedAgo: 2400, updatedAgo: 30, tasks: [
            task(101, run: 10, 0, "Add a token bucket", "merged",
                 instructions: "Write a token bucket in src/limit.rs: capacity, refill per second, and try_take(). Keep it free of I/O.",
                 files: ["src/limit.rs"], test: "cargo test limit", agent: 21, checks: passing, review: ["verdict": "approve"]),
            task(102, run: 10, 1, "Rate-limit the login route", "review",
                 instructions: "Use the bucket in the login handler: 5 attempts a minute per IP, answering 429 with Retry-After.",
                 files: ["src/routes/login.rs"], test: "cargo test login", dependsOn: [101], agent: 22, checks: passing),
            task(103, run: 10, 2, "Rate-limit the API routes", "blocked",
                 instructions: "Add a tower layer that takes from a per-key bucket on every /api request.",
                 files: ["src/routes/api/**", "src/limit/layer.rs"], test: "cargo test api", dependsOn: [101], retries: 1, agent: 23,
                 lastFailure: "tests failed after the retry: api::burst_is_limited panicked",
                 checks: [
                    "scope": ["ok": false, "out_of_scope": ["Cargo.lock"], "protected": [String](), "uncommitted": [String]()],
                    "tests": ["status": "failed", "exit_code": 101, "output_tail": "running 9 tests\ntest api::burst_is_limited ... FAILED\n\nthread 'api::burst_is_limited' panicked at src/routes/api/mod.rs:88:9:\nassertion `left == right` failed\n  left: 200\n right: 429"],
                 ],
                 review: ["verdict": "changes", "notes": "The layer takes a token before auth, so unauthenticated floods drain real keys' buckets. Key the bucket after auth."]),
            task(104, run: 10, 3, "Load-test the limiter", "running",
                 instructions: "Add a criterion bench that hammers try_take() from 8 threads.",
                 files: ["benches/limit.rs"], dependsOn: [101], agent: 24),
            task(105, run: 10, 4, "Document the limits", "proposed",
                 instructions: "Describe the limits and the 429 answers in docs/limits.md.",
                 files: ["docs/limits.md"], dependsOn: [102, 103]),
        ]),
        RunSpec(id: 11, workspace: 4, goal: "Move the settings page to the new form components", state: "awaiting_plan_approval", preset: "strict", chief: 26, startedAgo: 600, updatedAgo: 45, tasks: [
            task(111, run: 11, 0, "Port the profile form", "proposed",
                 instructions: "Rewrite ProfileForm with <Form>, <Field> and <Submit> from src/ui/form. Keep the field names and validation messages exactly as they are, and keep the existing tests passing.",
                 files: ["src/settings/ProfileForm.tsx", "src/settings/ProfileForm.test.tsx"], test: "pnpm test settings/Profile"),
            task(112, run: 11, 1, "Port the notifications form", "proposed",
                 instructions: "Same as the profile form, for NotificationsForm. The toggles become <Switch>.",
                 files: ["src/settings/NotificationsForm.tsx"], test: "pnpm test settings/Notifications"),
            task(113, run: 11, 2, "Remove the old form helpers", "proposed",
                 instructions: "Delete src/settings/legacy-form.ts once nothing imports it.",
                 files: ["src/settings/legacy-form.ts", "src/settings/index.ts"], test: "pnpm typecheck", dependsOn: [111, 112]),
        ]),
        RunSpec(id: 12, workspace: 5, goal: "Cache the dashboard queries", state: "planning", preset: "trusted", cap: 40, chief: 27, startedAgo: 90, updatedAgo: 90, tasks: []),
        RunSpec(id: 13, workspace: 6, goal: "Upgrade to React 19", state: "reporting", chief: 28, startedAgo: 7200, updatedAgo: 20, tasks: [
            task(131, run: 13, 0, "Bump react and react-dom", "merged", instructions: "Upgrade both packages.", files: ["package.json", "pnpm-lock.yaml"], test: "pnpm test", checks: passing, review: ["verdict": "approve"]),
            task(132, run: 13, 1, "Replace forwardRef", "merged", instructions: "Use ref as a prop.", files: ["src/ui/**"], test: "pnpm test", dependsOn: [131], checks: passing, review: ["verdict": "approve"]),
        ]),
        RunSpec(id: 7, workspace: 3, goal: "Fix the flaky session tests", state: "done", chief: 15, reportPath: "/Users/me/.cos/runs/7/report.md", startedAgo: 90000, updatedAgo: 86000, tasks: [
            task(71, run: 7, 0, "Freeze the clock in session tests", "merged", instructions: "Inject a clock.", files: ["src/auth/session.ts", "src/auth/session.test.ts"], test: "pnpm test session", checks: passing, review: ["verdict": "approve"]),
            task(72, run: 7, 1, "Remove the retry wrapper", "merged", instructions: "Drop the jest retry.", files: ["jest.config.js"], dependsOn: [71], checks: passing, review: ["verdict": "approve"]),
        ]),
        RunSpec(id: 6, workspace: 3, goal: "Split the monolith's billing module", state: "failed", reason: "the chief failed every task: billing has no tests to check a split against", startedAgo: 200000, updatedAgo: 190000, tasks: [
            task(61, run: 6, 0, "Extract invoices", "failed", instructions: "Move invoices.", files: ["src/billing/invoices/**"], lastFailure: "no tests cover invoices"),
        ]),
        RunSpec(id: 5, workspace: 3, goal: "Try the new parser", state: "cancelled", reason: "wrong goal", startedAgo: 300000, updatedAgo: 299000, tasks: []),
    ]

    static func runJSON(_ spec: RunSpec, withCounts: Bool) -> [String: Any] {
        var run: [String: Any] = [
            "id": spec.id, "workspace_id": spec.workspace, "goal": spec.goal, "state": spec.state,
            "preset": spec.preset, "approval_cap": spec.cap,
            "created_at": date(spec.startedAgo), "updated_at": date(spec.updatedAgo),
        ]
        if let chief = spec.chief { run["chief_agent_id"] = chief }
        if let reason = spec.reason { run["reason"] = reason }
        if let path = spec.reportPath { run["report_path"] = path }
        if withCounts { run["tasks"] = counts(spec.tasks) }
        return run
    }

    static func counts(_ tasks: [[String: Any]]) -> [String: Int] {
        var counts = ["proposed": 0, "ready": 0, "running": 0, "checking": 0, "review": 0, "merged": 0, "blocked": 0, "failed": 0, "cancelled": 0]
        for task in tasks { counts[task["state"] as! String, default: 0] += 1 }
        counts["total"] = tasks.count
        return counts
    }

    static var list: String {
        json(["runs": runs.sorted { $0.startedAgo < $1.startedAgo }.map { runJSON($0, withCounts: true) }])
    }

    static func board(_ id: Int) -> String? {
        guard let spec = runs.first(where: { $0.id == id }) else { return nil }
        let workspace = (try? JSONSerialization.jsonObject(with: Data(Fixtures.workspace(spec.workspace).utf8))) ?? NSNull()
        return json(["run": runJSON(spec, withCounts: false), "workspace": workspace, "tasks": spec.tasks, "counts": counts(spec.tasks)])
    }

    static func limits(_ id: Int) -> String? {
        guard let spec = runs.first(where: { $0.id == id }) else { return nil }
        return json(["preset": spec.preset, "approval_cap": spec.cap, "chief_approvals": id == 10 ? 7 : (id == 13 ? 12 : 0)])
    }

    static func report(_ id: Int) -> String? {
        guard id == 7 else { return nil }
        return json(["markdown": """
        # Fix the flaky session tests

        Both tasks merged into `cos/lisbon`. The session tests passed 200 times in a row afterwards.

        ## What changed

        - **Freeze the clock in session tests**: `createSession` takes a `now()` function; the tests pass a fixed one.
        - **Remove the retry wrapper**: `jest.retryTimes(3)` is gone, so a flaky test fails loudly again.

        ## Worth a look

        1. `src/auth/session.ts` still reads `Date.now()` in `refresh()`; that path has no test yet.
        2. The CI cache key didn't change, so the first run after merging is slow.

        ## Approved on your behalf (2)

        1. `pnpm install` — Bash, approved by the chief (install) at 14:03. Reason: the task adds a dev dependency
        2. `pnpm build` — Bash, approved by the chief (worktree) at 14:10. Reason: checking the build

        Rust rules allowed 41 routine requests, the chief refused 1.
        """])
    }

    static let questions = """
    {"questions":[
     {"id":4,"run_id":10,"agent_id":20,"question":"Should API keys share one bucket per account, or get one bucket each?","choices":["One per account","One per key"],"state":"open","answer":null,"created_ms":\(now - 240_000),"answered_ms":null},
     {"id":5,"run_id":12,"agent_id":27,"question":"Is the Redis in docker-compose.yml used in production, or only for local development?","choices":[],"state":"open","answer":null,"created_ms":\(now - 50_000),"answered_ms":null}]}
    """

    /// Agents the runs started.
    static let agents: [String] = [
        Fixtures.agent(id: 20, workspace: 3, title: "Chief: rate limiting", state: "idle", role: "chief", run: 10, model: "opus", startedAgo: 2400, activeAgo: 240,
                       lastMessage: "Asked you about per-key buckets; waiting on the review of task 2."),
        Fixtures.agent(id: 21, workspace: 3, title: "Add a token bucket", state: "exited", role: "worker", run: 10, startedAgo: 2300, activeAgo: 1500, exitCode: 0, lastMessage: "DONE"),
        Fixtures.agent(id: 22, workspace: 3, title: "Rate-limit the login route", state: "idle", role: "worker", run: 10, startedAgo: 1400, activeAgo: 200, lastMessage: "DONE"),
        Fixtures.agent(id: 23, workspace: 3, title: "Rate-limit the API routes", state: "waiting_approval", role: "worker", run: 10, startedAgo: 1400, activeAgo: 60, approval: 31),
        Fixtures.agent(id: 24, workspace: 3, title: "Load-test the limiter", state: "waiting_approval", role: "worker", run: 10, startedAgo: 900, activeAgo: 30, approval: 30),
        Fixtures.agent(id: 25, workspace: 3, title: "Review: Rate-limit the login route", state: "working", role: "reviewer", run: 10, startedAgo: 120, activeAgo: 5),
        Fixtures.agent(id: 26, workspace: 4, title: "Chief: settings forms", state: "idle", role: "chief", run: 11, model: "opus", startedAgo: 600, activeAgo: 45),
        Fixtures.agent(id: 27, workspace: 5, title: "Chief: dashboard cache", state: "idle", role: "chief", run: 12, model: "opus", startedAgo: 90, activeAgo: 50),
        Fixtures.agent(id: 28, workspace: 6, title: "Chief: React 19", state: "working", role: "chief", run: 13, model: "opus", startedAgo: 7200, activeAgo: 3),
    ]

    static func approval(
        _ id: Int, agent: Int, command: String, state: String = "pending", decidedBy: String? = nil,
        tier: String, route: String, reason: String, ago: Int64
    ) -> String {
        json([
            "id": id, "agent_id": agent, "workspace_id": 3, "tool_name": "Bash", "tool_input": ["command": command],
            "summary": "Bash \(command)", "state": state, "decided_by": decidedBy ?? NSNull(), "message": NSNull(),
            "created_ms": now - ago * 1000, "decided_ms": state == "pending" ? NSNull() : now - ago * 1000 + 800,
            "tier": tier, "route": route, "reason": reason,
        ] as [String: Any])
    }

    /// Pending ones: one for you, one routed to the chief.
    static let pending: [String] = [
        approval(30, agent: 24, command: "docker compose up -d redis", tier: "ask_user", route: "user", reason: "docker isn't a program cos knows how to check", ago: 30),
        approval(31, agent: 23, command: "cargo build --release", tier: "chief", route: "chief", reason: "runs a build in the worktree", ago: 60),
    ]

    /// Decided ones, for the run's activity.
    static let decided: [String] = [
        approval(32, agent: 24, command: "cargo test limit", state: "allowed", decidedBy: "rule", tier: "allowed", route: "auto", reason: "the task's test command", ago: 200),
        approval(33, agent: 21, command: "cargo add governor", state: "denied", decidedBy: "chief", tier: "chief", route: "chief", reason: "installs need the trusted preset", ago: 1900),
        approval(34, agent: 21, command: "cargo build", state: "allowed", decidedBy: "chief", tier: "chief", route: "chief", reason: "runs a build in the worktree", ago: 2000),
        approval(35, agent: 22, command: "git push origin HEAD", state: "denied", decidedBy: "user", tier: "never_delegated", route: "user", reason: "git push is never delegated", ago: 1000),
    ]
}
