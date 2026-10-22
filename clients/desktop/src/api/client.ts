import type { Bridge, HttpMethod } from "../bridge/types";
import { DecodeError, isObject, page } from "./decode";
import {
  type EnvAnswer,
  type EnvVars,
  type ProjectEnv,
  decodeEnvAnswer,
  decodeGlobalEnv,
  decodeProjectEnv,
} from "./env";
import { CosError } from "./errors";
import { type JudgeOverview, type JudgeProject, decodeJudge, decodeJudgeProject } from "./judge";
import {
  type Agent,
  type AgentScreen,
  type Approval,
  type Board,
  type Device,
  type LoggedEvent,
  type Project,
  type Question,
  type Run,
  type RunLimits,
  type Status,
  type TailnetStatus,
  type Workspace,
  decodeAgent,
  decodeApproval,
  decodeBoard,
  decodeDevice,
  decodeEvent,
  decodeLimits,
  decodeProject,
  decodeQuestion,
  decodeRun,
  decodeScreen,
  decodeStatus,
  decodeTailnet,
  decodeWorkspace,
} from "./models";
import {
  type InboxItem,
  type Staff,
  type Transcript,
  decodeInbox,
  decodeStaff,
  decodeTranscript,
} from "./staff";

export interface NewAgent {
  prompt?: string;
  title?: string;
  /** `sonnet`, `opus`, `haiku`; omitted for Claude's default. */
  model?: string;
  /** `default`, `acceptEdits`, `plan` or `dontAsk`. */
  permissionMode?: string;
  userSettings?: boolean;
  rows?: number;
  cols?: number;
}

export interface NewRun {
  workspaceId: number;
  goal: string;
  preset?: string;
  approvalCap?: number;
}

export interface ProposedTask {
  title: string;
  instructions: string;
  files: string[];
  test_command?: string;
  depends_on?: string[];
}

export type Decision = "allow" | "deny";

/** Blank strings are left out, so the daemon picks its defaults. */
function trimmed(value: string | undefined | null): string | undefined {
  const text = value?.trim();
  return text ? text : undefined;
}

/** Base64 of raw bytes, for `POST /v1/agents/{id}/keys`. */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

export function fromBase64(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function query(params: Record<string, string | number | null | undefined>): string {
  const parts = Object.entries(params)
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([key, value]) => `${key}=${encodeURIComponent(String(value))}`);
  return parts.length ? `?${parts.join("&")}` : "";
}

function envPath(projectId: number | null, name: string): string {
  const scope = projectId === null ? "" : `/projects/${projectId}`;
  return `/v1${scope}/env/${encodeURIComponent(name)}`;
}

/**
 * Typed calls for every route in docs/daemon.md, docs/agents-api.md and
 * docs/orchestration.md, over the bridge's `daemon_request`. Non-2xx answers
 * throw `CosError`s carrying the daemon's message; a daemon without a newer
 * route answers 404 (`not_found`).
 */
export class DaemonClient {
  constructor(private readonly bridge: Bridge) {}

  async request(method: HttpMethod, path: string, body?: unknown): Promise<unknown> {
    let response;
    try {
      response = await this.bridge.daemonRequest(method, path, body);
    } catch (error) {
      throw CosError.from(error);
    }
    if (response.status < 200 || response.status >= 300) {
      throw CosError.fromResponse(response.status, response.body, path);
    }
    return response.body;
  }

  private async get<T>(path: string, decode: (body: unknown) => T): Promise<T> {
    return this.decoded(path, decode, await this.request("GET", path));
  }

  private async send<T>(
    method: HttpMethod,
    path: string,
    body: unknown,
    decode: (body: unknown) => T,
  ): Promise<T> {
    return this.decoded(path, decode, await this.request(method, path, body));
  }

  private decoded<T>(path: string, decode: (body: unknown) => T, body: unknown): T {
    try {
      return decode(body);
    } catch (error) {
      if (error instanceof DecodeError) {
        throw new CosError(
          "bad_response",
          `Unexpected answer from cos for ${path}: ${error.message}`,
        );
      }
      throw error;
    }
  }

