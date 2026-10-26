// The fake daemon's Chiefs of Staff (docs/chief-of-staff.md): one per
// project. cos's (agent 40) has a conversation in which it started agents 7
// and 8; opal-app's hasn't started. Messaging a chief appends to its
// transcript and logs `agent.transcript`, and with simulation on it answers
// a moment later. A paused chief keeps its updates waiting until it's
// resumed. Transcripts carry how full each chief's context is.

import type { DaemonResponse } from "../types";
import { FakeFiles } from "./attachments";
import type { FakeAgent, Raw } from "./fixtures";
import type { ScreenSpec } from "./screen";

/** What the desk needs from the fake daemon. */
export interface StaffHost {
  readonly projects: Raw[];
  readonly agents: FakeAgent[];
  readonly simulate: boolean;
  /** Uploaded attachments (fake/attachments.ts). */
  readonly files: FakeFiles;
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
2. **Trust the folder in porto.** The parser agent there is waiting on Claude's trust dialog.
3. **Finish the release notes.** They're drafted in \`CHANGELOG.md\` under 0.2.0, but they don't mention the login fix yet.

Want me to start an agent on the release notes while you review lisbon?`;

const update = `[cos · 2 updates]
- Agent 7 (Fix login bug) finished a turn: Sessions were expiring after 5 minutes because TTL_MS used minutes where it meant hours. I fixed the constant and added a test.
- Agent 8 (Add left-pad to the formatter) is waiting for your approval: npm install left-pad --save`;

/** A chief's window, as the daemon reads it from the model. */
const WINDOW = 200_000;

export function makeStaff(
  now: number,
  options: { paused?: boolean; contextPercent?: number } = {},
) {
  const ms = (agoSeconds: number) => now - agoSeconds * 1000;
  let n = 0;
  const text = (role: string, ago: number, body: string): Raw => ({
    n: ++n,
    role,
    kind: role === "user" && body.startsWith("[cos ·") ? "update" : "text",
    text: body,
    ts_ms: ms(ago),
  });
  const tool = (
    ago: number,
    name: string,
    summary: string,
    isError = false,
    agentId?: number,
  ): Raw => ({
    n: ++n,
    role: "assistant",
    kind: "tool",
    text: "",
    tool: { name, summary, is_error: isError, ...(agentId ? { agent_id: agentId } : {}) },
    ts_ms: ms(ago),
  });

  const transcript: Raw[] = [
    {
      ...text("user", 960, ask),
      attachments: [
        {
          name: "login-expired.svg",
          path: `${home}/worktrees/cos-1/chief/.cos/attachments/login-expired.svg`,
          content_type: "image/svg+xml",
          size_label: "2 KB",
        },
        {
          name: "auth.log",
          path: `${home}/worktrees/cos-1/chief/.cos/attachments/auth.log`,
          content_type: "text/plain",
          size_label: "2 KB",
        },
      ],
    },
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
    tool(905, "mcp__cos__agent_spawn", "Fix login bug", false, 7),
    tool(882, "mcp__cos__agent_spawn", "Add left-pad to the formatter", false, 8),
    text(
      "assistant",
      878,
      "Both agents are working in **lisbon**. I'll report back when they finish or need you.",
    ),
    text("user", 17, update),
    tool(15, "mcp__cos__agent_read", "agent 7 · idle", false, 7),
    tool(13, "Bash", "git log --oneline origin/main..cos/lisbon"),
    tool(
      11,
      "mcp__cos__agent_send",
      "agent 7: run the whole suite before I call it ready",
      false,
      7,
    ),
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
        paused: options.paused ?? false,
        updates_waiting: options.paused ? 3 : 0,
      },
      { project_id: 2, agent_id: null, model: null, brief: "", paused: false, updates_waiting: 0 },
    ] as Raw[],
    transcripts: { 40: transcript, ...agentTranscripts(ms) } as Record<number, Raw[]>,
    contexts: {
      40: {
        used_tokens: Math.round((WINDOW * (options.contextPercent ?? 42)) / 100) + 123,
        window_tokens: WINDOW,
      },
    } as Record<number, Raw>,
    inboxes: { 1: inbox } as Record<number, Raw[]>,
    memories: { 1: memory(ms) } as Record<number, Raw[]>,
  };
}

/** What cos's chief keeps in its memory notes. */
function memory(ms: (agoSeconds: number) => number): Raw[] {
  const note = (name: string, description: string, body: string, ago: number): Raw => ({
    name,
    description,
    body,
    created_ms: ms(ago + 86_400 * 3),
    updated_ms: ms(ago),
  });
  return [
    note(
      "release-process",
      "How a cos release is cut: version bump PR, then a v* tag pushed only with your OK",
      "1. Bump the version in a PR (`chore: version X.Y.Z`).\n2. Once it's merged, ask before pushing the `v*` tag: the tag starts the release workflow.\n3. See docs/releasing.md.",
      3_600 * 5,
    ),
    note(
      "testing",
      "Tests and lints run in Docker with make test; integration tests use cos-fake-claude",
      "`make test`, `make lint` and `make fmt` run in the cos-dev image. When a test needs new Claude behaviour, extend cos-fake-claude instead of calling the real CLI.",
      86_400,
    ),
    note(
      "user-preferences",
      "You like short replies, the chat as the main view, and PRs you can review the same day",
      "- Keep replies short and lead with the result.\n- Small PRs, one goal each.\n- Ask before anything that can't be undone, like a tag push.",
      600,
    ),
  ];
}

/** What agents 7 and 8, which the cos chief started in lisbon, have said so far. */
function agentTranscripts(ms: (agoSeconds: number) => number): Record<number, Raw[]> {
  const chat = (lines: [string, number, string, string?][]): Raw[] =>
    lines.map(([role, ago, body, tool], index) => ({
      n: index + 1,
      role: role === "tool" ? "assistant" : role,
      kind: role === "tool" ? "tool" : "text",
      text: role === "tool" ? "" : body,
      ...(role === "tool" ? { tool: { name: body, summary: tool ?? "", is_error: false } } : {}),
      ts_ms: ms(ago),
    }));
  return {
    8: chat([
      [
        "user",
        300,
        "Add left-pad to the formatter and use it for the table columns.\n\n[cos] The Chief of Staff of cos started you.",
      ],
      ["assistant", 296, "I'll look at how the formatter pads columns first, then add the helper."],
      ["tool", 292, "Read", "src/format/table.ts"],
      ["tool", 288, "Grep", "padStart · 3 matches"],
      [
        "assistant",
        60,
        "The table columns pad by hand in three places. `left-pad` does it in one call, so I'll install it and use it in `table.ts`.",
      ],
      ["tool", 21, "Bash", "npm install left-pad --save"],
    ]),
    7: chat([
      [
        "user",
        600,
        "Logins expire after five minutes. Find out why and fix it, with a regression test.\n\n[cos] The Chief of Staff of cos started you.",
      ],
      ["tool", 590, "Read", "src/auth/session.ts"],
      [
        "assistant",
        560,
        "Found it: `TTL_MS` is `5 * 60 * 1000`, five minutes, where the comment and the config both say five hours.",
      ],
      ["tool", 540, "Edit", "src/auth/session.ts · +1 −1"],
      ["tool", 500, "Write", "src/auth/session.test.ts"],
      [
        "assistant",
        460,
        "Fixed. Sessions now last five hours, and `session.test.ts` checks the expiry. Running the whole suite before I call it ready.",
      ],
      ["tool", 30, "Bash", "npm test -- --runInBand"],
    ]),
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

function replyWithFiles(text: string, attachments: Raw[]): string {
  const names = attachments.map((a) => `\`${a.name as string}\``).join(", ");
  const ask = text ? ` about *${text.replace(/\s+/g, " ").trim().slice(0, 60)}*` : "";
  return `I read ${names}${ask}.

I'll hand ${attachments.length === 1 ? "it" : "them"} to an agent in a new workspace with the ask, and report back when it's done.`;
}

