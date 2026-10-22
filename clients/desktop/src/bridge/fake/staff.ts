// The fake daemon's Chiefs of Staff (docs/chief-of-staff.md): one per
// project. cos's (agent 40) has a conversation in which it started agents 7
// and 8; opal-app's hasn't started. Messaging a chief appends to its
// transcript and logs `agent.transcript`, and with simulation on it answers
// a moment later.

import type { DaemonResponse } from "../types";
import type { FakeAgent, Raw } from "./fixtures";
import type { ScreenSpec } from "./screen";

/** What the desk needs from the fake daemon. */
export interface StaffHost {
  readonly projects: Raw[];
  readonly agents: FakeAgent[];
  readonly simulate: boolean;
  readonly screens: Record<number, Omit<ScreenSpec, "cols" | "rows" | "cwd">>;
  id(): number;
  log(kind: string, data: Raw): void;
  later(ms: number, action: () => void): void;
  setAgentState(agent: FakeAgent, state: string): void;
}

export class StaffError {
  constructor(
    readonly status: number,
    readonly message: string,
  ) {}
}

const home = "/Users/me/.cos";

function slug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

const ask =
  "Logins expire after five minutes, and the formatter needs left-pad. Can you get agents on both, and tell me what's left before the 0.2 release?";

const answer = `Here's where things stand.

**The login bug is fixed.** Sessions expired after five minutes because \`TTL_MS\` was counted in minutes where it meant hours. The agent fixed the constant and added a regression test; it's running the whole suite now, and the branch is \`cos/lisbon\`.

**left-pad is waiting on you.** The formatter agent wants to run \`npm install left-pad --save\`. Allow it below, or tell me to have it write the helper by hand instead.

Before the 0.2 release, three things are left:

1. **Review and merge lisbon.** One commit so far, and the new test covers the expiry.
2. **Approve the plan in porto.** The run there split the streaming tokenizer into four tasks and is waiting for you.
3. **Finish the release notes.** They're drafted in \`CHANGELOG.md\` under 0.2.0, but they don't mention the login fix yet.

Want me to start an agent on the release notes while you review lisbon?`;

const update = `[cos · 2 updates]
- Agent 7 (Fix login bug) finished a turn: Sessions were expiring after 5 minutes because TTL_MS used minutes where it meant hours. I fixed the constant and added a test.
- Agent 8 (Add left-pad to the formatter) is waiting for your approval: npm install left-pad --save`;

export function makeStaff(now: number) {
  const ms = (agoSeconds: number) => now - agoSeconds * 1000;
  let n = 0;
  const text = (role: string, ago: number, body: string): Raw => ({
    n: ++n,
    role,
    kind: role === "user" && body.startsWith("[cos ·") ? "update" : "text",
    text: body,
    ts_ms: ms(ago),
  });
  const tool = (ago: number, name: string, summary: string, isError = false): Raw => ({
    n: ++n,
    role: "assistant",
    kind: "tool",
    text: "",
    tool: { name, summary, is_error: isError },
    ts_ms: ms(ago),
  });

  const transcript: Raw[] = [
    text("user", 960, ask),
    text(
      "assistant",
      950,
      "I'll check what's already running first, so the two jobs don't land on top of something else.",
    ),
    tool(948, "mcp__cos__workspaces_list", "4 workspaces · 6 agents"),
    text(
      "assistant",
      930,
      "Nothing touches the login code or the formatter. Both fit in **lisbon**, which is already on the auth branch, so I'll give each its own agent there.",
    ),
    tool(905, "mcp__cos__agent_spawn", "Fix login bug · in lisbon (opus)"),
    tool(882, "mcp__cos__agent_spawn", "Add left-pad to the formatter · in lisbon"),
    text(
      "assistant",
      878,
      "Both agents are working in **lisbon**. I'll report back when they finish or need you.",
    ),
    text("user", 17, update),
    tool(15, "mcp__cos__agent_read", "agent 7 · idle"),
    tool(13, "Bash", "git log --oneline origin/main..cos/lisbon"),
    tool(11, "mcp__cos__agent_send", "agent 7: run the whole suite before I call it ready"),
    text("assistant", 8, answer),
  ];

  const inbox: Raw[] = [
    {
      id: 2,
      kind: "approval",
      agent_id: 8,
      agent_title: "Add left-pad to the formatter",
      text: "Waiting for your approval: npm install left-pad --save",
      ts_ms: ms(19),
    },
    {
      id: 1,
      kind: "stop",
      agent_id: 7,
      agent_title: "Fix login bug",
      text: "Sessions were expiring after 5 minutes because TTL_MS used minutes where it meant hours. I fixed the constant and added a test.",
      ts_ms: ms(44),
    },
  ];

  const agent: FakeAgent = {
    id: 40,
    workspace_id: null,
    run_id: null,
    task_id: null,
    role: "staff",
    title: "Chief of Staff",
    session_id: "5504b543-4897-4751-add5-f81f24a60040",
    pid: 25_540,
    state: "idle",
    model: "opus",
    permission_mode: "default",
    user_settings: true,
    rows: 34,
    cols: 110,
    started_ms: ms(1000),
    last_activity_ms: ms(8),
    exited_ms: null,
    exit_code: null,
    last_message: answer,
    pending_approval_id: null,
    cwd: `${home}/worktrees/cos-1/chief`,
    spawned_by: null,
    transcript_path: "/Users/me/.claude/projects/session-40.jsonl",
  };

  return {
    agent,
    screen: { prompt: ask, transcript: "notes", footer: { kind: "idle" } } as const,
    staffs: [
      {
        project_id: 1,
        agent_id: 40,
        model: "opus",
        brief: "Ship 0.2: login fix, left-pad, release notes.",
      },
      { project_id: 2, agent_id: null, model: null, brief: "" },
    ] as Raw[],
    transcripts: { 40: transcript } as Record<number, Raw[]>,
    inboxes: { 1: inbox } as Record<number, Raw[]>,
  };
}

