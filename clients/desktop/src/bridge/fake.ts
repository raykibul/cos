// The fake bridge: the same interface as tauri.ts, backed by an in-memory
// daemon with canned data (fake/fixtures.ts). Actions change its state and
// log events like the real daemon, so the UI behaves live in a plain
// browser: a new agent starts and goes to work, a new run plans, approving
// a plan starts its tasks. Terminals replay a Claude Code screen.

import { toBase64 } from "../api/client";
import { makeFixtures, type FakeAgent, type Fixtures, type Raw } from "./fake/fixtures";
import {
  type FakeHost,
  firewallReason,
  makeDevices,
  makeHosts,
  makePeers,
  refusalOf,
  tailnetStatus,
  tailscaleReason,
} from "./fake/remote";
import { type Footer, type Screen, inputUpdate, renderScreen, spinnerUpdate } from "./fake/screen";
import { FakeEnv, makeEnv } from "./fake/env";
import { FakeStaffDesk, StaffError, makeStaff } from "./fake/staff";
import type {
  Bridge,
  BridgeError,
  DaemonInfo,
  DaemonResponse,
  EventMessage,
  Host,
  HostList,
  HttpMethod,
  RawTailnetPeers,
  TerminalMessage,
} from "./types";

export interface FakeOptions {
  /** How long each request takes, in ms. */
  latencyMs?: number;
  /** Whether agents and runs move by themselves over time. */
  simulate?: boolean;
  /** Start with the daemon stopped. */
  running?: boolean;
  now?: number;
  /** The host in use at the start (`local`, `h1` Studio, `h2` Travel laptop). */
  host?: string;
  /** This Mac's tailnet listener reports the macOS firewall blocking it. */
  firewall?: boolean;
  /** Tailscale is off on this Mac: the tailnet's devices can't be listed. */
  tailscaleOff?: boolean;
  /** How long a refused or unreachable stream waits before trying again. */
  retryMs?: number;
  /**
   * With `simulate`, how long after it's first tried Studio approves this
   * Mac.
   */
  approveAfterMs?: number;
}

interface Follower {
  id: number;
  onMessage: (message: EventMessage) => void;
  lastSeq: number;
}

interface TerminalStream {
  id: number;
  agentId: number;
  onMessage: (message: TerminalMessage) => void;
  screen: Screen | null;
  input: string;
  timer: ReturnType<typeof setInterval> | null;
  seconds: number;
}

class HttpError {
  constructor(
    readonly status: number,
    readonly message: string,
  ) {}
}

const cities = [
  "lima",
  "quito",
  "accra",
  "hanoi",
  "tunis",
  "riga",
  "porto",
  "bern",
  "cork",
  "turin",
];
const encoder = new TextEncoder();
const b64 = (text: string) => toBase64(encoder.encode(text));

function copy<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** An in-memory daemon behind the fake bridge. */
export class FakeDaemon {
  readonly fixtures: Fixtures;
  readonly projects: Raw[];
  readonly workspaces: Raw[];
  readonly agents: FakeAgent[];
  readonly approvals: Raw[];
  readonly runs: Raw[];
  readonly tasks: Raw[];
  readonly questions: Raw[];
  readonly limits: Record<number, Raw>;
  readonly reports: Record<number, string>;
  /** Devices that tried this daemon's tailnet listener. */
  readonly devices: Raw[];
  tailnet: Raw;
  /** The other devices on the tailnet, for `GET /v1/tailnet/peers` (port 7700). */
  peers: Raw[];
  /** Why Tailscale can't be asked for them; the peers are then empty. */
  peersReason: string | null;
  /** Reached over the tailnet: device and listener routes answer `host_only`. */
  readonly remote: boolean;
  readonly events: Raw[] = [];
  /** Every request, for tests: `GET /v1/projects`. */
  readonly requests: { method: string; path: string; body: unknown }[] = [];
  running: boolean;
  readonly simulate: boolean;
  private readonly startedAt: string;
  private nextId = 200;
  private readonly timers = new Set<ReturnType<typeof setTimeout>>();
  readonly followers = new Map<number, Follower>();
  readonly terminals = new Map<number, TerminalStream>();
  /** The Chiefs of Staff and transcripts (fake/staff.ts). */
  readonly staffDesk: FakeStaffDesk;
  /** Environment variables for agents (fake/env.ts). */
  readonly env: FakeEnv;

  constructor(options: FakeOptions = {}, remote = false) {
    this.fixtures = makeFixtures(options.now);
    this.remote = remote;
    this.devices = remote ? [] : makeDevices(this.fixtures.now);
    this.tailnet = tailnetStatus(!remote, 7700, options.firewall ? firewallReason : null);
    this.peersReason = options.tailscaleOff ? tailscaleReason : null;
    this.peers = options.tailscaleOff ? [] : makePeers(this.fixtures.now);
    const f = copy({ ...this.fixtures, screens: undefined });
    this.projects = f.projects;
    this.workspaces = f.workspaces;
    this.agents = f.agents;
    this.approvals = f.approvals;
    this.runs = f.runs;
    this.tasks = f.tasks;
    this.questions = f.questions;
    this.limits = f.limits;
    this.reports = f.reports;
    this.running = options.running ?? true;
    this.simulate = options.simulate ?? true;
    this.startedAt = new Date(this.fixtures.now - 90_100_000).toISOString();
    const staff = makeStaff(this.fixtures.now);
    this.agents.push(staff.agent);
    this.fixtures.screens[staff.agent.id] = staff.screen;
    this.staffDesk = new FakeStaffDesk(this, staff);
    this.env = new FakeEnv(this, makeEnv());
    for (const item of this.fixtures.history) {
      this.events.push({
        seq: this.events.length + 1,
        ts: new Date(this.fixtures.now - item.ago * 1000).toISOString(),
        kind: item.kind,
        run_id: item.data.run_id !== undefined ? String(item.data.run_id) : null,
        data: item.data,
      });
    }
  }

  get lastSeq(): number {
    return this.events.length;
  }

  status(): Raw {
    return {
      version: "0.1.0",
      pid: 4242,
      started_at: this.startedAt,
      home: "/Users/me/.cos",
      events: this.events.length,
      last_seq: this.lastSeq,
      tailnet: copy(this.tailnet),
    };
  }

  /** Logs an event and sends it to every follower. */
  log(kind: string, data: Raw): void {
    const runId = data.run_id;
    const event = {
      seq: this.events.length + 1,
      ts: new Date().toISOString(),
      kind,
      run_id: runId !== undefined && runId !== null ? String(runId) : null,
      data,
    };
    this.events.push(event);
    for (const follower of this.followers.values()) {
      follower.lastSeq = event.seq;
      follower.onMessage({ type: "event", event: copy(event) });
    }
  }

  later(ms: number, action: () => void): void {
    if (!this.simulate) return;
    const timer = setTimeout(() => {
      this.timers.delete(timer);
      if (this.running) action();
    }, ms);
    this.timers.add(timer);
  }

