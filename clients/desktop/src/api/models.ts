// The daemon's JSON as typed values: docs/daemon.md and docs/agents-api.md. Decoded leniently (see decode.ts). States, roles
// and kinds are open strings: a value a newer daemon adds still decodes.

import {
  DecodeError,
  type Json,
  asObject,
  bool,
  isObject,
  list,
  num,
  numbers,
  optNum,
  optStr,
  reqNum,
  reqStr,
  str,
  strings,
  time,
} from "./decode";

// MARK: Status and events

export interface Status {
  version: string;
  pid: number;
  startedAt: number;
  home: string;
  events: number;
  lastSeq: number;
}

export function decodeStatus(value: unknown): Status {
  const o = asObject(value, "status");
  return {
    version: str(o.version, "?"),
    pid: num(o.pid, 0),
    startedAt: time(o.started_at),
    home: str(o.home),
    events: num(o.events, 0),
    lastSeq: num(o.last_seq, 0),
  };
}

export interface LoggedEvent {
  seq: number;
  ts: number;
  kind: string;
  data: Json;
}

export function decodeEvent(value: unknown): LoggedEvent {
  const o = asObject(value, "event");
  return {
    seq: reqNum(o, "seq", "event"),
    ts: time(o.ts),
    kind: str(o.kind, "unknown"),
    data: isObject(o.data) ? o.data : {},
  };
}

// MARK: Projects and workspaces

export interface Workspace {
  id: number;
  projectId: number;
  name: string;
  /** Null when it has none: clients show the city instead (see `workspaceName`). */
  title: string | null;
  /** The city it's named after: the last segment of its branch, as in `topflexi/lisbon`. */
  city: string;
  branch: string;
  path: string;
  /** `origin` or `head`. */
  base: string;
  baseRef: string;
  baseCommit: string;
  createdAt: number;
}

export interface Project {
  id: number;
  name: string;
  root: string;
  baseBranch: string;
  hasOrigin: boolean;
  createdAt: number;
  workspaces: Workspace[];
}