  // MARK: Status and events

  status(): Promise<Status> {
    return this.get("/v1/status", decodeStatus);
  }

  events(after?: number, limit?: number): Promise<LoggedEvent[]> {
    return this.get(`/v1/events${query({ after, limit })}`, (b) => page(b, "events", decodeEvent));
  }

  // MARK: Projects and workspaces

  projects(): Promise<Project[]> {
    return this.get("/v1/projects", (b) => page(b, "projects", decodeProject));
  }

  /** Registers the repo at `path`; any folder inside it works. */
  addProject(path: string): Promise<Project> {
    return this.send("POST", "/v1/projects", { path }, decodeProject);
  }

  async removeProject(id: number): Promise<void> {
    await this.request("DELETE", `/v1/projects/${id}`);
  }

  createWorkspace(projectId: number, base: string, title?: string): Promise<Workspace> {
    const body: Record<string, string> = { base };
    const name = trimmed(title);
    if (name) body.title = name;
    return this.send("POST", `/v1/projects/${projectId}/workspaces`, body, decodeWorkspace);
  }

  renameWorkspace(id: number, title: string): Promise<Workspace> {
    return this.send("PATCH", `/v1/workspaces/${id}`, { title }, decodeWorkspace);
  }

  async removeWorkspace(id: number): Promise<void> {
    await this.request("DELETE", `/v1/workspaces/${id}`);
  }

  // MARK: Agents

  agents(filter: { workspaceId?: number; state?: string } = {}): Promise<Agent[]> {
    const path = `/v1/agents${query({ workspace_id: filter.workspaceId, state: filter.state })}`;
    return this.get(path, (b) => page(b, "agents", decodeAgent));
  }

  agent(id: number): Promise<Agent> {
    return this.get(`/v1/agents/${id}`, decodeAgent);
  }

  createAgent(workspaceId: number, request: NewAgent = {}): Promise<Agent> {
    const body: Record<string, unknown> = {};
    const prompt = trimmed(request.prompt);
    const title = trimmed(request.title);
    const model = trimmed(request.model);
    const mode = trimmed(request.permissionMode);
    if (prompt) body.prompt = prompt;
    if (title) body.title = title;
    if (model) body.model = model;
    if (mode) body.permission_mode = mode;
    if (request.userSettings !== undefined) body.user_settings = request.userSettings;
    if (request.rows) body.rows = request.rows;
    if (request.cols) body.cols = request.cols;
    return this.send("POST", `/v1/workspaces/${workspaceId}/agents`, body, decodeAgent);
  }