  dispose(): void {
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    for (const stream of this.terminals.values()) if (stream.timer) clearInterval(stream.timer);
    this.terminals.clear();
    this.followers.clear();
  }

  /** Stops or starts the daemon, as `cos daemon` would. */
  setRunning(running: boolean): void {
    if (running === this.running) return;
    this.running = running;
    if (!running) {
      for (const follower of this.followers.values()) {
        follower.onMessage({
          type: "disconnected",
          reason: "The cos daemon is not running (nothing answers on /Users/me/.cos/cos.sock).",
          notRunning: true,
        });
      }
      for (const stream of this.terminals.values()) {
        stream.onMessage({ type: "disconnected", reason: "The cos daemon stopped." });
      }
    } else {
      for (const follower of this.followers.values()) {
        follower.onMessage({ type: "connected", status: this.status() });
        for (const event of this.events.filter((e) => (e.seq as number) > follower.lastSeq)) {
          follower.onMessage({ type: "event", event: copy(event) });
        }
        follower.lastSeq = this.lastSeq;
      }
      for (const stream of this.terminals.values()) this.sendSnapshot(stream);
    }
  }

  // MARK: Requests

  handle(method: HttpMethod, rawPath: string, body: unknown): DaemonResponse {
    this.requests.push({ method, path: rawPath, body });
    const url = new URL(rawPath, "http://cos");
    const path = url.pathname;
    const q = url.searchParams;
    const b = (typeof body === "object" && body !== null ? body : {}) as Raw;
    try {
      const result = this.route(method, path, q, b);
      if (result === undefined) return { status: 204, body: null };
      return result;
    } catch (error) {
      if (error instanceof HttpError || error instanceof StaffError)
        return { status: error.status, body: { error: error.message } };
      throw error;
    }
  }

  get screens(): Fixtures["screens"] {
    return this.fixtures.screens;
  }

  private route(
    method: string,
    path: string,
    q: URLSearchParams,
    b: Raw,
  ): DaemonResponse | undefined {
    const ok = (body: unknown, status = 200): DaemonResponse => ({ status, body: copy(body) });
    const m = (pattern: RegExp) => path.match(pattern);
    let p: RegExpMatchArray | null;

    if (method === "GET" && path === "/v1/status") return ok(this.status());
    if (method === "GET" && path === "/v1/events") {
      const after = Number(q.get("after") ?? 0);
      const limit = Math.min(Number(q.get("limit") ?? 100), 1000);
      return ok({ events: this.events.filter((e) => (e.seq as number) > after).slice(0, limit) });
    }
    const staff = this.staffDesk.route(method, path, q, b);
    if (staff) return staff;
    const env = this.env.route(method, path, b);
    if (env) return env;

    // Projects and workspaces.
    if (method === "GET" && path === "/v1/projects") {
      return ok({ projects: this.projects.map((project) => this.projectJson(project)) });
    }
    if (method === "POST" && path === "/v1/projects") return this.addProject(b);
    if ((p = m(/^\/v1\/projects\/(\d+)$/)) && method === "DELETE")
      return this.removeProject(Number(p[1]));
    if ((p = m(/^\/v1\/projects\/(\d+)\/workspaces$/)) && method === "POST") {
      return this.createWorkspace(Number(p[1]), b);
    }
    if ((p = m(/^\/v1\/workspaces\/(\d+)$/))) {
      const ws = this.find(this.workspaces, Number(p[1]), "workspace");
      if (method === "PATCH") {
        const title = String(b.title ?? "").trim();
        if (!title) throw new HttpError(400, "title can't be empty");
        ws.title = title;
        this.log("workspace.renamed", { workspace_id: ws.id, project_id: ws.project_id, title });
        return ok(ws);
      }
      if (method === "DELETE") return this.removeWorkspace(ws);
    }
    if ((p = m(/^\/v1\/workspaces\/(\d+)\/agents$/)) && method === "POST") {
      return this.createAgent(Number(p[1]), b);
    }

    // Agents.
    if (method === "GET" && path === "/v1/agents") {
      let list = [...this.agents].sort(
        (x, y) => (y.started_ms as number) - (x.started_ms as number),
      );
      const ws = q.get("workspace_id");
      const state = q.get("state");
      if (ws) list = list.filter((a) => a.workspace_id === Number(ws));
      if (state) list = list.filter((a) => a.state === state);
      for (const agent of list) this.env.stamp(agent);
      return ok({ agents: list });
    }
    if ((p = m(/^\/v1\/agents\/(\d+)(?:\/([a-z]+))?$/))) {
      const agent = this.find(this.agents, Number(p[1]), "agent") as FakeAgent;
      const action = p[2];
      if (!action && method === "GET") return ok(this.env.stamp(agent));
      if (!action && method === "DELETE") {
        if (this.alive(agent))
          throw new HttpError(409, "the agent is still running; stop it first");
        this.agents.splice(this.agents.indexOf(agent), 1);
        this.log("agent.removed", { agent_id: agent.id });
        return undefined;
      }
      if (method === "POST" && action) return this.agentAction(agent, action, b);
      if (method === "GET" && action === "screen") {
        return ok({ rows: agent.rows, cols: agent.cols, text: "", cursor: [0, 0] });
      }
    }

    // Approvals.
    if (method === "GET" && path === "/v1/approvals") {
      const state = q.get("state");
      const list = this.approvals
        .filter((a) => !state || a.state === state)
        .sort((x, y) => (y.created_ms as number) - (x.created_ms as number));
      return ok({ approvals: list });
    }
    if ((p = m(/^\/v1\/approvals\/(\d+)(\/chief)?$/))) {
      const approval = this.find(this.approvals, Number(p[1]), "approval");
      if (method === "GET" && !p[2]) return ok(approval);
      if (method === "POST") return this.decide(approval, b, p[2] ? "chief" : "user");
    }

    // Runs.
    if (method === "GET" && path === "/v1/runs") {
      return ok({
        runs: [...this.runs]
          .sort((x, y) => (y.id as number) - (x.id as number))
          .map((r) => ({ ...r, tasks: this.counts(r.id as number) })),
      });
    }
    if (method === "POST" && path === "/v1/runs") return this.createRun(b);
    if ((p = m(/^\/v1\/runs\/(\d+)(?:\/([a-z-]+))?$/))) {
      const run = this.find(this.runs, Number(p[1]), "run");
      const action = p[2];
      if (!action && method === "GET") return ok(this.board(run));
      if (action === "approve-plan" && method === "POST") return this.approvePlan(run);
      if (action === "cancel" && method === "POST")
        return this.finishRun(run, "cancelled", String(b.reason ?? "") || null);
      if (action === "message" && method === "POST") {
        if (!String(b.text ?? "").trim()) throw new HttpError(400, "text can't be empty");
        return { status: 202, body: null };
      }
      if (action === "report" && method === "GET") {
        const report = this.reports[run.id as number];
        if (!report) throw new HttpError(404, "the report hasn't been written yet");
        return ok({ markdown: report });
      }
      if (action === "limits") {
        const limits = this.limits[run.id as number] ?? {
          preset: run.preset,
          approval_cap: run.approval_cap,
          chief_approvals: 0,
        };
        if (method === "GET") return ok(limits);
        if (method === "PUT") {
          if (["done", "failed", "cancelled"].includes(run.state as string))
            throw new HttpError(409, "the run is finished");
          if (b.preset) limits.preset = b.preset;
          if (b.approval_cap !== undefined) {
            if (Number(b.approval_cap) > 1000)
              throw new HttpError(400, "approval_cap can't be over 1000");
            limits.approval_cap = Number(b.approval_cap);
          }
          this.limits[run.id as number] = limits;
          run.preset = limits.preset;
          run.approval_cap = limits.approval_cap;
          this.log("run.limits_changed", { run_id: run.id, ...limits });
          return ok(limits);
        }
      }
    }
    if ((p = m(/^\/v1\/tasks\/(\d+)$/)) && method === "PATCH") {
      return this.updateTask(this.find(this.tasks, Number(p[1]), "task"), b);
    }

    // Questions.
    if (method === "GET" && path === "/v1/questions") {
      const state = q.get("state");
      return ok({ questions: this.questions.filter((x) => !state || x.state === state) });
    }
    if ((p = m(/^\/v1\/questions\/(\d+)\/answer$/)) && method === "POST") {
      const question = this.find(this.questions, Number(p[1]), "question");
      if (question.state !== "open") throw new HttpError(409, "this question was already answered");
      const answer = String(b.answer ?? "").trim();
      if (!answer) throw new HttpError(400, "answer can't be empty");
      Object.assign(question, { state: "answered", answer, answered_ms: Date.now() });
      this.log("question.answered", { question_id: question.id, run_id: question.run_id });
      return ok(question);
    }

    // Devices and the tailnet: on the socket only.
    if (path.startsWith("/v1/tailnet") || path.startsWith("/v1/devices")) {
      return this.remoteAccess(method, path, q, b);
    }

    throw new HttpError(404, `no route for ${method} ${path}`);
  }