/** A project's name as a git-safe slug, the first half of its workspace branches. */
export function projectSlug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** The last segment of a branch: `topflexi/lisbon-v1` → `lisbon-v1`. */
export function cityOf(branch: string): string {
  const parts = branch.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

export function decodeWorkspace(value: unknown): Workspace {
  const o = asObject(value, "workspace");
  const id = reqNum(o, "id", "workspace");
  const name = str(o.name);
  const branch = str(o.branch);
  const title = str(o.title).trim();
  return {
    id,
    projectId: num(o.project_id, 0),
    name,
    title: title || null,
    city: str(o.city) || cityOf(branch) || name || `workspace ${id}`,
    branch,
    path: str(o.path),
    base: str(o.base, "origin"),
    baseRef: str(o.base_ref),
    baseCommit: str(o.base_commit),
    createdAt: time(o.created_at),
  };
}

/** What every client shows for a workspace: its title, else its city. */
export function workspaceName(workspace: Pick<Workspace, "title" | "city">): string {
  return workspace.title ?? workspace.city;
}

export function decodeProject(value: unknown): Project {
  const o = asObject(value, "project");
  return {
    id: reqNum(o, "id", "project"),
    name: reqStr(o, "name", "project"),
    root: str(o.root),
    baseBranch: str(o.base_branch, "main"),
    hasOrigin: bool(o.has_origin, false),
    createdAt: time(o.created_at),
    workspaces: list(o.workspaces, decodeWorkspace),
  };
}

// MARK: Agents and approvals

export type AgentState =
  | "starting"
  | "needs_trust"
  /** Claude shows its login screen: the agent's CLAUDE_CONFIG_DIR isn't logged in. */
  | "needs_login"
  | "working"
  | "waiting_approval"
  | "idle"
  | "exited"
  | "interrupted"
  | (string & {});

export type AgentRole = "user" | "staff" | (string & {});

export interface Agent {
  id: number;
  /** Null for an agent outside any workspace, such as a Chief of Staff in its own worktree. */
  workspaceId: number | null;
  role: AgentRole;
  title: string;
  sessionId: string | null;
  pid: number | null;
  state: AgentState;
  model: string | null;
  permissionMode: string | null;
  userSettings: boolean;
  rows: number;
  cols: number;
  startedMs: number;
  lastActivityMs: number | null;
  exitedMs: number | null;
  exitCode: number | null;
  lastMessage: string | null;
  pendingApprovalId: number | null;
  /** Only when it runs outside the workspace folder. */
  cwd: string | null;
  /** The Chief of Staff agent that started it, if one did. */
  spawnedBy: number | null;
  /** Claude's JSONL transcript, once a hook has reported it. */
  transcriptPath: string | null;
  /**
   * The environment variables cos set for it at launch (names only), from
   * its project's and the global list. It keeps them until it restarts.
   */
  envNames: string[];
}

export function decodeAgent(value: unknown): Agent {
  const o = asObject(value, "agent");
  const id = reqNum(o, "id", "agent");
  return {
    id,
    workspaceId: optNum(o.workspace_id),
    role: str(o.role, "user"),
    title: str(o.title) || `Agent ${id}`,
    sessionId: optStr(o.session_id),
    pid: optNum(o.pid),
    state: str(o.state, "starting"),
    model: optStr(o.model),
    permissionMode: optStr(o.permission_mode),
    userSettings: bool(o.user_settings, true),
    rows: num(o.rows, 40),
    cols: num(o.cols, 120),
    startedMs: num(o.started_ms, 0),
    lastActivityMs: optNum(o.last_activity_ms),
    exitedMs: optNum(o.exited_ms),
    exitCode: optNum(o.exit_code),
    lastMessage: optStr(o.last_message),
    pendingApprovalId: optNum(o.pending_approval_id),
    cwd: optStr(o.cwd),
    spawnedBy: optNum(o.spawned_by),
    transcriptPath: optStr(o.transcript_path),
    envNames: strings(o.env_names),
  };
}

export type ApprovalState =
  "pending" | "allowed" | "denied" | "settled" | "expired" | (string & {});
export type ApprovalTier = "allowed" | "chief" | "ask_user" | "never_delegated" | (string & {});

export interface Approval {
  id: number;
  agentId: number;
  workspaceId: number | null;
  toolName: string;
  toolInput: unknown;
  toolUseId: string | null;
  summary: string;
  state: ApprovalState;
  decidedBy: string | null;
  message: string | null;
  createdMs: number;
  decidedMs: number | null;
  /** Null from a daemon without the approvals policy. */
  tier: ApprovalTier | null;
  reason: string | null;
}

export function decodeApproval(value: unknown): Approval {
  const o = asObject(value, "approval");
  const toolName = str(o.tool_name, "tool");
  return {
    id: reqNum(o, "id", "approval"),
    agentId: reqNum(o, "agent_id", "approval"),
    workspaceId: optNum(o.workspace_id),
    toolName,
    toolInput: o.tool_input ?? null,
    toolUseId: optStr(o.tool_use_id),
    summary: str(o.summary) || toolName,
    state: str(o.state, "pending"),
    decidedBy: optStr(o.decided_by),
    message: optStr(o.message),
    createdMs: num(o.created_ms, 0),
    decidedMs: optNum(o.decided_ms),
    tier: optStr(o.tier),
    reason: optStr(o.reason),
  };
}

export interface AgentScreen {
  rows: number;
  cols: number;
  text: string;
  cursor: [number, number];
}

export function decodeScreen(value: unknown): AgentScreen {
  const o = asObject(value, "screen");
  const cursor = numbers(o.cursor);
  return {
    rows: num(o.rows, 0),
    cols: num(o.cols, 0),
    text: str(o.text),
    cursor: [cursor[0] ?? 0, cursor[1] ?? 0],
  };
}

// MARK: Shells (socket-only)

/** A login shell the daemon runs in a folder, for you to type in. */
export interface Shell {
  id: number;
  /** The project whose environment it got. */
  projectId: number | null;
  cwd: string;
  pid: number | null;
  startedMs: number;
  exited: boolean;
  exitCode: number | null;
}

export function decodeShell(value: unknown): Shell {
  const o = asObject(value, "shell");
  return {
    id: reqNum(o, "id", "shell"),
    projectId: optNum(o.project_id),
    cwd: str(o.cwd),
    pid: optNum(o.pid),
    startedMs: num(o.started_ms, 0),
    exited: bool(o.exited, false),
    exitCode: optNum(o.exit_code),
  };
}

// MARK: Devices and the tailnet (socket-only routes)

export interface Device {
  id: number;
  nodeId: string;
  name: string;
  hostname: string;
  os: string;
  loginName: string;
  state: "pending" | "approved" | "denied" | (string & {});
  lastIp: string | null;
  firstSeenAt: number;
  lastSeenAt: number;
  decidedAt: number | null;
}

export function decodeDevice(value: unknown): Device {
  const o = asObject(value, "device");
  return {
    id: reqNum(o, "id", "device"),
    nodeId: str(o.node_id),
    name: str(o.name),
    hostname: str(o.hostname),
    os: str(o.os),
    loginName: str(o.login_name),
    state: str(o.state, "pending"),
    lastIp: optStr(o.last_ip),
    firstSeenAt: time(o.first_seen_at),
    lastSeenAt: time(o.last_seen_at),
    decidedAt: o.decided_at ? time(o.decided_at) : null,
  };
}

export interface TailnetStatus {
  enabled: boolean;
  port: number;
  state: "off" | "starting" | "listening" | "unavailable" | (string & {});
  addresses: string[];
  reason: string | null;
  host: string | null;
  owner: string | null;
}

export function decodeTailnet(value: unknown): TailnetStatus {
  const o = asObject(value, "tailnet");
  return {
    enabled: bool(o.enabled, false),
    port: num(o.port, 7700),
    state: str(o.state, "off"),
    addresses: strings(o.addresses),
    reason: optStr(o.reason),
    host: optStr(o.host),
    owner: optStr(o.owner),
  };
}

/** One device on this Mac's tailnet (`GET /v1/tailnet/peers`). */
export interface TailnetPeer {
  /** Tailscale's stable node ID. */
  id: string;
  name: string;
  hostname: string;
  dnsName: string;
  /** `macOS`, `linux`, `iOS`, `windows`, `android`, … */
  os: string;
  ips: string[];
  online: boolean;
  /** When it was last online (ms epoch), for an offline device Tailscale knows. */
  lastSeenMs: number | null;
  /** Logged in as the same Tailscale user and not tagged: it could let this Mac in. */
  mine: boolean;
  tagged: boolean;
  /** Whether cos answers at `address`, from the daemon's TCP probe. */
  cos: "running" | "not_running" | "unknown" | (string & {});
  /** `ip:port` where cos would listen. */
  address: string;
}

export interface TailnetPeers {
  /** Ordered by the daemon: cos running first, then online, then by name. */
  peers: TailnetPeer[];
  selfName: string | null;
  /** Why Tailscale can't be asked (peers is then empty); null when it can. */
  reason: string | null;
}

export function decodeTailnetPeer(value: unknown): TailnetPeer {
  const o = asObject(value, "tailnet peer");
  const ips = strings(o.ips);
  const ipv4 = ips.find((ip) => !ip.includes(":"));
  const address = str(o.address) || (ipv4 ? `${ipv4}:7700` : "");
  const name = str(o.name) || str(o.hostname) || address;
  if (!name) throw new DecodeError("tailnet peer has no name");
  return {
    id: str(o.id) || name,
    name,
    hostname: str(o.hostname),
    dnsName: str(o.dns_name),
    os: str(o.os),
    ips,
    online: bool(o.online, false),
    lastSeenMs: optNum(o.last_seen_ms),
    mine: bool(o.mine, false),
    tagged: bool(o.tagged, false),
    cos: str(o.cos, "unknown"),
    address,
  };
}

export function decodeTailnetPeers(value: unknown): TailnetPeers {
  const o = asObject(value, "tailnet peers");
  return {
    peers: list(o.peers, decodeTailnetPeer),
    selfName: optStr(o.self_name),
    reason: optStr(o.reason),
  };
}
