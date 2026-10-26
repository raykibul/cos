import type { Bridge, HttpMethod } from "../bridge/types";
import {
  type AttachTarget,
  type Attachment,
  MAX_ATTACHMENT_BYTES,
  decodeAttachment,
  uploadPath,
} from "./attachments";
import { DecodeError, page } from "./decode";
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
  type Device,
  type LoggedEvent,
  type Project,
  type Shell,
  type Status,
  type TailnetStatus,
  type Workspace,
  decodeAgent,
  decodeApproval,
  decodeDevice,
  decodeEvent,
  decodeProject,
  decodeScreen,
  decodeShell,
  decodeStatus,
  decodeTailnet,
  decodeWorkspace,
} from "./models";
import {
  type InboxItem,
  type Staff,
  type StaffNote,
  type Transcript,
  decodeInbox,
  decodeStaff,
  decodeStaffMemory,
  decodeStaffNote,
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
  /** Files uploaded to the workspace, by name. */
  attachments?: string[];
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
 * Typed calls for every route in docs/daemon.md and docs/agents-api.md, over
 * the bridge's `daemon_request`. Non-2xx answers
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
    if (request.attachments?.length) body.attachments = request.attachments;
    return this.send("POST", `/v1/workspaces/${workspaceId}/agents`, body, decodeAgent);
  }

  /** Types the text and presses Return; with files, as one paste naming them. */
  async sendInput(agentId: number, text: string, attachments: string[] = []): Promise<void> {
    await this.request("POST", `/v1/agents/${agentId}/input`, {
      text,
      ...(attachments.length ? { attachments } : {}),
    });
  }

  // MARK: Attachments

  /**
   * Uploads a file into the folder `target` works in. The answer's `name`
   * goes in the message's `attachments`.
   */
  async uploadAttachment(
    target: AttachTarget,
    file: Blob & { name?: string },
  ): Promise<Attachment> {
    if (file.size > MAX_ATTACHMENT_BYTES) {
      throw new CosError(
        "bad_request",
        `${file.name ?? "The file"} is over 32 MB, the most cos takes.`,
      );
    }
    const name = file.name || "file";
    const path = uploadPath(target, name);
    const data = toBase64(new Uint8Array(await file.arrayBuffer()));
    let response;
    try {
      response = await this.bridge.daemonUpload(
        path,
        file.type || "application/octet-stream",
        data,
      );
    } catch (error) {
      throw CosError.from(error);
    }
    if (response.status < 200 || response.status >= 300) {
      throw CosError.fromResponse(response.status, response.body, path);
    }
    return this.decoded(path, decodeAttachment, response.body);
  }

  /** An agent's attachment, to show: a `data:` URL. */
  async attachmentUrl(agentId: number, name: string): Promise<string> {
    const path = `/v1/agents/${agentId}/attachments/${encodeURIComponent(name)}`;
    try {
      const file = await this.bridge.daemonFetchBytes(path);
      return `data:${file.contentType};base64,${file.data}`;
    } catch (error) {
      throw CosError.from(error);
    }
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

  // MARK: Shells

  /** Starts a login shell in `cwd`, with the project's environment when given. */
  startShell(cwd: string, projectId: number | null, rows?: number, cols?: number): Promise<Shell> {
    const body: Record<string, unknown> = { cwd };
    if (projectId !== null) body.project_id = projectId;
    if (rows) body.rows = rows;
    if (cols) body.cols = cols;
    return this.send("POST", "/v1/shells", body, decodeShell);
  }

  async shellKeys(shellId: number, bytes: Uint8Array): Promise<void> {
    await this.request("POST", `/v1/shells/${shellId}/keys`, { data: toBase64(bytes) });
  }

  async resizeShell(shellId: number, rows: number, cols: number): Promise<void> {
    await this.request("POST", `/v1/shells/${shellId}/resize`, { rows, cols });
  }

  /** Kills the shell, if it still runs, and forgets it. */
  async closeShell(shellId: number): Promise<void> {
    await this.request("DELETE", `/v1/shells/${shellId}`);
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

  // MARK: The Chief of Staff (docs/chief-of-staff.md)

  /** It exists before it first starts, with a null `agent_id`. */
  staff(projectId: number): Promise<Staff> {
    return this.get(`/v1/projects/${projectId}/staff`, decodeStaff);
  }

  /**
   * Changes what's given and leaves the rest: `model` (`null` goes back to
   * the default), `paused` (coordination on hold), `brief` and `identity`
   * (replaced, trimmed; read from its next start). A daemon older than
   * editable briefs or identities ignores them and answers the old ones.
   */
  setStaff(
    projectId: number,
    settings: { model?: string | null; paused?: boolean; brief?: string; identity?: string },
  ): Promise<Staff> {
    return this.send("PUT", `/v1/projects/${projectId}/staff`, settings, decodeStaff);
  }

  /** Starts or resumes it; nothing happens if it's running. */
  startStaff(projectId: number): Promise<Staff> {
    return this.send("POST", `/v1/projects/${projectId}/staff/start`, undefined, decodeStaff);
  }

  /**
   * Drops its Claude session and starts it in a new one, with its brief and
   * memory notes. It stays the same agent; a running chief is stopped first.
   */
  newStaffSession(projectId: number): Promise<Staff> {
    return this.send("POST", `/v1/projects/${projectId}/staff/new-session`, undefined, decodeStaff);
  }

  /** Starts it if needed and types the text in. While it works, the text waits in its inbox. */
  async messageStaff(projectId: number, text: string, attachments: string[] = []): Promise<void> {
    await this.request("POST", `/v1/projects/${projectId}/staff/message`, {
      text,
      ...(attachments.length ? { attachments } : {}),
    });
  }

  /** Its memory notes, by name, with their bodies. 404 from a daemon older than notes. */
  staffMemory(projectId: number): Promise<StaffNote[]> {
    return this.get(`/v1/projects/${projectId}/staff/memory`, decodeStaffMemory);
  }

  /** Creates or replaces a note; it reads the new text from its next start. */
  writeStaffNote(
    projectId: number,
    name: string,
    note: { description: string; body: string },
  ): Promise<StaffNote> {
    return this.send(
      "PUT",
      `/v1/projects/${projectId}/staff/memory/${encodeURIComponent(name)}`,
      note,
      decodeStaffNote,
    );
  }

  /** 404 if there is no such note. */
  async deleteStaffNote(projectId: number, name: string): Promise<void> {
    await this.request(
      "DELETE",
      `/v1/projects/${projectId}/staff/memory/${encodeURIComponent(name)}`,
    );
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