  private remoteAccess(
    method: string,
    path: string,
    q: URLSearchParams,
    b: Raw,
  ): DaemonResponse | undefined {
    if (this.remote) {
      return {
        status: 403,
        body: { error: "remote access is managed on the host itself", code: "host_only" },
      };
    }
    const ok = (body: unknown): DaemonResponse => ({ status: 200, body: copy(body) });
    if (method === "GET" && path === "/v1/tailnet") return ok(this.tailnet);
    if (method === "GET" && path === "/v1/tailnet/peers") {
      const port = Number(q.get("port") ?? 7700);
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new HttpError(400, `port must be between 1 and 65535, not ${q.get("port")}`);
      }
      const peers = this.peers.map((peer) => ({
        ...peer,
        address: String(peer.address).replace(/:\d+$/, `:${port}`),
      }));
      return ok({
        peers,
        self_name: this.peersReason ? null : "my-mac",
        reason: this.peersReason,
      });
    }
    if (method === "PUT" && path === "/v1/tailnet") {
      const port = b.port === undefined ? (this.tailnet.port as number) : Number(b.port);
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new HttpError(400, `port must be between 1 and 65535, not ${String(b.port)}`);
      }
      const reason = (this.tailnet.reason as string | undefined) ?? null;
      this.tailnet = tailnetStatus(Boolean(b.enabled), port, reason);
      this.log("tailnet.changed", copy(this.tailnet));
      return ok(this.tailnet);
    }
    if (method === "GET" && path === "/v1/devices") return ok({ devices: this.devices });
    const p = path.match(/^\/v1\/devices\/(\d+)(?:\/(approve|deny))?$/);
    if (!p) throw new HttpError(404, `no route for ${method} ${path}`);
    const device = this.find(this.devices, Number(p[1]), "device");
    const data = { device_id: device.id, node_id: device.node_id, name: device.name };
    if (method === "DELETE" && !p[2]) {
      this.devices.splice(this.devices.indexOf(device), 1);
      this.log("device.removed", data);
      return undefined;
    }
    if (method === "POST" && p[2]) {
      device.state = p[2] === "approve" ? "approved" : "denied";
      device.decided_at = new Date().toISOString();
      this.log(p[2] === "approve" ? "device.approved" : "device.denied", data);
      return ok(device);
    }
    throw new HttpError(404, `no route for ${method} ${path}`);
  }

  /** A new device asks for access, as a first connection does. */
  requestDevice(name: string, hostname: string, os = "macOS"): Raw {
    const now = new Date().toISOString();
    const device: Raw = {
      id: this.id(),
      node_id: `n${name}CNTRL`,
      name,
      hostname,
      os,
      login_name: "me@example.com",
      state: "pending",
      last_ip: "100.101.102.130",
      first_seen_at: now,
      last_seen_at: now,
    };
    this.devices.push(device);
    this.log("device.requested", {
      device_id: device.id,
      node_id: device.node_id,
      name,
      hostname,
      os,
    });
    return device;
  }

  private find(list: Raw[], id: number, what: string): Raw {
    const item = list.find((x) => x.id === id);
    if (!item) throw new HttpError(404, `no ${what} ${id}`);
    return item;
  }

  id(): number {
    return ++this.nextId;
  }

  private projectJson(project: Raw): Raw {
    return { ...project, workspaces: this.workspaces.filter((w) => w.project_id === project.id) };
  }

  private alive(agent: FakeAgent): boolean {
    return agent.state !== "exited" && agent.state !== "interrupted";
  }

  private addProject(b: Raw): DaemonResponse {
    const path = String(b.path ?? "")
      .trim()
      .replace(/\/+$/, "");
    if (!path.startsWith("/")) throw new HttpError(400, "the path must be absolute");
    const existing = this.projects.find(
      (p) => path === p.root || path.startsWith(`${p.root as string}/`),
    );
    if (existing) return { status: 200, body: copy(this.projectJson(existing)) };
    if (/not-a-repo|tmp$/.test(path))
      throw new HttpError(400, `${path} is not inside a git repository`);
    const project = {
      id: this.id(),
      name: path.split("/").pop() || "repo",
      root: path,
      base_branch: "main",
      has_origin: !path.includes("local"),
      created_at: new Date().toISOString(),
    };
    this.projects.push(project);
    this.log("project.added", { project_id: project.id, name: project.name, root: project.root });
    return { status: 201, body: copy(this.projectJson(project)) };
  }

  private removeProject(id: number): DaemonResponse | undefined {
    const project = this.find(this.projects, id, "project");
    if (this.workspaces.some((w) => w.project_id === id)) {
      throw new HttpError(409, `${project.name as string} still has workspaces; remove them first`);
    }
    this.projects.splice(this.projects.indexOf(project), 1);
    this.log("project.removed", { project_id: id, name: project.name });
    return undefined;
  }

  private createWorkspace(projectId: number, b: Raw): DaemonResponse {
    const project = this.find(this.projects, projectId, "project");
    const base = b.base === "head" ? "head" : "origin";
    if (base === "origin" && !project.has_origin) {
      throw new HttpError(
        400,
        `${project.name as string} has no origin remote; start from HEAD instead`,
      );
    }
    // A city nobody uses yet, else the first with a `-vN` suffix, as the daemon does.
    const taken = new Set(this.workspaces.map((w) => w.name));
    const name =
      cities.find((c) => !taken.has(c)) ??
      `${cities[0]}-v${[...taken].filter((t) => String(t).startsWith(`${cities[0]}-v`)).length + 1}`;
    const slug = String(project.name)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    const ws = {
      id: this.id(),
      project_id: projectId,
      name,
      title: String(b.title ?? "").trim() || null,
      city: name,
      branch: `${slug}/${name}`,
      path: `/Users/me/.cos/worktrees/${project.name as string}-${projectId}/${name}`,
      base,
      base_ref: base === "origin" ? `origin/${project.base_branch as string}` : "HEAD",
      base_commit: "4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f70",
      created_at: new Date().toISOString(),
    };
    this.workspaces.push(ws);
    this.log("workspace.created", {
      workspace_id: ws.id,
      project_id: projectId,
      name,
      branch: ws.branch,
      path: ws.path,
      base,
    });
    return { status: 201, body: copy(ws) };
  }

  private removeWorkspace(ws: Raw): DaemonResponse | undefined {
    if (this.agents.some((a) => a.workspace_id === ws.id && this.alive(a))) {
      throw new HttpError(409, "an agent is still running in this workspace; stop it first");
    }
    if (ws.name === "lisbon") {
      throw new HttpError(409, "the worktree has uncommitted changes: src/auth/session.ts");
    }
    this.workspaces.splice(this.workspaces.indexOf(ws), 1);
    for (const agent of this.agents.filter((a) => a.workspace_id === ws.id)) {
      this.agents.splice(this.agents.indexOf(agent), 1);
    }
    this.log("workspace.removed", {
      workspace_id: ws.id,
      project_id: ws.project_id,
      name: ws.name,
      branch: ws.branch,
    });
    return undefined;
  }

  // MARK: Agents

  setAgentState(agent: FakeAgent, state: string): void {
    if (agent.state === state) return;
    const from = agent.state;
    agent.state = state;
    agent.last_activity_ms = Date.now();
    this.log("agent.state_changed", { agent_id: agent.id, state, from });
    this.redraw(agent);
  }

  private createAgent(workspaceId: number, b: Raw): DaemonResponse {
    this.find(this.workspaces, workspaceId, "workspace");
    const id = this.id();
    const prompt = String(b.prompt ?? "").trim();
    const title = String(b.title ?? "").trim() || prompt.slice(0, 60) || `Agent ${id}`;
    const agent: FakeAgent = {
      id,
      workspace_id: workspaceId,
      run_id: null,
      task_id: null,
      role: "user",
      title,
      session_id: crypto.randomUUID?.() ?? `session-${id}`,
      pid: 30_000 + id,
      state: "starting",
      model: (b.model as string) ?? null,
      permission_mode: (b.permission_mode as string) ?? "default",
      user_settings: b.user_settings !== false,
      rows: Number(b.rows ?? 34),
      cols: Number(b.cols ?? 110),
      started_ms: Date.now(),
      last_activity_ms: Date.now(),
      exited_ms: null,
      exit_code: null,
      last_message: null,
      pending_approval_id: null,
    };
    this.env.stamp(agent);
    this.agents.push(agent);
    this.fixtures.screens[id] = {
      prompt,
      transcript: "blank",
      footer: prompt ? { kind: "working", verb: "Thinking", seconds: 0 } : { kind: "idle" },
    };
    this.log("agent.started", {
      agent_id: id,
      workspace_id: workspaceId,
      role: "user",
      session_id: agent.session_id,
    });
    this.later(900, () => this.setAgentState(agent, prompt ? "working" : "idle"));
    if (prompt) {
      this.later(7000, () => {
        if (agent.state !== "working") return;
        agent.last_message = "Done. I looked through the code and made the change; the tests pass.";
        this.fixtures.screens[id] = { prompt, transcript: "notes", footer: { kind: "idle" } };
        this.setAgentState(agent, "idle");
      });
    }
    return { status: 201, body: copy(agent) };
  }

  private agentAction(agent: FakeAgent, action: string, b: Raw): DaemonResponse | undefined {
    switch (action) {
      case "input":
      case "keys": {
        if (!this.alive(agent)) throw new HttpError(409, "the agent has exited");
        const text = action === "input" ? `${String(b.text ?? "")}\r` : atob(String(b.data ?? ""));
        this.typed(agent, text);
        return undefined;
      }
      case "resize": {
        const rows = Number(b.rows);
        const cols = Number(b.cols);
        if (!(rows > 0 && cols > 0)) throw new HttpError(400, "rows and cols must be positive");
        agent.rows = rows;
        agent.cols = cols;
        for (const stream of this.terminals.values()) {
          if (stream.agentId !== agent.id) continue;
          stream.onMessage({ type: "resize", rows, cols });
        }
        // Claude redraws itself on SIGWINCH.
        this.redraw(agent);
        return undefined;
      }
      case "trust":
        if (agent.state !== "needs_trust")
          throw new HttpError(409, "the agent isn't showing the trust dialog");
        this.fixtures.screens[agent.id] = {
          prompt: "",
          transcript: "blank",
          footer: { kind: "idle" },
        };
        this.setAgentState(agent, "starting");
        this.later(700, () => this.setAgentState(agent, "idle"));
        return undefined;
      case "stop":
        if (!this.alive(agent)) throw new HttpError(409, "the agent has already exited");
        this.later(600, () => this.exitAgent(agent, 0));
        if (!this.simulate) this.exitAgent(agent, 0);
        return { status: 202, body: null };
      case "resume": {
        if (this.alive(agent)) throw new HttpError(409, "the agent is still running");
        agent.pid = 30_000 + this.id();
        agent.started_ms = Date.now();
        agent.exited_ms = null;
        agent.exit_code = null;
        agent.state = "starting";
        // A resume launches it again, with the environment as it is now.
        this.env.stamp(agent, true);
        this.fixtures.screens[agent.id] = {
          prompt: "",
          transcript: "blank",
          footer: { kind: "idle" },
        };
        this.log("agent.started", {
          agent_id: agent.id,
          workspace_id: agent.workspace_id,
          role: agent.role,
          session_id: agent.session_id,
        });
        this.later(800, () => this.setAgentState(agent, "idle"));
        return { status: 201, body: copy(agent) };
      }
    }
    throw new HttpError(404, `no route for POST /v1/agents/${agent.id}/${action}`);
  }

  private exitAgent(agent: FakeAgent, code: number | null): void {
    if (!this.alive(agent)) return;
    const from = agent.state;
    agent.state = "exited";
    agent.pid = null;
    agent.exit_code = code;
    agent.exited_ms = Date.now();
    agent.pending_approval_id = null;
    for (const approval of this.approvals.filter(
      (a) => a.agent_id === agent.id && a.state === "pending",
    )) {
      Object.assign(approval, { state: "settled", decided_ms: Date.now() });
      this.log("approval.decided", {
        approval_id: approval.id,
        state: "settled",
        decided_by: null,
      });
    }
    this.log("agent.state_changed", { agent_id: agent.id, state: "exited", from });
    this.log("agent.exited", { agent_id: agent.id, exit_code: code });
    for (const stream of this.terminals.values()) {
      if (stream.agentId !== agent.id) continue;
      if (stream.timer) clearInterval(stream.timer);
      stream.timer = null;
      stream.onMessage({ type: "output", data: b64(`\r\n\u001b[2mbye\u001b[0m\r\n`) });
      stream.onMessage({ type: "exit", exitCode: code });
    }
  }

  private decide(approval: Raw, b: Raw, by: "user" | "chief"): DaemonResponse {
    if (approval.state !== "pending") throw new HttpError(409, "this request was already answered");
    const decision = b.decision === "allow" ? "allow" : b.decision === "deny" ? "deny" : null;
    if (!decision) throw new HttpError(400, "decision must be allow or deny");
    Object.assign(approval, {
      state: decision === "allow" ? "allowed" : "denied",
      decided_by: by,
      decided_ms: Date.now(),
      message: (b.message as string) ?? (b.reason as string) ?? null,
    });
    this.log("approval.decided", {
      approval_id: approval.id,
      state: approval.state,
      decided_by: by,
    });
    const agent = this.agents.find((a) => a.id === approval.agent_id);
    if (agent) {
      agent.pending_approval_id = null;
      const spec = this.fixtures.screens[agent.id];
      if (spec) {
        this.fixtures.screens[agent.id] = {
          ...spec,
          footer: {
            kind: "working",
            verb: decision === "allow" ? "Running" : "Rethinking",
            seconds: 0,
          },
        };
      }
      this.setAgentState(agent, "working");
      this.later(6000, () => {
        if (agent.state !== "working") return;
        if (spec) this.fixtures.screens[agent.id] = { ...spec, footer: { kind: "idle" } };
        this.setAgentState(agent, "idle");
      });
    }
    return { status: 200, body: copy(approval) };
  }

  // MARK: Runs

  counts(runId: number): Raw {
    const counts: Record<string, number> = {
      proposed: 0,
      ready: 0,
      running: 0,
      checking: 0,
      review: 0,
      merged: 0,
      blocked: 0,
      failed: 0,
      cancelled: 0,
    };
    const tasks = this.tasks.filter((t) => t.run_id === runId);
    for (const task of tasks)
      counts[task.state as string] = (counts[task.state as string] ?? 0) + 1;
    return { ...counts, total: tasks.length };
  }

  board(run: Raw): Raw {
    return {
      run,
      workspace: this.workspaces.find((w) => w.id === run.workspace_id) ?? null,
      tasks: this.tasks
        .filter((t) => t.run_id === run.id)
        .sort((a, b) => (a.position as number) - (b.position as number)),
      counts: this.counts(run.id as number),
    };
  }

  private createRun(b: Raw): DaemonResponse {
    const workspaceId = Number(b.workspace_id);
    const ws = this.find(this.workspaces, workspaceId, "workspace");
    const goal = String(b.goal ?? "").trim();
    if (!goal) throw new HttpError(400, "the goal can't be empty");
    if (
      this.runs.some(
        (r) =>
          r.workspace_id === workspaceId &&
          !["done", "failed", "cancelled"].includes(r.state as string),
      )
    ) {
      throw new HttpError(409, `${(ws.title ?? ws.city) as string} already has a run going`);
    }
    const id = Math.max(...this.runs.map((r) => r.id as number)) + 1;
    const chief = this.id();
    const now = new Date().toISOString();
    const run: Raw = {
      id,
      workspace_id: workspaceId,
      goal,
      state: "planning",
      preset: (b.preset as string) ?? "normal",
      approval_cap: Number(b.approval_cap ?? 20),
      chief_agent_id: chief,
      created_at: now,
      updated_at: now,
    };
    this.runs.push(run);
    this.limits[id] = { preset: run.preset, approval_cap: run.approval_cap, chief_approvals: 0 };
    this.agents.push({
      id: chief,
      workspace_id: workspaceId,
      run_id: id,
      task_id: null,
      role: "chief",
      title: "Chief",
      state: "working",
      model: "opus",
      permission_mode: "default",
      user_settings: false,
      rows: 34,
      cols: 110,
      started_ms: Date.now(),
      pid: 31_000 + chief,
    });
    this.fixtures.screens[chief] = {
      prompt: goal,
      transcript: "blank",
      footer: { kind: "working", verb: "Exploring the repo", seconds: 0 },
    };
    this.log("run.created", {
      run_id: id,
      workspace_id: workspaceId,
      goal,
      preset: run.preset,
      approval_cap: run.approval_cap,
    });
    this.log("agent.started", {
      agent_id: chief,
      workspace_id: workspaceId,
      role: "chief",
      session_id: `chief-${chief}`,
    });
    this.later(3500, () => {
      if (run.state !== "planning") return;
      const titles = ["Read the code the goal touches", "Make the change", "Cover it with tests"];
      titles.forEach((title, position) => {
        this.tasks.push({
          id: this.id(),
          run_id: id,
          position,
          title,
          instructions: `${title} for: ${goal}`,
          files: position === 2 ? ["tests/**"] : ["src/**"],
          test_command: position === 2 ? "cargo test" : null,
          depends_on: position === 0 ? [] : [this.nextId - 1],
          state: "proposed",
          retries: 0,
        });
      });
      this.log("plan.proposed", { run_id: id, tasks: titles.length, replaced: 0 });
      this.moveRun(run, "awaiting_plan_approval");
    });
    return { status: 201, body: copy(run) };
  }

  private moveRun(run: Raw, to: string, extra: Raw = {}): void {
    const from = run.state;
    run.state = to;
    run.updated_at = new Date().toISOString();
    Object.assign(run, extra);
    this.log("run.state_changed", { run_id: run.id, from, to, ...extra });
  }

  private moveTask(task: Raw, to: string, reason?: string): void {
    const from = task.state;
    task.state = to;
    task.updated_at = new Date().toISOString();
    if (reason) task.last_failure = reason;
    this.log("task.state_changed", {
      run_id: task.run_id,
      task_id: task.id,
      title: task.title,
      from,
      to,
      retries: task.retries,
      ...(reason ? { reason } : {}),
    });
  }

  private approvePlan(run: Raw): DaemonResponse {
    if (run.state !== "awaiting_plan_approval")
      throw new HttpError(409, "no plan is waiting for approval");
    const tasks = this.tasks.filter((t) => t.run_id === run.id);
    this.log("plan.approved", { run_id: run.id, tasks: tasks.length });
    this.moveRun(run, "running");
    for (const task of tasks.filter((t) => (t.depends_on as number[]).length === 0)) {
      this.moveTask(task, "ready");
      this.later(1500, () => {
        if (task.state !== "ready") return;
        task.branch = `cos/task-${task.id as number}`;
        this.moveTask(task, "running");
      });
    }
    return { status: 200, body: copy(this.board(run)) };
  }

  private finishRun(run: Raw, to: "cancelled" | "failed", reason: string | null): DaemonResponse {
    if (["done", "failed", "cancelled"].includes(run.state as string)) {
      throw new HttpError(409, "the run is already finished");
    }
    for (const task of this.tasks.filter((t) => t.run_id === run.id)) {
      if (!["merged", "failed", "cancelled"].includes(task.state as string))
        this.moveTask(task, "cancelled");
    }
    this.moveRun(run, to, reason ? { reason } : {});
    for (const agent of this.agents.filter((a) => a.run_id === run.id && this.alive(a))) {
      this.later(500, () => this.exitAgent(agent, 0));
    }
    for (const question of this.questions.filter(
      (q) => q.run_id === run.id && q.state === "open",
    )) {
      question.state = "answered";
      question.answer = "(the run was cancelled)";
    }
    return { status: 200, body: copy(this.board(run)) };
  }

  private updateTask(task: Raw, b: Raw): DaemonResponse {
    const run = this.find(this.runs, task.run_id as number, "run");
    if (run.state !== "running")
      throw new HttpError(409, "tasks only move while their run is running");
    const to = String(b.state ?? "");
    const reason = String(b.reason ?? "").trim();
    const state = task.state as string;
    if ((to === "ready" || to === "failed" || to === "blocked") && !reason) {
      throw new HttpError(400, `moving a task to ${to} needs a reason`);
    }
    if (to === "ready") {
      if (state !== "blocked") throw new HttpError(409, `a ${state} task can't be retried`);
      if ((task.retries as number) >= 1)
        throw new HttpError(409, "this task has used its retry; fail or cancel it instead");
      task.retries = (task.retries as number) + 1;
      this.moveTask(task, "ready", reason);
      this.later(1200, () => task.state === "ready" && this.moveTask(task, "running"));
    } else if (to === "failed" || to === "cancelled") {
      if (["merged", "failed", "cancelled"].includes(state))
        throw new HttpError(409, `the task is already ${state}`);
      this.moveTask(task, to, reason || undefined);
      for (const other of this.tasks.filter(
        (t) => t.run_id === task.run_id && t.state === "proposed",
      )) {
        if ((other.depends_on as number[]).includes(task.id as number)) {
          this.moveTask(other, "blocked", `depends on "${task.title as string}", which was ${to}`);
        }
      }
    } else {
      throw new HttpError(409, `can't move a ${state} task to ${to}`);
    }
    return { status: 200, body: copy(this.board(run)) };
  }

  // MARK: Terminals

  private screenFor(agent: FakeAgent, seconds?: number): Screen {
    const spec = this.fixtures.screens[agent.id] ?? {
      prompt: "",
      transcript: "blank" as const,
      footer: { kind: "idle" } as Footer,
    };
    let footer = spec.footer;
    if (!this.alive(agent)) footer = { kind: "exited" };
    else if (footer.kind === "working" && seconds !== undefined)
      footer = { ...footer, seconds: footer.seconds + seconds };
    const ws = this.workspaces.find((w) => w.id === agent.workspace_id);
    return renderScreen({
      ...spec,
      footer,
      cwd: (agent.cwd as string | undefined) ?? (ws?.path as string | undefined) ?? "/Users/me",
      rows: agent.rows,
      cols: agent.cols,
    });
  }

  attach(agentId: number, onMessage: (message: TerminalMessage) => void, id: number): void {
    const stream: TerminalStream = {
      id,
      agentId,
      onMessage,
      screen: null,
      input: "",
      timer: null,
      seconds: 0,
    };
    this.terminals.set(id, stream);
    this.sendSnapshot(stream);
  }

  private sendSnapshot(stream: TerminalStream): void {
    const agent = this.agents.find((a) => a.id === stream.agentId);
    if (!agent) {
      stream.onMessage({ type: "disconnected", reason: `no agent ${stream.agentId}` });
      return;
    }
    stream.screen = this.screenFor(agent, stream.seconds);
    stream.onMessage({
      type: "snapshot",
      rows: agent.rows,
      cols: agent.cols,
      data: b64(stream.screen.data),
    });
    if (!this.alive(agent)) {
      stream.onMessage({ type: "exit", exitCode: agent.exit_code as number | null });
      return;
    }
    this.animate(stream, agent);
  }

  private animate(stream: TerminalStream, agent: FakeAgent): void {
    if (stream.timer) clearInterval(stream.timer);
    stream.timer = null;
    const spec = this.fixtures.screens[agent.id];
    if (!this.simulate || agent.state !== "working" || spec?.footer.kind !== "working") return;
    const verb = spec.footer.verb;
    const base = spec.footer.seconds;
    stream.timer = setInterval(() => {
      if (!this.running || !stream.screen?.spinnerRow) return;
      stream.seconds += 1;
      stream.onMessage({
        type: "output",
        data: b64(
          spinnerUpdate(stream.screen.spinnerRow, verb, base + stream.seconds, stream.screen.input),
        ),
      });
    }, 1000);
  }

  /** The agent's screen changed: redraw every stream of it. */
  private redraw(agent: FakeAgent): void {
    for (const stream of this.terminals.values()) {
      if (stream.agentId !== agent.id || !this.alive(agent)) continue;
      stream.screen = this.screenFor(agent, stream.seconds);
      stream.onMessage({
        type: "output",
        data: b64(
          stream.screen.data +
            (stream.input && stream.screen.input
              ? inputUpdate(stream.screen.input, stream.input)
              : ""),
        ),
      });
      this.animate(stream, agent);
    }
  }

  /** Keystrokes: echo printable ones into the input box; Return submits. */
  private typed(agent: FakeAgent, text: string): void {
    // Return on the login screen stands in for logging in through the browser.
    if (agent.state === "needs_login" && text.includes("\r")) {
      this.fixtures.screens[agent.id] = {
        prompt: "",
        transcript: "blank",
        footer: { kind: "idle" },
      };
      this.setAgentState(agent, "idle");
      return;
    }
    for (const stream of this.terminals.values()) {
      if (stream.agentId !== agent.id || !stream.screen?.input) continue;
      // Drop bracketed-paste markers and other escape sequences.
      // eslint-disable-next-line no-control-regex
      const clean = text.replace(/\u001b\[[0-9;]*[~A-Za-z]/g, "");
      let submitted = false;
      for (const ch of clean) {
        if (ch === "\r") submitted = true;
        else if (ch === "\u007f") stream.input = [...stream.input].slice(0, -1).join("");
        else if (ch >= " ") stream.input += ch;
      }
      if (submitted && stream.input.trim()) {
        const prompt = stream.input;
        stream.input = "";
        this.fixtures.screens[agent.id] = {
          prompt,
          transcript: "blank",
          footer: { kind: "working", verb: "Thinking", seconds: 0 },
        };
        this.setAgentState(agent, "working");
        this.later(5000, () => {
          if (agent.state !== "working") return;
          agent.last_message = `Done: ${prompt}`;
          this.fixtures.screens[agent.id] = {
            prompt,
            transcript: "notes",
            footer: { kind: "idle" },
          };
          this.setAgentState(agent, "idle");
        });
        return;
      }
      stream.onMessage({
        type: "output",
        data: b64(inputUpdate(stream.screen.input, stream.input)),
      });
    }
  }

  detach(id: number): void {
    const stream = this.terminals.get(id);
    if (stream?.timer) clearInterval(stream.timer);
    this.terminals.delete(id);
  }
}