function reply(text: string): string {
  const short = text.replace(/\s+/g, " ").trim();
  const quoted = short.length > 80 ? `${short.slice(0, 77)}…` : short;
  return `Got it: *${quoted}*

I'll give this its own workspace so it doesn't collide with **lisbon**:

1. **Create a workspace** from \`origin/main\`.
2. **Start an agent there** with the ask above and a clear finish line.
3. **Report back** when it finishes, or sooner if it needs you.`;
}

/** The staff routes and every agent's transcript. */
export class FakeStaffDesk {
  readonly staffs: Raw[];
  readonly transcripts: Record<number, Raw[]>;
  readonly inboxes: Record<number, Raw[]>;

  constructor(
    private readonly host: StaffHost,
    fixtures: ReturnType<typeof makeStaff>,
  ) {
    this.staffs = fixtures.staffs;
    this.transcripts = fixtures.transcripts;
    this.inboxes = fixtures.inboxes;
  }

  /** The answer, or null when the route isn't the desk's. */
  route(method: string, path: string, q: URLSearchParams, b: Raw): DaemonResponse | null {
    const ok = (body: unknown, status = 200): DaemonResponse => ({
      status,
      body: JSON.parse(JSON.stringify(body)) as unknown,
    });
    let p = path.match(/^\/v1\/agents\/(\d+)\/transcript$/);
    if (p && method === "GET") {
      const id = Number(p[1]);
      if (!this.host.agents.some((a) => a.id === id)) throw new StaffError(404, `no agent ${id}`);
      const after = Number(q.get("after") ?? 0);
      const all = this.transcripts[id] ?? [];
      const last = all[all.length - 1];
      return ok({
        messages: all.filter((m) => (m.n as number) > after),
        next: last ? (last.n as number) : 0,
      });
    }
    p = path.match(/^\/v1\/projects\/(\d+)\/staff(?:\/([a-z]+))?$/);
    if (!p) return null;
    const projectId = Number(p[1]);
    const action = p[2];
    const staff = this.staff(projectId);
    if (!action && method === "GET") return ok(this.json(staff));
    if (!action && method === "PUT") {
      if ("model" in b) staff.model = typeof b.model === "string" && b.model ? b.model : null;
      this.host.log("staff.changed", { project_id: projectId });
      return ok(this.json(staff));
    }
    if (action === "start" && method === "POST") {
      this.start(staff);
      return ok(this.json(staff));
    }
    if (action === "message" && method === "POST") {
      const text = String(b.text ?? "").trim();
      if (!text) throw new StaffError(400, "text can't be empty");
      this.message(staff, text);
      return { status: 202, body: null };
    }
    if (action === "inbox" && method === "GET") {
      return ok({ items: this.inboxes[projectId] ?? [] });
    }
    throw new StaffError(404, `no route for ${method} ${path}`);
  }

