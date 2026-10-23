// Canned daemon data in the daemon's own JSON (snake_case), shaped as in
// docs/daemon.md and docs/agents-api.md:
//
// - two projects: cos (lisbon, porto, madrid, oslo) and opal-app (nairobi, kyoto),
//   with branches `<project>/<city>`; porto and kyoto have no title
// - your agents in every state
// - cos's Chief of Staff (agent 40), which started agents 7 and 8, with a
//   conversation (fake/staff.ts); opal-app's chief hasn't started
// - a pending approval waiting for you
import type { ScreenSpec } from "./screen";

export type Raw = Record<string, unknown>;

export interface FakeAgent extends Raw {
  id: number;
  /** Null for a Chief of Staff, which works in its own worktree. */
  workspace_id: number | null;
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
      role,
      title,
      session_id: `5504b543-4897-4751-add5-f81f24a6${String(id).padStart(4, "0")}`,
      pid: exited ? null : (pid += 7),
      state,
      model: options.model === undefined ? "sonnet" : options.model,
      permission_mode: options.mode ?? "default",
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
        reason: "it changes or downloads dependencies (npm install)",
      },
    ),
  ];

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
      ago: 900,
      kind: "agent.started",
      data: { agent_id: 7, workspace_id: 3, role: "user", session_id: "5504b543-…0007" },
    },
    {
      ago: 300,
      kind: "agent.started",
      data: { agent_id: 8, workspace_id: 3, role: "user", session_id: "5504b543-…0008" },
    },
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
      },
    },
    {
      ago: 20,
      kind: "agent.state_changed",
      data: { agent_id: 8, state: "waiting_approval", from: "working" },
    },
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
    screens,
    history,
  };
}