/** `host[:port]` → `host:port`, as the shell reads it (port 7700 by default). */
export function parseAddress(address: string): string {
  const typed = address.trim();
  const refuse = (why: string): BridgeError => ({
    kind: "bad_request",
    message: `"${typed}" is not a host address (${why}). Use a name or IP, like my-mac, my-mac:7700 or 100.101.102.103`,
  });
  if (!typed) throw refuse("it's empty");
  if (typed.includes("://") || typed.includes("/")) throw refuse("leave out http:// and any path");
  if (/\s/.test(typed)) throw refuse("it has spaces");
  const withPort = /^(\[[^\]]+\]|[^:]+):\d+$/.test(typed);
  const full = withPort
    ? typed
    : typed.includes(":") && !typed.startsWith("[")
      ? `[${typed}]:7700`
      : `${typed}:7700`;
  const port = Number(full.slice(full.lastIndexOf(":") + 1));
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw refuse("bad port");
  const host = full.slice(0, full.lastIndexOf(":"));
  const valid = host.startsWith("[")
    ? /^\[[0-9a-fA-F:.]+\]$/.test(host)
    : /^[A-Za-z0-9._-]+$/.test(host);
  if (!valid) throw refuse("not a name or IP address");
  return full;
}

interface OpenStream {
  hostId: string;
  /** A retry waiting while the host refuses or can't be reached. */
  timer: ReturnType<typeof setTimeout> | null;
}