  /** Types the text and presses Return. */
  async sendInput(agentId: number, text: string): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/input`, { text });
  }

  /** Raw keystrokes, exactly as typed. */
  async sendKeys(agentId: number, bytes: Uint8Array): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/keys`, { data: toBase64(bytes) });
  }

  async resizeAgent(agentId: number, rows: number, cols: number): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/resize`, { rows, cols });
  }

  async trustAgent(agentId: number): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/trust`);
  }

  async stopAgent(agentId: number): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/stop`);
  }

  resumeAgent(agentId: number): Promise<Agent> {
    return this.send("POST", `/v1/agents/${agentId}/resume`, undefined, decodeAgent);
  }

  async removeAgent(agentId: number): Promise<void> {
    await this.request("DELETE", `/v1/agents/${agentId}`);
  }

  screen(agentId: number): Promise<AgentScreen> {
    return this.get(`/v1/agents/${agentId}/screen`, decodeScreen);
  }

  // MARK: Approvals

  /** Pending ones by default; `null` for every approval. */
  approvals(state: string | null = "pending"): Promise<Approval[]> {
    return this.get(`/v1/approvals${query({ state })}`, (b) =>
      page(b, "approvals", decodeApproval),
    );
  }

  approval(id: number): Promise<Approval> {
    return this.get(`/v1/approvals/${id}`, decodeApproval);
  }

  decide(id: number, decision: Decision, message?: string): Promise<Approval> {
    const body: Record<string, string> = { decision };
    const text = trimmed(message);
    if (text) body.message = text;
    return this.send("POST", `/v1/approvals/${id}`, body, decodeApproval);
  }

  /** The chief's `approval_decide`. Apps don't normally call it. */
  chiefDecide(id: number, decision: Decision, reason: string): Promise<Approval> {
    return this.send("POST", `/v1/approvals/${id}/chief`, { decision, reason }, decodeApproval);
  }

  // MARK: Runs

  runs(): Promise<Run[]> {
    return this.get("/v1/runs", (b) => page(b, "runs", decodeRun));
  }

  createRun(request: NewRun): Promise<Run> {
    const body: Record<string, unknown> = {
      workspace_id: request.workspaceId,
      goal: request.goal.trim(),
    };
    if (request.preset) body.preset = request.preset;
    if (request.approvalCap !== undefined) body.approval_cap = request.approvalCap;
    return this.send("POST", "/v1/runs", body, decodeRun);
  }

  board(runId: number): Promise<Board> {
    return this.get(`/v1/runs/${runId}`, decodeBoard);
  }

  /** The chief's `board_propose`, raw. Apps don't normally call it. */
  proposePlan(runId: number, tasks: ProposedTask[]): Promise<Board> {
    return this.send("POST", `/v1/runs/${runId}/plan`, { tasks }, decodeBoard);
  }

  approvePlan(runId: number): Promise<Board> {
    return this.send("POST", `/v1/runs/${runId}/approve-plan`, undefined, decodeBoard);
  }

  cancelRun(runId: number, reason?: string): Promise<Board> {
    const text = trimmed(reason);
    return this.send(
      "POST",
      `/v1/runs/${runId}/cancel`,
      text ? { reason: text } : undefined,
      decodeBoard,
    );
  }

  /** `ready` retries a blocked task; `failed` and `cancelled` end it. */
  updateTask(taskId: number, state: string, reason?: string): Promise<Board> {
    const body: Record<string, string> = { state };
    const text = trimmed(reason);
    if (text) body.reason = text;
    return this.send("PATCH", `/v1/tasks/${taskId}`, body, decodeBoard);
  }

  /** Open ones by default; `null` for all. */
  questions(state: string | null = "open"): Promise<Question[]> {
    return this.get(`/v1/questions${query({ state })}`, (b) =>
      page(b, "questions", decodeQuestion),
    );
  }

  answer(questionId: number, answer: string): Promise<Question> {
    return this.send("POST", `/v1/questions/${questionId}/answer`, { answer }, decodeQuestion);
  }

  async messageChief(runId: number, text: string): Promise<void> {
    await this.request("POST", `/v1/runs/${runId}/message`, { text });
  }

  /** The report's markdown; `not_found` before it's written. */
  report(runId: number): Promise<string> {
    return this.get(`/v1/runs/${runId}/report`, (body) => {
      if (isObject(body) && typeof body.markdown === "string") return body.markdown;
      if (typeof body === "string") return body;
      throw new DecodeError("report has no markdown");
    });
  }

  limits(runId: number): Promise<RunLimits> {
    return this.get(`/v1/runs/${runId}/limits`, decodeLimits);
  }

  setLimits(runId: number, limits: { preset?: string; approvalCap?: number }): Promise<RunLimits> {
    const body: Record<string, unknown> = {};
    if (limits.preset) body.preset = limits.preset;
    if (limits.approvalCap !== undefined) body.approval_cap = limits.approvalCap;
    return this.send("PUT", `/v1/runs/${runId}/limits`, body, decodeLimits);
  }

  // MARK: The Chief of Staff (docs/chief-of-staff.md)

  /** It exists before it first starts, with a null `agent_id`. */
  staff(projectId: number): Promise<Staff> {
    return this.get(`/v1/projects/${projectId}/staff`, decodeStaff);
  }

  /** `null` goes back to the default model. */
  setStaff(projectId: number, settings: { model: string | null }): Promise<Staff> {
    return this.send("PUT", `/v1/projects/${projectId}/staff`, settings, decodeStaff);
  }

  /** Starts or resumes it; nothing happens if it's running. */
  startStaff(projectId: number): Promise<Staff> {
    return this.send("POST", `/v1/projects/${projectId}/staff/start`, undefined, decodeStaff);
  }

  /** Starts it if needed and types the text in. While it works, the text waits in its inbox. */
  async messageStaff(projectId: number, text: string): Promise<void> {
    await this.request("POST", `/v1/projects/${projectId}/staff/message`, { text });
  }

  /** What the agents it started reported, newest first. */
  staffInbox(projectId: number): Promise<InboxItem[]> {
    return this.get(`/v1/projects/${projectId}/staff/inbox`, decodeInbox);
  }

  /** An agent's conversation, read from Claude's transcript: messages after `after`. */
  transcript(agentId: number, after?: number): Promise<Transcript> {
    return this.get(
      `/v1/agents/${agentId}/transcript${query({ after: after || undefined })}`,
      decodeTranscript,
    );
  }

  // MARK: Environment variables for agents

  /** The list every project's agents get. */
  env(): Promise<EnvVars> {
    return this.get("/v1/env", decodeGlobalEnv);
  }

  /** A project's own list, the global one, and the merge its agents get. */
  projectEnv(projectId: number): Promise<ProjectEnv> {
    return this.get(`/v1/projects/${projectId}/env`, decodeProjectEnv);
  }

  /**
   * Sets a variable for all projects (`projectId` null) or one. It applies
   * to agents launched or resumed after. Answers the scope's lists.
   */
  async setEnv(projectId: number | null, name: string, value: string): Promise<EnvAnswer> {
    const body = await this.request("PUT", envPath(projectId, name), { value });
    return decodeEnvAnswer(projectId, body);
  }

  /** Removing a name that isn't set is fine. Answers the scope's lists. */
  async removeEnv(projectId: number | null, name: string): Promise<EnvAnswer> {
    const body = await this.request("DELETE", envPath(projectId, name));
    return decodeEnvAnswer(projectId, body);
  }

  // MARK: jev, the judgment model in shadow mode

  /** The key, the projects it judges, and what it would have done. */
  judge(): Promise<JudgeOverview> {
    return this.get("/v1/judge", decodeJudge);
  }

  /** Switches jev on or off for a project. */
  setProjectJudge(projectId: number, enabled: boolean): Promise<JudgeProject> {
    return this.send("PUT", `/v1/projects/${projectId}/judge`, { enabled }, decodeJudgeProject);
  }

  /** Looks for the API key again, probing the login shell. */
  reloadJudge(): Promise<JudgeOverview> {
    return this.send("POST", "/v1/judge/reload", undefined, decodeJudge);
  }

  // MARK: Devices and the tailnet

  devices(): Promise<Device[]> {
    return this.get("/v1/devices", (b) => page(b, "devices", decodeDevice));
  }

  approveDevice(id: number): Promise<Device> {
    return this.send("POST", `/v1/devices/${id}/approve`, undefined, decodeDevice);
  }

  denyDevice(id: number): Promise<Device> {
    return this.send("POST", `/v1/devices/${id}/deny`, undefined, decodeDevice);
  }

  async removeDevice(id: number): Promise<void> {
    await this.request("DELETE", `/v1/devices/${id}`);
  }

  tailnet(): Promise<TailnetStatus> {
    return this.get("/v1/tailnet", decodeTailnet);
  }

  setTailnet(enabled: boolean, port?: number): Promise<TailnetStatus> {
    const body: Record<string, unknown> = { enabled };
    if (port !== undefined) body.port = port;
    return this.send("PUT", "/v1/tailnet", body, decodeTailnet);
  }
}
