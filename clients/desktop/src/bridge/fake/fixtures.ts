// Canned daemon data in the daemon's own JSON (snake_case), shaped as in
// docs/daemon.md, docs/agents-api.md and docs/orchestration.md:
//
// - two projects: cos (lisbon, porto, madrid, oslo) and opal-app (nairobi, kyoto),
//   with branches `<project>/<city>`; porto and kyoto have no title
// - your agents in every state, and each run's chief, workers and reviewers
// - cos's Chief of Staff (agent 40), which started agents 7 and 8, with a
//   conversation (fake/staff.ts); opal-app's chief hasn't started
// - a pending approval routed to you and one routed to a chief, plus decided ones
// - a run in each state: 10 running (tasks in every state), 11 plan waiting,
//   12 planning (with a question), 13 reporting, 7 done (with a report),
//   6 failed, 5 cancelled
import type { ScreenSpec } from "./screen";

export type Raw = Record<string, unknown>;

export interface FakeAgent extends Raw {
  id: number;
  /** Null for a Chief of Staff, which works in its own worktree. */
  workspace_id: number | null;
  run_id: number | null;
  task_id: number | null;
  role: string;
  title: string;
  state: string;
  rows: number;
  cols: number;
  started_ms: number;
}

export interface Fixtures {
  now: number;
  projects: Raw[];
  workspaces: Raw[];
  agents: FakeAgent[];
  approvals: Raw[];
  runs: Raw[];
  tasks: Raw[];
  questions: Raw[];
  limits: Record<number, Raw>;
  reports: Record<number, string>;
  screens: Record<number, Omit<ScreenSpec, "cols" | "rows" | "cwd">>;
  history: { ago: number; kind: string; data: Raw }[];
}