/** Why a host doesn't let this Mac in right now. */
interface Gate {
  message: string;
  notRunning: boolean;
  code: string | null;
  status: number;
}

/**
 * The bridge for a plain browser: everything answered by `FakeDaemon`s, one
 * per host. This Mac's daemon is `daemon`. Studio (`h1`) keeps this Mac
 * waiting for approval (until `approve("h1")`, or a while after it's first
 * tried when simulating); the travel laptop (`h2`) can't be reached.
 */
export class FakeBridge implements Bridge {
  readonly kind = "fake" as const;
  readonly hosts: FakeHost[];
  readonly daemons = new Map<string, FakeDaemon>();
  current: string;
  private readonly options: FakeOptions;
  private readonly latencyMs: number;
  private readonly retryMs: number;
  private readonly approveAfterMs: number;
  private readonly simulate: boolean;
  private nextStream = 1;
  private nextHost = 3;
  private readonly open = new Map<number, OpenStream>();
  /** What the shell would show; tests read these. */
  readonly calls: { setWaitingCount: number[]; notify: { title: string; body: string }[] } = {
    setWaitingCount: [],
    notify: [],
  };

  constructor(options: FakeOptions = {}) {
    this.options = options;
    this.hosts = makeHosts();
    for (const host of this.hosts) this.addDaemon(host);
    this.current = this.hosts.some((h) => h.id === options.host) ? options.host! : "local";
    this.latencyMs = options.latencyMs ?? 60;
    this.retryMs = options.retryMs ?? 3000;
    this.approveAfterMs = options.approveAfterMs ?? 12_000;
    this.simulate = options.simulate ?? true;
  }