/** The staff routes and every agent's transcript. */
export class FakeStaffDesk {
  readonly staffs: Raw[];
  readonly transcripts: Record<number, Raw[]>;
  readonly inboxes: Record<number, Raw[]>;
  /** Memory notes by project id. */
  readonly memories: Record<number, Raw[]>;
  /** `{used_tokens, window_tokens}` by agent id. */
  readonly contexts: Record<number, Raw>;

  constructor(
    private readonly host: StaffHost,
    fixtures: ReturnType<typeof makeStaff>,
  ) {
    this.staffs = fixtures.staffs;
    this.transcripts = fixtures.transcripts;
    this.inboxes = fixtures.inboxes;
    this.memories = fixtures.memories;
    this.contexts = fixtures.contexts;
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
        context: this.contexts[id] ?? null,
      });
    }
    p = path.match(/^\/v1\/projects\/(\d+)\/staff\/memory\/([^/]+)$/);
    if (p) return this.note(method, Number(p[1]), decodeURIComponent(p[2]!), b);
    p = path.match(/^\/v1\/projects\/(\d+)\/staff(?:\/([a-z-]+))?$/);
    if (!p) return null;
    const projectId = Number(p[1]);
    const action = p[2];
    const staff = this.staff(projectId);
    if (!action && method === "GET") return ok(this.json(staff));
    if (!action && method === "PUT") {
      if (typeof b.brief === "string") {
        const brief = b.brief.trim();
        if (new TextEncoder().encode(brief).length > 4096)
          throw new StaffError(400, "the brief is too long; keep it under 4096 bytes");
        staff.brief = brief;
      }
      if ("model" in b) staff.model = typeof b.model === "string" && b.model ? b.model : null;
      if (typeof b.paused === "boolean" && b.paused !== staff.paused) {
        staff.paused = b.paused;
        // Resumed: what waited goes in with the next wakeup.
        if (!b.paused && (staff.updates_waiting as number) > 0) {
          staff.updates_waiting = 0;
          this.host.log("staff.inbox", { project_id: projectId, count: 0 });
        }
      }
      this.host.log("staff.changed", { project_id: projectId });
      return ok(this.json(staff));
    }
    if (action === "start" && method === "POST") {
      this.start(staff);
      return ok(this.json(staff));
    }
    if (action === "new-session" && method === "POST") {
      this.newSession(staff);
      return ok(this.json(staff));
    }
    if (action === "message" && method === "POST") {
      const text = String(b.text ?? "").trim();
      const names = Array.isArray(b.attachments) ? b.attachments.map(String) : [];
      if (!text && names.length === 0) throw new StaffError(400, "the message is empty");
      const worktree = this.json(staff).worktree as string;
      const files = names.map((name) => this.host.files.find(worktree, name));
      this.message(staff, text, FakeFiles.block(files));
      return { status: 202, body: null };
    }
    if (action === "inbox" && method === "GET") {
      return ok({ items: this.inboxes[projectId] ?? [] });
    }
    if (action === "memory" && method === "GET") {
      const notes = [...(this.memories[projectId] ?? [])].sort((x, y) =>
        String(x.name).localeCompare(String(y.name)),
      );
      return ok({ notes });
    }
    throw new StaffError(404, `no route for ${method} ${path}`);
  }

  /** `PUT` or `DELETE …/staff/memory/{name}`, checked as the daemon does. */
  private note(method: string, projectId: number, name: string, b: Raw): DaemonResponse {
    this.staff(projectId);
    const notes = (this.memories[projectId] ??= []);
    const index = notes.findIndex((n) => n.name === name.trim());
    if (method === "DELETE") {
      if (index < 0) throw new StaffError(404, `there is no note "${name}"`);
      notes.splice(index, 1);
    } else if (method === "PUT") {
      const description = String(b.description ?? "").trim();
      const body = String(b.body ?? "").trim();
      if (!/^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$/.test(name.trim()))
        throw new StaffError(400, "a note's name is lowercase letters, digits and -");
      if (!description || description.includes("\n") || description.length > 200)
        throw new StaffError(400, "the description is one line of at most 200 characters");
      if (!body) throw new StaffError(400, "the body is empty; delete the note instead");
      if (index < 0 && notes.length >= 50)
        throw new StaffError(400, "the chief keeps at most 50 notes");
      const now = Date.now();
      const note: Raw = {
        name: name.trim(),
        description,
        body,
        created_ms: index < 0 ? now : notes[index]!.created_ms,
        updated_ms: now,
      };
      if (index < 0) notes.push(note);
      else notes[index] = note;
      this.host.log("staff.changed", { project_id: projectId });
      return { status: 200, body: JSON.parse(JSON.stringify(note)) as unknown };
    } else {
      throw new StaffError(404, `no route for ${method}`);
    }
    this.host.log("staff.changed", { project_id: projectId });
    return { status: 204, body: null };
  }

  /** The chief's worktree, where files attached for it go. */
  worktree(projectId: number): string {
    return this.json(this.staff(projectId)).worktree as string;
  }

  private staff(projectId: number): Raw {
    const project = this.host.projects.find((p) => p.id === projectId);
    if (!project) throw new StaffError(404, `no project ${projectId}`);
    let staff = this.staffs.find((s) => s.project_id === projectId);
    if (!staff) {
      staff = {
        project_id: projectId,
        agent_id: null,
        model: null,
        brief: "",
        paused: false,
        updates_waiting: 0,
      };
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
        (a) =>
          a.state === "waiting_approval" || a.state === "needs_trust" || a.state === "needs_login",
      ).length,
      paused: staff.paused === true,
      updates_waiting: (staff.updates_waiting as number | undefined) ?? 0,
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

  /** `POST /v1/agents/{id}/input` to an agent of yours: what you typed joins its transcript, and it answers. */
  agentInput(agent: FakeAgent, text: string): void {
    if (agent.role === "staff" || !text.trim()) return;
    this.append(agent, { role: "user", kind: "text", text });
    if (!this.host.simulate) return;
    this.host.later(1500, () =>
      this.append(agent, {
        role: "assistant",
        kind: "tool",
        text: "",
        tool: { name: "Read", summary: "the files that ask touches", is_error: false },
      }),
    );
    this.host.later(4800, () =>
      this.append(agent, {
        role: "assistant",
        kind: "text",
        text: "Done. I made that change and the tests still pass.",
      }),
    );
  }

  /** Drops its session: a new session id, an empty transcript, then a start. */
  private newSession(staff: Raw): void {
    const agent = this.agent(staff);
    if (agent) {
      Object.assign(agent, {
        state: "exited",
        session_id: `staff-${staff.project_id as number}-${this.host.id()}`,
        transcript_path: null,
      });
      this.transcripts[agent.id] = [];
      this.host.log("agent.transcript", { agent_id: agent.id, next: 0 });
    }
    this.start(staff);
  }

  private append(agent: FakeAgent, message: Raw): void {
    const list = (this.transcripts[agent.id] ??= []);
    const last = list[list.length - 1];
    const n = last ? (last.n as number) + 1 : 1;
    list.push({ n, ts_ms: Date.now(), ...message });
    agent.transcript_path ??= `/Users/me/.claude/projects/session-${agent.id}.jsonl`;
    this.host.log("agent.transcript", { agent_id: agent.id, next: n });
  }

  private message(staff: Raw, text: string, attachments: Raw[] = []): void {
    const agent = this.start(staff);
    const host = this.host;
    this.append(agent, {
      role: "user",
      kind: "text",
      text,
      ...(attachments.length ? { attachments } : {}),
    });
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
      // Each exchange fills a little more of its context.
      const context = (this.contexts[agent.id] ??= { used_tokens: 18_000, window_tokens: WINDOW });
      context.used_tokens = Math.min(WINDOW, (context.used_tokens as number) + 2_400);
      const answer = attachments.length ? replyWithFiles(text, attachments) : reply(text);
      this.append(agent, { role: "assistant", kind: "text", text: answer });
      agent.last_message = answer;
      host.screens[agent.id] = { prompt: text, transcript: "notes", footer: { kind: "idle" } };
      host.setAgentState(agent, "idle");
    });
  }
}