  private staff(projectId: number): Raw {
    const project = this.host.projects.find((p) => p.id === projectId);
    if (!project) throw new StaffError(404, `no project ${projectId}`);
    let staff = this.staffs.find((s) => s.project_id === projectId);
    if (!staff) {
      staff = { project_id: projectId, agent_id: null, model: null, brief: "" };
      this.staffs.push(staff);
    }
    return staff;
  }

  json(staff: Raw): Raw {
    const project = this.host.projects.find((p) => p.id === staff.project_id)!;
    const name = slug(project.name as string);
    const agent = this.agent(staff);
    const spawned = agent ? this.host.agents.filter((a) => a.spawned_by === agent.id) : [];
    return {
      project_id: staff.project_id,
      agent_id: staff.agent_id,
      state: agent?.state ?? null,
      model: staff.model,
      brief: staff.brief,
      worktree: `${home}/worktrees/${name}-${project.id as number}/chief`,
      branch: `${name}/chief`,
      working_agents: spawned.filter((a) => a.state === "working" || a.state === "starting").length,
      waiting_on_you: spawned.filter(
        (a) => a.state === "waiting_approval" || a.state === "needs_trust",
      ).length,
    };
  }

  private agent(staff: Raw): FakeAgent | undefined {
    return this.host.agents.find((a) => a.id === staff.agent_id);
  }

  /** Starts it the first time, resumes it after an exit, else does nothing. */
  private start(staff: Raw): FakeAgent {
    const existing = this.agent(staff);
    const host = this.host;
    if (existing && existing.state !== "exited" && existing.state !== "interrupted")
      return existing;
    const json = this.json(staff);
    const agent: FakeAgent = existing ?? {
      id: host.id(),
      workspace_id: null,
      run_id: null,
      task_id: null,
      role: "staff",
      title: "Chief of Staff",
      session_id: `staff-${staff.project_id as number}`,
      state: "starting",
      model: staff.model ?? null,
      permission_mode: "default",
      user_settings: true,
      rows: 34,
      cols: 110,
      started_ms: Date.now(),
      cwd: json.worktree,
      spawned_by: null,
      transcript_path: null,
    };
    Object.assign(agent, {
      state: "starting",
      pid: 32_000 + agent.id,
      started_ms: Date.now(),
      exited_ms: null,
      exit_code: null,
    });
    if (!existing) host.agents.push(agent);
    staff.agent_id = agent.id;
    host.screens[agent.id] ??= { prompt: "", transcript: "blank", footer: { kind: "idle" } };
    this.transcripts[agent.id] ??= [];
    host.log("agent.started", {
      agent_id: agent.id,
      workspace_id: null,
      role: "staff",
      session_id: agent.session_id,
    });
    host.log("staff.started", { project_id: staff.project_id, agent_id: agent.id });
    // Without simulation nothing moves by itself; it's ready at once.
    if (host.simulate) host.later(700, () => host.setAgentState(agent, "idle"));
    else agent.state = "idle";
    return agent;
  }

  private append(agent: FakeAgent, message: Raw): void {
    const list = (this.transcripts[agent.id] ??= []);
    const last = list[list.length - 1];
    const n = last ? (last.n as number) + 1 : 1;
    list.push({ n, ts_ms: Date.now(), ...message });
    agent.transcript_path ??= `/Users/me/.claude/projects/session-${agent.id}.jsonl`;
    this.host.log("agent.transcript", { agent_id: agent.id, next: n });
  }

  private message(staff: Raw, text: string): void {
    const agent = this.start(staff);
    const host = this.host;
    this.append(agent, { role: "user", kind: "text", text });
    host.screens[agent.id] = {
      prompt: text,
      transcript: "blank",
      footer: { kind: "working", verb: "Coordinating", seconds: 0 },
    };
    host.later(250, () => host.setAgentState(agent, "working"));
    host.later(1200, () =>
      this.append(agent, {
        role: "assistant",
        kind: "tool",
        text: "",
        tool: {
          name: "mcp__cos__workspaces_list",
          summary: "workspaces and their agents",
          is_error: false,
        },
      }),
    );
    host.later(2600, () => {
      this.append(agent, { role: "assistant", kind: "text", text: reply(text) });
      agent.last_message = reply(text);
      host.screens[agent.id] = { prompt: text, transcript: "notes", footer: { kind: "idle" } };
      host.setAgentState(agent, "idle");
    });
  }
}