  private addDaemon(host: FakeHost): void {
    const remote = host.address !== null;
    const options = remote ? { ...this.options, running: true } : this.options;
    this.daemons.set(host.id, new FakeDaemon(options, remote));
  }

  /** This Mac's daemon. */
  get daemon(): FakeDaemon {
    return this.daemons.get("local")!;
  }

  /** The daemon of the host in use. */
  get currentDaemon(): FakeDaemon {
    return this.daemons.get(this.current)!;
  }

  private hostOf(id: string): FakeHost | undefined {
    return this.hosts.find((h) => h.id === id);
  }

  /** Lets this Mac in, as `cos devices approve` on that host does. */
  approve(hostId: string): void {
    const host = this.hostOf(hostId);
    if (host) host.access = { kind: "open" };
  }

  /** Refuses this Mac from now on, with one of the daemon's codes. */
  refuse(hostId: string, code: string, message: string, status = 403): void {
    const host = this.hostOf(hostId);
    if (host) host.access = { kind: "refused", status, code, message };
  }

  /** Makes a host unreachable, or reachable again. */
  setReachable(hostId: string, reachable: boolean): void {
    const host = this.hostOf(hostId);
    if (host) host.access = reachable ? { kind: "open" } : { kind: "unreachable" };
  }

  private gate(hostId: string): Gate | null {
    const host = this.hostOf(hostId);
    if (!host) return null;
    const access = host.access;
    if (access.kind === "pending") {
      // When simulating, the first try starts the clock for the approval.
      if (access.approveAt === null && this.simulate) {
        access.approveAt = Date.now() + this.approveAfterMs;
      }
      if (access.approveAt !== null && Date.now() >= access.approveAt) {
        host.access = { kind: "open" };
        return null;
      }
    }
    if (host.access.kind === "unreachable") {
      return {
        message: `could not reach ${host.address}: Connection refused (os error 61)`,
        notRunning: true,
        code: null,
        status: 0,
      };
    }
    const refusal = refusalOf(host);
    return refusal ? { ...refusal, notRunning: false } : null;
  }