export function makeFixtures(now = Date.now()): Fixtures {
  const ms = (agoSeconds: number) => now - agoSeconds * 1000;
  const iso = (agoSeconds: number) => new Date(ms(agoSeconds)).toISOString();
  const home = "/Users/me/.cos";

  const workspace = (
    id: number,
    projectId: number,
    project: string,
    name: string,
    title: string | null,
    base: "origin" | "head",
    commit: string,
    ago: number,
  ): Raw => ({
    id,
    project_id: projectId,
    name,
    title,
    city: name,
    branch: `${project}/${name}`,
    path: `${home}/worktrees/${project}-${projectId}/${name}`,
    base,
    base_ref: base === "origin" ? (projectId === 1 ? "origin/main" : "origin/master") : "HEAD",
    base_commit: commit,
    created_at: iso(ago),
  });

  const workspaces = [
    workspace(
      3,
      1,
      "cos",
      "lisbon",
      "Fix the login flow",
      "origin",
      "3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f",
      86_000,
    ),
    workspace(
      4,
      1,
      "cos",
      "porto",
      null,
      "head",
      "9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7e",
      70_000,
    ),
    workspace(
      5,
      1,
      "cos",
      "madrid",
      "Dashboard speed",
      "origin",
      "1c3f5a7e9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d",
      50_000,
    ),
    workspace(
      6,
      1,
      "cos",
      "oslo",
      "React 19",
      "origin",
      "5b7d4c8f0a6e2b9d1c3f5a7e9b0d2c4f3f2c9a1e",
      40_000,
    ),
    workspace(
      8,
      2,
      "opal-app",
      "nairobi",
      "Billing export",
      "origin",
      "0a6e2b9d1c3f5a7e9b0d2c4f3f2c9a1e5b7d4c8f",
      20_000,
    ),
    workspace(
      9,
      2,
      "opal-app",
      "kyoto",
      null,
      "head",
      "e9b0d2c4f3f2c9a1e5b7d4c8f0a6e2b9d1c3f5a7",
      9_000,
    ),
  ];

  const projects: Raw[] = [
    {
      id: 1,
      name: "cos",
      root: "/Users/me/personal/apps/cos",
      base_branch: "main",
      has_origin: true,
      created_at: iso(90_000),
    },
    {
      id: 2,
      name: "opal-app",
      root: "/Users/me/work/opal-app",
      base_branch: "master",
      has_origin: true,
      created_at: iso(30_000),
    },
  ];

  let pid = 25_600;
  const agent = (
    id: number,
    workspaceId: number,
    title: string,
    state: string,
    options: {
      role?: string;
      run?: number;
      task?: number;
      model?: string | null;
      mode?: string;
      started?: number;
      active?: number;
      exitCode?: number | null;
      lastMessage?: string | null;
      approval?: number | null;
      cwd?: string;
      spawnedBy?: number;
    } = {},
  ): FakeAgent => {
    const exited = state === "exited" || state === "interrupted";
    const role = options.role ?? "user";
    return {
      id,
      workspace_id: workspaceId,
      run_id: options.run ?? null,
      task_id: options.task ?? null,
      role,
      title,
      session_id: `5504b543-4897-4751-add5-f81f24a6${String(id).padStart(4, "0")}`,
      pid: exited ? null : (pid += 7),
      state,
      model: options.model === undefined ? "sonnet" : options.model,
      permission_mode:
        options.mode ??
        (role === "worker" ? "acceptEdits" : role === "reviewer" ? "plan" : "default"),
      user_settings: role === "user",
      rows: 34,
      cols: 110,
      started_ms: ms(options.started ?? 600),
      last_activity_ms: ms(options.active ?? 5),
      exited_ms: exited ? ms(options.active ?? 5) : null,
      exit_code: options.exitCode ?? null,
      last_message: options.lastMessage ?? null,
      pending_approval_id: options.approval ?? null,
      ...(options.cwd ? { cwd: options.cwd } : {}),
      spawned_by: options.spawnedBy ?? null,
      transcript_path: `/Users/me/.claude/projects/session-${id}.jsonl`,
    };
  };

  const taskTree = (run: number, task: number, slug: string) =>
    `${home}/tasks/lisbon-3/r${run}-t${task}-${slug}`;

  const agents: FakeAgent[] = [
    // Yours.
    // Started by cos's Chief of Staff.
    agent(8, 3, "Add left-pad to the formatter", "waiting_approval", {
      started: 880,
      active: 20,
      approval: 12,
      spawnedBy: 40,
    }),
    agent(7, 3, "Fix login bug", "working", {
      model: "opus",
      started: 900,
      active: 3,
      spawnedBy: 40,
      lastMessage:
        "Sessions were expiring after 5 minutes because TTL_MS used minutes where it meant hours. I fixed the constant and added a test.",
    }),
    agent(9, 4, "Try the new parser", "needs_trust", { model: null, started: 40, active: 40 }),
    // cos's agents use ~/.claude-personal, which nobody has logged into yet.
    agent(11, 4, "Check the docs for broken links", "needs_login", {
      model: null,
      started: 25,
      active: 25,
    }),
    agent(6, 4, "Write the release notes", "idle", {
      model: "haiku",
      started: 3600,
      active: 1200,
      lastMessage: "Done. The release notes are in CHANGELOG.md under 0.2.0.",
    }),
    agent(5, 4, "Bump dependencies", "exited", {
      started: 7200,
      active: 5000,
      exitCode: 0,
      lastMessage: "All 214 tests pass with the new versions.",
    }),
    agent(4, 8, "Export invoices as CSV", "interrupted", {
      started: 9000,
      active: 8000,
      lastMessage: "I've added the CSV writer; next is the download route.",
    }),
    agent(3, 9, "Explore the billing service", "starting", {
      model: "opus",
      started: 2,
      active: 2,
    }),
    // Run 10's chief, workers and reviewer.
    agent(20, 3, "Chief", "idle", {
      role: "chief",
      run: 10,
      started: 2400,
      active: 40,
      lastMessage: "I asked you how to key the API buckets; task 3 waits on that.",
    }),
    agent(21, 3, "Add a token bucket", "exited", {
      role: "worker",
      run: 10,
      task: 101,
      started: 2200,
      active: 1500,
      exitCode: 0,
      lastMessage: "DONE",
      cwd: taskTree(10, 101, "add-a-token-bucket"),
    }),
    agent(22, 3, "Rate-limit the login route", "idle", {
      role: "worker",
      run: 10,
      task: 102,
      started: 1400,
      active: 200,
      lastMessage: "DONE",
      cwd: taskTree(10, 102, "rate-limit-the-login-route"),
    }),
    agent(26, 3, "Review: Rate-limit the login route", "working", {
      role: "reviewer",
      run: 10,
      task: 102,
      started: 120,
      active: 2,
      cwd: taskTree(10, 102, "rate-limit-the-login-route"),
    }),
    agent(23, 3, "Rate-limit the API routes", "idle", {
      role: "worker",
      run: 10,
      task: 103,
      started: 1400,
      active: 300,
      lastMessage: "STUCK: the burst test still sees 200 after the retry",
      cwd: taskTree(10, 103, "rate-limit-the-api-routes"),
    }),
    agent(25, 3, "Log rate-limit hits", "waiting_approval", {
      role: "worker",
      run: 10,
      task: 104,
      started: 600,
      active: 15,
      approval: 31,
      cwd: taskTree(10, 104, "log-rate-limit-hits"),
    }),
    agent(32, 3, "Expose limits in /metrics", "idle", {
      role: "worker",
      run: 10,
      task: 106,
      started: 900,
      active: 30,
      lastMessage: "DONE",
      cwd: taskTree(10, 106, "expose-limits-in-metrics"),
    }),
    // Chiefs of the other runs.
    agent(27, 4, "Chief", "idle", { role: "chief", run: 11, started: 1800, active: 600 }),
    agent(28, 5, "Chief", "working", { role: "chief", run: 12, started: 300, active: 1 }),
    agent(29, 6, "Chief", "working", { role: "chief", run: 13, started: 5000, active: 4 }),
    agent(30, 3, "Chief", "exited", {
      role: "chief",
      run: 7,
      started: 60_000,
      active: 52_000,
      exitCode: 0,
    }),
  ];

  const approval = (
    id: number,
    agentId: number,
    workspaceId: number,
    toolName: string,
    input: Raw,
    summary: string,
    ago: number,
    fields: Raw,
  ): Raw => ({
    id,
    agent_id: agentId,
    workspace_id: workspaceId,
    tool_name: toolName,
    tool_input: input,
    tool_use_id: `toolu_01${String(id).padStart(6, "0")}`,
    summary,
    state: "pending",
    decided_by: null,
    message: null,
    created_ms: ms(ago),
    decided_ms: null,
    tier: null,
    route: null,
    reason: null,
    ...fields,
  });

  const approvals: Raw[] = [
    approval(
      12,
      8,
      3,
      "Bash",
      {
        command: "npm install left-pad --save",
        description: "Install left-pad for the padding helper",
      },
      "Bash npm install left-pad --save",
      20,
      {
        tier: "ask_user",
        route: "user",
        reason: "it changes or downloads dependencies (npm install)",
      },
    ),
    approval(
      31,
      25,
      3,
      "Bash",
      { command: "cargo build --release", description: "Build to check the new log lines compile" },
      "Bash cargo build --release",
      15,
      {
        tier: "chief",
        route: "chief",
        reason: "runs a build inside the task's worktree",
      },
    ),
    approval(40, 21, 3, "Edit", { file_path: "src/limit.rs" }, "Edit src/limit.rs", 2100, {
      state: "allowed",
      decided_by: "rule",
      decided_ms: ms(2100),
      tier: "allowed",
      route: "auto",
      reason: "edits a file inside the task's scope",
    }),
    approval(41, 21, 3, "Bash", { command: "cargo test limit" }, "Bash cargo test limit", 1900, {
      state: "allowed",
      decided_by: "rule",
      decided_ms: ms(1900),
      tier: "allowed",
      route: "auto",
      reason: "runs the task's own test command",
    }),
    approval(
      42,
      22,
      3,
      "Bash",
      { command: "cargo clippy --all-targets" },
      "Bash cargo clippy --all-targets",
      1000,
      {
        state: "allowed",
        decided_by: "chief",
        decided_ms: ms(990),
        message: "A lint run in the task worktree is fine.",
        tier: "chief",
        route: "chief",
        reason: "runs a build tool inside the task's worktree",
      },
    ),
    approval(
      43,
      23,
      3,
      "Bash",
      { command: "curl -s https://crates.io/api/v1/crates/governor" },
      "Bash curl -s https://crates.io/api/v1/crates/governor",
      800,
      {
        state: "denied",
        decided_by: "user",
        decided_ms: ms(760),
        message: "Use the bucket we already have.",
        tier: "ask_user",
        route: "user",
        reason: "it reads from the network, which the Normal preset leaves to you",
      },
    ),
    approval(
      44,
      23,
      3,
      "Bash",
      { command: "git push origin HEAD" },
      "Bash git push origin HEAD",
      700,
      {
        state: "expired",
        decided_by: null,
        decided_ms: ms(400),
        tier: "never_delegated",
        route: "user",
        reason: "pushing to a remote is never delegated",
      },
    ),
    approval(
      45,
      26,
      3,
      "Read",
      { file_path: "src/routes/login.rs" },
      "Read src/routes/login.rs",
      100,
      {
        state: "settled",
        decided_by: "terminal",
        decided_ms: ms(98),
        tier: "allowed",
        route: "user",
      },
    ),
  ];

  const passing = {
    scope: { ok: true, out_of_scope: [], protected: [], uncommitted: [] },
    tests: { status: "passed", exit_code: 0, output_tail: "test result: ok. 12 passed; 0 failed" },
    commits_ahead: 1,
  };

  const task = (
    id: number,
    run: number,
    position: number,
    title: string,
    state: string,
    fields: Raw,
  ): Raw => {
    const slug = title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    const started = !["proposed", "ready"].includes(state);
    return {
      id,
      run_id: run,
      position,
      title,
      instructions: "",
      files: [],
      depends_on: [],
      state,
      retries: 0,
      ...(started
        ? { branch: `cos/lisbon-t${id}-${slug}`, worktree: taskTree(run, id, slug) }
        : {}),
      created_at: iso(3000),
      updated_at: iso(60),
      ...fields,
    };
  };

  const tasks: Raw[] = [
    task(101, 10, 0, "Add a token bucket", "merged", {
      instructions:
        "Write a token bucket in src/limit.rs: capacity, refill per second, and try_take(). Keep it free of I/O.",
      files: ["src/limit.rs"],
      test_command: "cargo test limit",
      agent_id: 21,
      checks: passing,
      review: { verdict: "approve", notes: "Small and well tested.", agent_id: 24 },
    }),
    task(102, 10, 1, "Rate-limit the login route", "review", {
      instructions:
        "Use the bucket in the login handler: 5 attempts a minute per IP, answering 429 with Retry-After.",
      files: ["src/routes/login.rs"],
      test_command: "cargo test login",
      depends_on: [101],
      agent_id: 22,
      checks: passing,
    }),
    task(103, 10, 2, "Rate-limit the API routes", "blocked", {
      instructions: "Add a tower layer that takes from a per-key bucket on every /api request.",
      files: ["src/routes/api/**", "src/limit/layer.rs"],
      test_command: "cargo test api",
      depends_on: [101],
      retries: 1,
      agent_id: 23,
      last_failure: "tests failed after the retry: api::burst_is_limited panicked",
      checks: {
        scope: { ok: false, out_of_scope: ["Cargo.lock"], protected: [], uncommitted: [] },
        tests: {
          status: "failed",
          exit_code: 101,
          output_tail:
            "running 9 tests\ntest api::burst_is_limited ... FAILED\n\nthread 'api::burst_is_limited' panicked at src/routes/api/mod.rs:88:9:\nassertion `left == right` failed\n  left: 200\n right: 429",
        },
        commits_ahead: 2,
      },
      review: {
        verdict: "changes",
        notes:
          "The layer takes a token before auth, so unauthenticated floods drain real keys' buckets. Key the bucket after auth.",
        agent_id: 33,
      },
    }),
    task(104, 10, 3, "Log rate-limit hits", "running", {
      instructions:
        "Log each 429 at info with the route and the key, rate-limited to one line a second.",
      files: ["src/limit.rs", "src/log.rs"],
      test_command: "cargo test log",
      depends_on: [101],
      agent_id: 25,
    }),
    task(106, 10, 4, "Expose limits in /metrics", "checking", {
      instructions:
        "Count allowed and refused requests per route and export them as Prometheus counters.",
      files: ["src/metrics.rs"],
      test_command: "cargo test metrics",
      depends_on: [101],
      agent_id: 32,
      checks: {
        scope: { ok: true, out_of_scope: [], protected: [], uncommitted: [] },
        tests: null,
        commits_ahead: 1,
      },
    }),
    task(107, 10, 5, "Add a config flag", "ready", {
      instructions: "Read RATE_LIMIT_PER_MINUTE from the environment, defaulting to 5.",
      files: ["src/config.rs"],
      depends_on: [101],
    }),
    task(105, 10, 6, "Document the limits", "proposed", {
      instructions: "Add a section to docs/api.md on the limits, the 429 answer and Retry-After.",
      files: ["docs/api.md"],
      depends_on: [102, 103],
    }),
    task(108, 10, 7, "Rate-limit websockets", "failed", {
      instructions: "Apply the same limits to websocket upgrades.",
      files: ["src/routes/ws.rs"],
      depends_on: [101],
      last_failure: "There are no websocket routes in this repo; nothing to limit.",
    }),
    task(109, 10, 8, "Share buckets through Redis", "cancelled", {
      instructions: "Move the buckets into Redis so several instances share them.",
      files: ["src/limit/redis.rs"],
      depends_on: [101],
      last_failure: "Out of scope for this run.",
    }),
    // Run 11: a plan waiting for you.
    task(111, 11, 0, "Write a streaming tokenizer", "proposed", {
      instructions:
        "Add src/parse/tokens.rs: a tokenizer that reads from any `Read` and yields tokens with their spans, without loading the whole file.",
      files: ["src/parse/tokens.rs"],
      test_command: "cargo test tokens",
    }),
    task(112, 11, 1, "Move the parser onto the tokenizer", "proposed", {
      instructions:
        "Make src/parse/mod.rs pull tokens from the new tokenizer instead of indexing into a String. Keep the public API.",
      files: ["src/parse/mod.rs", "src/parse/expr.rs"],
      test_command: "cargo test parse",
      depends_on: [111],
    }),
    task(113, 11, 2, "Benchmark a 200 MB file", "proposed", {
      instructions:
        "Add a criterion benchmark that parses a generated 200 MB file, and note the numbers in the PR.",
      files: ["benches/parse.rs", "Cargo.toml"],
      depends_on: [112],
    }),
    // Run 13: reporting.
    ...[
      [131, "Bump react and react-dom to 19", "merged"],
      [132, "Replace forwardRef with ref props", "merged"],
      [133, "Move to the new JSX transform", "merged"],
      [134, "Drop the legacy context API", "cancelled"],
    ].map(([id, title, state], position) =>
      task(id as number, 13, position, title as string, state as string, {
        instructions: `${title}.`,
        files: ["web/**"],
        test_command: "pnpm test",
        checks: state === "merged" ? passing : undefined,
        review: state === "merged" ? { verdict: "approve", notes: null, agent_id: 50 } : undefined,
        last_failure:
          state === "cancelled" ? "Nothing uses the legacy context any more." : undefined,
      }),
    ),
    // Run 7: done.
    task(71, 7, 0, "Freeze time in the session tests", "merged", {
      instructions: "Use a fake clock in tests/session.rs instead of sleeping.",
      files: ["tests/session.rs"],
      test_command: "cargo test session",
      checks: passing,
      review: { verdict: "approve", notes: null, agent_id: 51 },
    }),
    task(72, 7, 1, "Make the expiry check inclusive", "merged", {
      instructions: "A session that expires exactly now is expired.",
      files: ["src/auth/session.rs"],
      test_command: "cargo test session",
      depends_on: [71],
      checks: passing,
      review: { verdict: "approve", notes: null, agent_id: 52 },
    }),
    // Run 6: failed.
    task(61, 6, 0, "Add a Redis session store", "failed", {
      instructions: "Store sessions in Redis behind the SessionStore trait.",
      files: ["src/auth/redis.rs"],
      last_failure: "Redis isn't available in CI, so the tests can't run.",
    }),
    task(62, 6, 1, "Switch production to Redis", "cancelled", {
      instructions: "Use the Redis store when REDIS_URL is set.",
      files: ["src/main.rs"],
      depends_on: [61],
      last_failure: "The run failed.",
    }),
  ];

  const run = (
    id: number,
    workspaceId: number,
    goal: string,
    state: string,
    ago: number,
    updated: number,
    fields: Raw = {},
  ): Raw => ({
    id,
    workspace_id: workspaceId,
    goal,
    state,
    preset: "normal",
    approval_cap: 20,
    created_at: iso(ago),
    updated_at: iso(updated),
    ...fields,
  });

  const runs: Raw[] = [
    run(13, 6, "Upgrade the web app to React 19", "reporting", 5200, 10, {
      chief_agent_id: 29,
      preset: "trusted",
      approval_cap: 40,
    }),
    run(12, 5, "Make the dashboard load in under a second", "planning", 320, 300, {
      chief_agent_id: 28,
      preset: "strict",
      approval_cap: 0,
    }),
    run(11, 4, "Move the parser to a streaming tokenizer", "awaiting_plan_approval", 1900, 600, {
      chief_agent_id: 27,
    }),
    run(10, 3, "Add rate limiting to the login and API routes", "running", 2500, 15, {
      chief_agent_id: 20,
    }),
    run(7, 3, "Fix the flaky session tests", "done", 60_000, 52_000, {
      chief_agent_id: 30,
      report_path: "runs/7/report.md",
    }),
    run(6, 3, "Move sessions to Redis", "failed", 80_000, 78_000, {
      reason: "The chief failed the run: Redis isn't available in CI.",
    }),
    run(5, 3, "Try a structured logger", "cancelled", 84_000, 83_500, {
      reason: "Not now; logging changes wait for 0.3.",
    }),
  ];

  const questions: Raw[] = [
    {
      id: 5,
      run_id: 10,
      agent_id: 20,
      question:
        "Task 3 keeps draining real users' buckets. Should API buckets be keyed per user (after auth) or per IP (before auth)?",
      choices: ["Per user, after auth", "Per IP, before auth"],
      state: "open",
      answer: null,
      created_ms: ms(45),
      answered_ms: null,
    },
    {
      id: 4,
      run_id: 12,
      agent_id: 28,
      question:
        "The slowest query is the 30-day revenue rollup (2.4 s). Precompute it nightly, or cache it for 5 minutes?",
      choices: ["Precompute nightly", "Cache for 5 minutes"],
      state: "open",
      answer: null,
      created_ms: ms(120),
      answered_ms: null,
    },
  ];

  const limits: Record<number, Raw> = {
    13: { preset: "trusted", approval_cap: 40, chief_approvals: 12 },
    12: { preset: "strict", approval_cap: 0, chief_approvals: 0 },
    11: { preset: "normal", approval_cap: 20, chief_approvals: 0 },
    10: { preset: "normal", approval_cap: 20, chief_approvals: 3 },
    7: { preset: "normal", approval_cap: 20, chief_approvals: 4 },
    6: { preset: "normal", approval_cap: 20, chief_approvals: 1 },
    5: { preset: "normal", approval_cap: 20, chief_approvals: 0 },
  };

  const reports: Record<number, string> = {
    7: `# Fix the flaky session tests

Both tasks merged into \`cos/lisbon\`. The session tests passed **200 runs in a row** after the change, against 1 failure in 12 before.

## What changed

1. **Freeze time in the session tests:** \`tests/session.rs\` uses a fake clock instead of \`sleep\`, so a slow CI machine can't push a session past its expiry.
2. **Make the expiry check inclusive:** a session that expires exactly now is expired. This was the real bug: it made the tests pass or fail depending on the millisecond.

| Task | Commits | Tests |
|---|---|---|
| Freeze time in the session tests | 1 | 12 passed |
| Make the expiry check inclusive | 1 | 12 passed |

## Worth a look

- \`SessionStore::touch\` still reads the system clock directly. It isn't tested, so nothing flakes, but it should take the clock too.

\`\`\`rust
pub fn is_expired(&self, now: Instant) -> bool {
    now >= self.expires_at
}
\`\`\`

## Approved on your behalf (4)

- 3 by rule: the tasks' own test commands and edits in scope.
- 1 by the chief: \`cargo clippy --all-targets\` in a task worktree.
`,
  };

  const screens: Fixtures["screens"] = {
    8: {
      prompt: "Add left-pad to the formatter and use it for the table columns",
      transcript: "login",
      footer: {
        kind: "dialog",
        command: "npm install left-pad --save",
        description: "Install left-pad for the padding helper",
      },
    },
    7: {
      prompt: "Sessions expire after 5 minutes instead of 5 hours. Find out why and fix it.",
      transcript: "login",
      footer: { kind: "working", verb: "Running tests", seconds: 42 },
    },
    9: { prompt: "", transcript: "blank", footer: { kind: "trust" } },
    11: { prompt: "", transcript: "blank", footer: { kind: "login" } },
    6: {
      prompt: "Write the release notes for 0.2.0 from the git log",
      transcript: "notes",
      footer: { kind: "idle" },
    },
    5: {
      prompt: "Bump every dependency to its latest minor version",
      transcript: "notes",
      footer: { kind: "exited" },
    },
    4: { prompt: "", transcript: "blank", footer: { kind: "exited" } },
    3: { prompt: "", transcript: "blank", footer: { kind: "idle" } },
    20: {
      prompt: "(cos) Task 3 is blocked: tests failed after the retry.",
      transcript: "plan",
      footer: { kind: "idle" },
    },
    22: { prompt: "Rate-limit the login route", transcript: "limiter", footer: { kind: "idle" } },
    23: { prompt: "Rate-limit the API routes", transcript: "limiter", footer: { kind: "idle" } },
    25: {
      prompt: "Log rate-limit hits",
      transcript: "limiter",
      footer: {
        kind: "dialog",
        command: "cargo build --release",
        description: "Build to check the new log lines compile",
      },
    },
    26: {
      prompt: "Review task 102: Rate-limit the login route",
      transcript: "review",
      footer: { kind: "working", verb: "Reviewing", seconds: 18 },
    },
    28: {
      prompt: "Make the dashboard load in under a second",
      transcript: "blank",
      footer: { kind: "working", verb: "Exploring the repo", seconds: 95 },
    },
    29: {
      prompt: "(cos) Every task is finished. Write the report.",
      transcript: "blank",
      footer: { kind: "working", verb: "Writing the report", seconds: 31 },
    },
  };

  const history: Fixtures["history"] = [
    { ago: 90_100, kind: "daemon.started", data: { pid: 4242, version: "0.1.0" } },
    {
      ago: 90_000,
      kind: "project.added",
      data: { project_id: 1, name: "cos", root: "/Users/me/personal/apps/cos" },
    },
    {
      ago: 86_000,
      kind: "workspace.created",
      data: {
        workspace_id: 3,
        project_id: 1,
        name: "lisbon",
        branch: "cos/lisbon",
        path: `${home}/worktrees/cos-1/lisbon`,
        base: "origin",
      },
    },
    {
      ago: 70_000,
      kind: "workspace.created",
      data: {
        workspace_id: 4,
        project_id: 1,
        name: "porto",
        branch: "cos/porto",
        path: `${home}/worktrees/cos-1/porto`,
        base: "head",
      },
    },
    {
      ago: 60_000,
      kind: "run.created",
      data: {
        run_id: 7,
        workspace_id: 3,
        goal: "Fix the flaky session tests",
        preset: "normal",
        approval_cap: 20,
      },
    },
    {
      ago: 52_000,
      kind: "run.state_changed",
      data: { run_id: 7, from: "reporting", to: "done", report: "runs/7/report.md" },
    },
    {
      ago: 50_000,
      kind: "workspace.created",
      data: {
        workspace_id: 5,
        project_id: 1,
        name: "madrid",
        branch: "cos/madrid",
        path: `${home}/worktrees/cos-1/madrid`,
        base: "origin",
      },
    },
    {
      ago: 40_000,
      kind: "workspace.created",
      data: {
        workspace_id: 6,
        project_id: 1,
        name: "oslo",
        branch: "cos/oslo",
        path: `${home}/worktrees/cos-1/oslo`,
        base: "origin",
      },
    },
    {
      ago: 30_000,
      kind: "project.added",
      data: { project_id: 2, name: "opal-app", root: "/Users/me/work/opal-app" },
    },
    {
      ago: 20_000,
      kind: "workspace.created",
      data: {
        workspace_id: 8,
        project_id: 2,
        name: "nairobi",
        branch: "opal-app/nairobi",
        path: `${home}/worktrees/opal-app-2/nairobi`,
        base: "origin",
      },
    },
    {
      ago: 9_000,
      kind: "workspace.created",
      data: {
        workspace_id: 9,
        project_id: 2,
        name: "kyoto",
        branch: "opal-app/kyoto",
        path: `${home}/worktrees/opal-app-2/kyoto`,
        base: "head",
      },
    },
    {
      ago: 7_200,
      kind: "agent.started",
      data: { agent_id: 5, workspace_id: 4, role: "user", session_id: "5504b543-…0005" },
    },
    { ago: 5_000, kind: "agent.exited", data: { agent_id: 5, exit_code: 0 } },
    {
      ago: 2_500,
      kind: "run.created",
      data: {
        run_id: 10,
        workspace_id: 3,
        goal: "Add rate limiting to the login and API routes",
        preset: "normal",
        approval_cap: 20,
      },
    },
    { ago: 2_300, kind: "plan.proposed", data: { run_id: 10, tasks: 9, replaced: 0 } },
    { ago: 2_250, kind: "plan.approved", data: { run_id: 10, tasks: 9 } },
    {
      ago: 2_250,
      kind: "run.state_changed",
      data: { run_id: 10, from: "awaiting_plan_approval", to: "running" },
    },
    {
      ago: 2_250,
      kind: "task.state_changed",
      data: {
        run_id: 10,
        task_id: 101,
        title: "Add a token bucket",
        from: "proposed",
        to: "ready",
        retries: 0,
      },
    },
    {
      ago: 1_600,
      kind: "task.checked",
      data: { run_id: 10, task_id: 101, scope_ok: true, tests: "passed" },
    },
    { ago: 1_550, kind: "task.reviewed", data: { run_id: 10, task_id: 101, verdict: "approve" } },
    {
      ago: 1_500,
      kind: "task.merged",
      data: { run_id: 10, task_id: 101, commit: "8c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d" },
    },
    {
      ago: 1_900,
      kind: "run.created",
      data: {
        run_id: 11,
        workspace_id: 4,
        goal: "Move the parser to a streaming tokenizer",
        preset: "normal",
        approval_cap: 20,
      },
    },
    { ago: 600, kind: "plan.proposed", data: { run_id: 11, tasks: 3, replaced: 0 } },
    {
      ago: 900,
      kind: "agent.started",
      data: { agent_id: 7, workspace_id: 3, role: "user", session_id: "5504b543-…0007" },
    },
    {
      ago: 800,
      kind: "approval.decided",
      data: { approval_id: 43, state: "denied", decided_by: "user" },
    },
    {
      ago: 320,
      kind: "run.created",
      data: {
        run_id: 12,
        workspace_id: 5,
        goal: "Make the dashboard load in under a second",
        preset: "strict",
        approval_cap: 0,
      },
    },
    {
      ago: 300,
      kind: "agent.started",
      data: { agent_id: 8, workspace_id: 3, role: "user", session_id: "5504b543-…0008" },
    },
    {
      ago: 300,
      kind: "task.state_changed",
      data: {
        run_id: 10,
        task_id: 103,
        title: "Rate-limit the API routes",
        from: "checking",
        to: "blocked",
        retries: 1,
        reason: "tests failed after the retry: api::burst_is_limited panicked",
      },
    },
    { ago: 120, kind: "question.asked", data: { question_id: 4, run_id: 12 } },
    { ago: 45, kind: "question.asked", data: { question_id: 5, run_id: 10 } },
    {
      ago: 40,
      kind: "agent.started",
      data: { agent_id: 9, workspace_id: 4, role: "user", session_id: "5504b543-…0009" },
    },
    {
      ago: 40,
      kind: "agent.state_changed",
      data: { agent_id: 9, state: "needs_trust", from: "starting" },
    },
    {
      ago: 20,
      kind: "approval.requested",
      data: {
        approval_id: 12,
        agent_id: 8,
        summary: "Bash npm install left-pad --save",
        tier: "ask_user",
        route: "user",
      },
    },
    {
      ago: 20,
      kind: "agent.state_changed",
      data: { agent_id: 8, state: "waiting_approval", from: "working" },
    },
    {
      ago: 15,
      kind: "approval.routed",
      data: {
        approval_id: 31,
        route: "chief",
        tier: "chief",
        run_id: 10,
        agent_id: 25,
        summary: "Bash cargo build --release",
        reason: "runs a build inside the task's worktree",
      },
    },
    { ago: 10, kind: "run.state_changed", data: { run_id: 13, from: "running", to: "reporting" } },
    {
      ago: 2,
      kind: "agent.started",
      data: { agent_id: 3, workspace_id: 9, role: "user", session_id: "5504b543-…0003" },
    },
  ];
  history.sort((a, b) => b.ago - a.ago);

  return {
    now,
    projects,
    workspaces,
    agents,
    approvals,
    runs,
    tasks,
    questions,
    limits,
    reports,
    screens,
    history,
  };
}