  private wait(): Promise<void> {
    return this.latencyMs > 0
      ? new Promise((r) => setTimeout(r, this.latencyMs))
      : Promise.resolve();
  }

  private notRunning() {
    return {
      kind: "not_running" as const,
      message: "The cos daemon is not running (nothing answers on /Users/me/.cos/cos.sock).",
    };
  }

  private publicHost(host: FakeHost): Host {
    return { id: host.id, name: host.name, address: host.address };
  }

  async daemonInfo(): Promise<DaemonInfo> {
    return {
      socket: "/Users/me/.cos/cos.sock",
      home: "/Users/me/.cos",
      host: this.publicHost(this.hostOf(this.current)!),
    };
  }

  async daemonRequest(method: HttpMethod, path: string, body?: unknown): Promise<DaemonResponse> {
    await this.wait();
    if (!path.startsWith("/v1/"))
      throw { kind: "bad_request", message: `${path} isn't a /v1/ path` };
    const gate = this.gate(this.current);
    if (gate?.notRunning) throw { kind: "not_running", message: gate.message };
    if (gate) return { status: gate.status, body: { error: gate.message, code: gate.code } };
    const daemon = this.currentDaemon;
    if (!daemon.running) throw this.notRunning();
    return daemon.handle(method, path, body ?? null);
  }

  /**
   * Runs `connect` once the host lets this Mac in, sending `disconnected`
   * (once per reason) and trying again every `retryMs` until then.
   */
  private whenAdmitted(
    id: number,
    hostId: string,
    dropped: (gate: Gate) => void,
    connect: () => void,
  ): void {
    const stream: OpenStream = { hostId, timer: null };
    this.open.set(id, stream);
    let last: string | null = null;
    const attempt = () => {
      stream.timer = null;
      if (!this.open.has(id)) return;
      const gate = this.gate(hostId);
      if (gate) {
        const key = `${gate.code}:${gate.message}`;
        if (key !== last) {
          last = key;
          dropped(gate);
        }
        stream.timer = setTimeout(attempt, this.retryMs);
        return;
      }
      connect();
    };
    stream.timer = setTimeout(attempt, this.latencyMs);
  }

  async followEvents(
    after: number | null,
    onMessage: (message: EventMessage) => void,
  ): Promise<number> {
    const id = this.nextStream++;
    const daemon = this.currentDaemon;
    const follower = { id, onMessage, lastSeq: after ?? 0 };
    this.whenAdmitted(
      id,
      this.current,
      (gate) =>
        onMessage({
          type: "disconnected",
          reason: gate.message,
          notRunning: gate.notRunning,
          code: gate.code,
        }),
      () => {
        daemon.followers.set(id, follower);
        if (!daemon.running) {
          const reason = this.notRunning().message;
          onMessage({ type: "disconnected", reason, notRunning: true, code: null });
          return;
        }
        onMessage({ type: "connected", status: daemon.status() });
        for (const event of daemon.events.filter((e) => (e.seq as number) > follower.lastSeq)) {
          onMessage({ type: "event", event: copy(event) });
        }
        follower.lastSeq = daemon.lastSeq;
      },
    );
    return id;
  }

  async attachTerminal(
    agentId: number,
    onMessage: (message: TerminalMessage) => void,
  ): Promise<number> {
    const id = this.nextStream++;
    const daemon = this.currentDaemon;
    this.whenAdmitted(
      id,
      this.current,
      (gate) => onMessage({ type: "disconnected", reason: gate.message, code: gate.code }),
      () => {
        if (!daemon.running) {
          onMessage({ type: "disconnected", reason: this.notRunning().message, code: null });
          return;
        }
        daemon.attach(agentId, onMessage, id);
      },
    );
    return id;
  }

  async closeStream(id: number): Promise<void> {
    const stream = this.open.get(id);
    if (stream?.timer) clearTimeout(stream.timer);
    this.open.delete(id);
    for (const daemon of this.daemons.values()) {
      daemon.followers.delete(id);
      daemon.detach(id);
    }
  }

  private closeAll(): void {
    for (const id of [...this.open.keys()]) void this.closeStream(id);
  }

  /** Streams still open, for tests. */
  get openStreams(): number {
    return this.open.size;
  }

  async hostsList(): Promise<HostList> {
    return { hosts: this.hosts.map((h) => this.publicHost(h)), current: this.current };
  }

  async hostAdd(name: string, address: string): Promise<Host> {
    const trimmed = name.trim();
    if (!trimmed) throw { kind: "bad_request", message: "give the Mac a name" };
    const full = parseAddress(address);
    const existing = this.hosts.find((h) => h.address === full);
    if (existing) {
      throw { kind: "bad_request", message: `${full} is already saved as ${existing.name}` };
    }
    // A new Mac asks its owner to approve this one first.
    const host: FakeHost = {
      id: `h${this.nextHost++}`,
      name: trimmed,
      address: full,
      access: { kind: "pending", approveAt: null },
    };
    this.hosts.push(host);
    this.addDaemon(host);
    return this.publicHost(host);
  }

  async hostRemove(id: string): Promise<HostList> {
    if (id === "local") throw { kind: "bad_request", message: "This Mac can't be removed" };
    const index = this.hosts.findIndex((h) => h.id === id);
    if (index < 0) throw { kind: "bad_request", message: `no saved host ${id}` };
    this.hosts.splice(index, 1);
    this.daemons.get(id)?.dispose();
    this.daemons.delete(id);
    if (this.current === id) {
      this.closeAll();
      this.current = "local";
    }
    return this.hostsList();
  }

  async hostSelect(id: string): Promise<HostList> {
    if (!this.hostOf(id)) throw { kind: "bad_request", message: `no saved host ${id}` };
    this.closeAll();
    this.current = id;
    return this.hostsList();
  }

  /** Always This Mac's daemon, as the shell does, whichever host is in use. */
  async tailnetPeers(port?: number): Promise<RawTailnetPeers> {
    await this.wait();
    const daemon = this.daemon;
    if (!daemon.running) {
      throw {
        kind: "not_running",
        message:
          "This Mac's cos daemon isn't running (no answer on /Users/me/.cos/cos.sock), so it can't list your tailnet",
      } satisfies BridgeError;
    }
    const path = port === undefined ? "/v1/tailnet/peers" : `/v1/tailnet/peers?port=${port}`;
    const response = daemon.handle("GET", path, null);
    if (response.status < 200 || response.status >= 300) {
      const body = (response.body ?? {}) as { error?: string; code?: string };
      throw {
        kind: "io",
        message: body.error ?? `the cos daemon answered ${response.status}`,
        ...(body.code ? { code: body.code } : {}),
      } satisfies BridgeError;
    }
    return response.body;
  }

  async setWaitingCount(count: number): Promise<void> {
    this.calls.setWaitingCount.push(count);
    console.info(`[fake bridge] set_waiting_count(${count})`);
  }

  async notify(title: string, body: string): Promise<void> {
    this.calls.notify.push({ title, body });
    console.info(`[fake bridge] notify(${JSON.stringify(title)}, ${JSON.stringify(body)})`);
  }

  async revealPath(path: string): Promise<void> {
    console.info(`[fake bridge] reveal_path(${JSON.stringify(path)})`);
  }

  async openInTerminal(path: string): Promise<void> {
    console.info(`[fake bridge] open_in_terminal(${JSON.stringify(path)})`);
  }
}
