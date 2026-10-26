// The daemon's state for the UI, kept current from `follow_events`: a port
// of CosKit's DaemonMonitor. Each event kind
// refetches what it changes, through coalesced refreshes, and actions apply
// the daemon's answer at once and then refresh.

import { createStore, type StoreApi } from "zustand/vanilla";
import type { AttachTarget, Attachment, SentAttachment } from "../api/attachments";
import { DaemonClient, type Decision, type NewAgent } from "../api/client";
import { CosError, isConflict, isNotFound } from "../api/errors";
import {
  type Agent,
  type Approval,
  type Device,
  type LoggedEvent,
  type Project,
  type Status,
  type TailnetPeers,
  type TailnetStatus,
  type Workspace,
  decodeEvent,
  decodeStatus,
  decodeTailnet,
  decodeTailnetPeers,
} from "../api/models";
import { DecodeError } from "../api/decode";
import { type EnvAnswer, type EnvVars, type ProjectEnv } from "../api/env";
import { type JudgeOverview, type JudgeProject } from "../api/judge";
import {
  type ContextUsage,
  type InboxItem,
  type Staff,
  type StaffNote,
  type Transcript,
  type TranscriptMessage,
} from "../api/staff";
import type { Bridge, DaemonInfo, EventMessage, Forward, Host, HostList } from "../bridge/types";
import {
  agentOf,
  changesAgents,
  changesApprovals,
  changesDevices,
  changesEnv,
  changesJudge,
  changesProjects,
  changesStaff,
  projectOf,
} from "../domain/events";
import { type PendingMessage, mergeMessages, reconcilePending } from "../domain/transcript";
import { CoalescedRefresh, type Result } from "./coalesce";

/**
 * `refused`: a remote daemon answers but won't let this Mac in (see
 * `refusal`). `not_running`: nothing answers, on the socket or at the host's
 * address.
 */
export type Connection = "connecting" | "running" | "not_running" | "reconnecting" | "refused";

/** Why a remote daemon turned this Mac away, from its 403 or 503. */
export interface Refusal {
  /** `device_pending`, `device_denied`, `wrong_user`, `tagged_device`, `unknown_peer`, `tailscale_unavailable`, … */
  code: string;
  /** The daemon's message, which names the command to run on the host. */
  message: string;
}

export interface DaemonState {
  connection: Connection;
  /** Why the stream dropped, while reconnecting or not running. */
  connectionReason: string | null;
  /** Set while `connection` is `refused`. */
  refusal: Refusal | null;
  status: Status | null;
  info: DaemonInfo | null;
  /** The host in use; null until the shell says (taken as this Mac). */
  host: Host | null;
  /** This Mac and the saved hosts. */
  hosts: Host[];
  /** Devices that tried this Mac's tailnet listener, oldest first. Only for This Mac. */
  devices: Device[];
  devicesLoaded: boolean;
  /** This Mac's tailnet listener; null on a remote host or before it loads. */
  tailnet: TailnetStatus | null;
  /** False when the daemon has no remote access routes (404). */
  remoteAccessSupported: boolean;
  remoteAccessError: string | null;
  /** The most recent events, oldest first. */
  events: LoggedEvent[];
  /** In the order they were added, each with its workspaces. Kept while the daemon is away. */
  projects: Project[];
  projectsLoaded: boolean;
  projectsError: string | null;
  /** Every agent, newest first. */
  agents: Agent[];
  /** Pending approvals, oldest first. */
  approvals: Approval[];
  /** Whether the pending approvals have been fetched once. */
  approvalsLoaded: boolean;
  /** False when the daemon answers 404 for the agents routes. */
  agentsSupported: boolean;
  agentsError: string | null;
  /** Each project's Chief of Staff, by project id. */
  staffs: Record<number, Staff>;
  /** False when the daemon answers 404 for the staff routes. */
  staffSupported: boolean;
  /** What the chief's agents reported, newest first, by project id, while a page shows it. */
  inboxes: Record<number, InboxItem[]>;
  /** Conversations read from transcripts, by agent id, while a page shows them. */
  transcripts: Record<number, TranscriptState>;
  /** Each chief's memory notes, by project id, while a page shows them. */
  memories: Record<number, MemoryState>;
  /** What you sent each project's chief that its transcript doesn't show yet. */
  pending: Record<number, PendingMessage[]>;
  /** What you sent each agent (by agent id) that its transcript doesn't show yet. */
  agentPending: Record<number, PendingMessage[]>;
  /** The environment variables every project's agents get; null until loaded. */
  globalEnv: EnvVars | null;
  /** Each project's own variables, with the global ones and the merge, by project id. */
  projectEnvs: Record<number, ProjectEnv>;
  /** False when the daemon answers 404 for the env routes. */
  envSupported: boolean;
  /** Why a list couldn't be loaded, by `envKey`. */
  envErrors: Record<string, string>;
  /** jev in shadow mode: its key, the projects it judges, its stats; null until loaded. */
  judge: JudgeOverview | null;
  /** False when the daemon answers 404 for `/v1/judge`. */
  judgeSupported: boolean;
  judgeError: string | null;
  /** The ports forwarded from the host in use, by remote port; always empty on This Mac. */
  forwards: Forward[];
}

/** `global`, or `project:<id>`: where an environment list's error is kept. */
export function envKey(projectId: number | null): string {
  return projectId === null ? "global" : `project:${projectId}`;
}

export interface MemoryState {
  /** By name; null until loaded. */
  notes: StaffNote[] | null;
  /** False when the daemon answers 404: it's older than memory notes. */
  supported: boolean;
  error: string | null;
}

export interface TranscriptState {
  messages: TranscriptMessage[];
  /** What to ask for next (`after=next`). */
  next: number;
  /** How full its context is, as of the last read; null until the daemon says. */
  context: ContextUsage | null;
  loaded: boolean;
  error: string | null;
}

export function initialState(): DaemonState {
  return {
    connection: "connecting",
    connectionReason: null,
    refusal: null,
    status: null,
    info: null,
    host: null,
    hosts: [],
    devices: [],
    devicesLoaded: false,
    tailnet: null,
    remoteAccessSupported: true,
    remoteAccessError: null,
    events: [],
    projects: [],
    projectsLoaded: false,
    projectsError: null,
    agents: [],
    approvals: [],
    approvalsLoaded: false,
    agentsSupported: true,
    agentsError: null,
    staffs: {},
    staffSupported: true,
    inboxes: {},
    memories: {},
    transcripts: {},
    pending: {},
    agentPending: {},
    globalEnv: null,
    projectEnvs: {},
    envSupported: true,
    envErrors: {},
    judge: null,
    judgeSupported: true,
    judgeError: null,
    forwards: [],
  };
}

interface Watch<T> {
  count: number;
  refresh: CoalescedRefresh<T>;
}

export interface StoreOptions {
  /** How many events to keep in memory. */
  keep?: number;
  /** How many events before the newest to start with. */
  backlog?: number;
  /** How long after a watched agent's turn ends to read its transcript once more. */
  lateReadMs?: number;
}

/**
 * Claude can write a turn's last reply after the hook that ends it, and a
 * daemon before this fix looked at the file only once, so the transcript is
 * read again this long after the turn ends.
 */
const LATE_READ_MS = 2500;

function message(error: unknown): string {
  return CosError.from(error).message;
}

function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  if (!(key in record)) return record;
  const next = { ...record };
  delete next[key];
  return next;
}

/** Whether the host is this Mac (a missing host, from an older shell, is too). */
export function isThisMac(host: Host | null): boolean {
  return host === null || host.address === null;
}

export class CosStore {
  readonly state: StoreApi<DaemonState>;
  readonly client: DaemonClient;
  private readonly keep: number;
  private readonly backlog: number;
  private streamId: number | null = null;
  /** `follow_forwards`, while another machine is in use. */
  private forwardsStreamId: number | null = null;
  private started = false;
  /**
   * Bumped by `stop` and on each switch of host, so answers and stream
   * messages from before are dropped.
   */
  private generation = 0;
  private pendingEvents: LoggedEvent[] = [];
  private flushScheduled = false;

  // Made again for each host, so an answer from the last one is never applied.
  private projectsRefresh!: CoalescedRefresh<Project[]>;
  private agentsRefresh!: CoalescedRefresh<Agent[]>;
  private approvalsRefresh!: CoalescedRefresh<Approval[]>;
  private devicesRefresh!: CoalescedRefresh<Device[]>;
  private tailnetRefresh!: CoalescedRefresh<TailnetStatus>;
  private globalEnvRefresh!: CoalescedRefresh<EnvVars>;
  private judgeRefresh!: CoalescedRefresh<JudgeOverview>;
  private readonly projectEnvRefreshes = new Map<number, CoalescedRefresh<ProjectEnv>>();
  /** Set on each (re)connect: the next project list refetches every project's environment. */
  private envStale = true;
  private readonly staffRefreshes = new Map<number, CoalescedRefresh<Staff>>();
  /** Set on each (re)connect: the next project list refetches every chief. */
  private staffStale = true;
  private readonly transcriptWatches = new Map<number, Watch<Transcript>>();
  /** Agents whose transcript restarted: the next fetch reads it from the top. */
  private readonly transcriptResets = new Set<number>();
  /** A second read of a transcript, after its agent's turn ended, by agent id. */
  private readonly lateReads = new Map<number, ReturnType<typeof setTimeout>>();
  private readonly lateReadMs: number;
  private readonly inboxWatches = new Map<number, Watch<InboxItem[]>>();
  private readonly memoryWatches = new Map<number, Watch<StaffNote[]>>();
  private nextPendingId = 1;
  /** Attachments fetched to show, by `agentId/name`. */
  private readonly attachmentUrls = new Map<string, Promise<string>>();

  constructor(
    readonly bridge: Bridge,
    options: StoreOptions = {},
  ) {
    this.state = createStore<DaemonState>()(() => initialState());
    this.client = new DaemonClient(bridge);
    this.keep = options.keep ?? 1000;
    this.backlog = options.backlog ?? 200;
    this.lateReadMs = options.lateReadMs ?? LATE_READ_MS;
    this.createRefreshes();
  }

  private disposeRefreshes(): void {
    for (const refresh of [
      this.projectsRefresh,
      this.agentsRefresh,
      this.approvalsRefresh,
      this.devicesRefresh,
      this.tailnetRefresh,
      this.globalEnvRefresh,
      this.judgeRefresh,
    ]) {
      refresh.dispose();
    }
  }

  private createRefreshes(): void {
    const client = this.client;
    this.devicesRefresh = new CoalescedRefresh(
      () => client.devices(),
      (result) => {
        if (result.ok) {
          this.set({ devices: result.value, devicesLoaded: true, remoteAccessError: null });
        } else if (isNotFound(result.error)) {
          this.set({ devices: [], devicesLoaded: true, remoteAccessSupported: false });
        } else {
          this.set({ remoteAccessError: message(result.error) });
        }
      },
    );
    this.tailnetRefresh = new CoalescedRefresh(
      () => client.tailnet(),
      (result) => {
        if (result.ok) {
          this.set({ tailnet: result.value, remoteAccessSupported: true, remoteAccessError: null });
        } else if (isNotFound(result.error)) {
          this.set({ remoteAccessSupported: false });
        } else {
          this.set({ remoteAccessError: message(result.error) });
        }
      },
    );

    this.globalEnvRefresh = new CoalescedRefresh(
      () => client.env(),
      (result) => {
        if (result.ok) {
          this.set((s) => ({
            globalEnv: result.value,
            envSupported: true,
            envErrors: withoutKey(s.envErrors, envKey(null)),
          }));
        } else this.envFailed(null, result.error);
      },
    );

    this.judgeRefresh = new CoalescedRefresh(
      () => client.judge(),
      (result) => {
        if (result.ok) {
          this.set({ judge: result.value, judgeSupported: true, judgeError: null });
        } else if (isNotFound(result.error)) {
          this.set({ judge: null, judgeSupported: false, judgeError: null });
        } else {
          this.set({ judgeError: message(result.error) });
        }
      },
    );

    this.projectsRefresh = new CoalescedRefresh(
      () => client.projects(),
      (result) => {
        if (result.ok) {
          this.set({ projects: result.value, projectsLoaded: true, projectsError: null });
          this.followProjects(result.value);
        } else this.set({ projectsError: message(result.error) });
      },
    );
    this.agentsRefresh = new CoalescedRefresh(
      () => client.agents(),
      (result) => this.applyAgentsResult(result, (agents) => ({ agents })),
    );
    this.approvalsRefresh = new CoalescedRefresh(
      () => client.approvals("pending"),
      (result) =>
        this.applyAgentsResult(result, (approvals) => ({
          approvals: approvals
            .filter((a) => a.state === "pending")
            .sort((a, b) => a.createdMs - b.createdMs),
          approvalsLoaded: true,
        })),
    );
  }

  get(): DaemonState {
    return this.state.getState();
  }

  private set(
    partial: Partial<DaemonState> | ((state: DaemonState) => Partial<DaemonState>),
  ): void {
    this.state.setState(partial);
  }

  /** An agents or approvals answer. A 404 means a daemon without agents. */
  private applyAgentsResult<T>(result: Result<T>, store: (value: T) => Partial<DaemonState>): void {
    if (result.ok) {
      this.set({ ...store(result.value), agentsSupported: true, agentsError: null });
    } else if (isNotFound(result.error)) {
      this.set({ agentsSupported: false, agents: [], approvals: [], agentsError: null });
    } else {
      this.set({ agentsError: message(result.error) });
    }
  }

  // MARK: Following the daemon

  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    const generation = this.generation;
    const current = () => this.started && this.generation === generation;
    // Which host this is decides what's fetched (devices only on This Mac).
    await this.loadHosts();
    if (!current()) return;
    if (!this.isThisMac()) void this.followForwards(current);
    // Start a backlog before the newest event, so the log has some history
    // without replaying all of it.
    const after = await this.client.status().then(
      (status) => Math.max(0, status.lastSeq - this.backlog),
      () => null,
    );
    if (!current()) return;
    try {
      const id = await this.bridge.followEvents(after, (message) => {
        if (this.generation === generation) this.handle(message);
      });
      if (current()) this.streamId = id;
      else void this.bridge.closeStream(id);
    } catch (error) {
      if (!current()) return;
      const failure = CosError.from(error);
      this.set({
        connection: failure.kind === "not_running" ? "not_running" : "reconnecting",
        connectionReason: failure.message,
      });
    }
  }

  stop(): void {
    this.started = false;
    this.generation += 1;
    if (this.streamId !== null) void this.bridge.closeStream(this.streamId);
    this.streamId = null;
    if (this.forwardsStreamId !== null) void this.bridge.closeStream(this.forwardsStreamId);
    this.forwardsStreamId = null;
  }

  /** Follows the same host again from scratch: "Try now" on a refusal. */
  async reconnect(): Promise<void> {
    this.stop();
    await this.start();
  }

  /** Where the daemon is and which host is in use, from the shell. */
  private async loadHosts(): Promise<void> {
    const [info, list] = await Promise.allSettled([
      this.bridge.daemonInfo(),
      this.bridge.hostsList(),
    ]);
    const partial: Partial<DaemonState> = {};
    if (info.status === "fulfilled") partial.info = info.value;
    if (list.status === "fulfilled") partial.hosts = list.value.hosts;
    const host =
      (info.status === "fulfilled" ? info.value.host : undefined) ??
      (list.status === "fulfilled"
        ? list.value.hosts.find((h) => h.id === list.value.current)
        : undefined);
    if (host) partial.host = host;
    this.set(partial);
  }

  /** Whether the host in use is this Mac, where devices and the listener are managed. */
  isThisMac(): boolean {
    return isThisMac(this.get().host);
  }

  /** One message from `follow_events`. */
  handle(message: EventMessage): void {
    switch (message.type) {
      case "connected": {
        let status: Status | null;
        try {
          status = decodeStatus(message.status);
        } catch {
          status = null;
        }
        this.set({ status, connection: "running", connectionReason: null, refusal: null });
        this.staffStale = true;
        this.envStale = true;
        this.refreshProjects();
        this.globalEnvRefresh.request();
        this.judgeRefresh.request();
        this.refreshAgents();
        // What changed while away: the conversations and inboxes on screen.
        for (const watch of this.transcriptWatches.values()) watch.refresh.request();
        for (const watch of this.inboxWatches.values()) watch.refresh.request();
        for (const watch of this.memoryWatches.values()) watch.refresh.request();
        this.refreshRemoteAccess();
        break;
      }
      case "event": {
        let event: LoggedEvent;
        try {
          event = decodeEvent(message.event);
        } catch {
          return;
        }
        this.applyEvent(event);
        break;
      }
      case "log_reset":
        this.pendingEvents = [];
        this.set({ events: [] });
        break;
      case "disconnected":
        this.set({
          status: null,
          connection: message.code
            ? "refused"
            : message.notRunning
              ? "not_running"
              : "reconnecting",
          connectionReason: message.reason,
          refusal: message.code ? { code: message.code, message: message.reason } : null,
        });
        break;
    }
  }

  private applyEvent(event: LoggedEvent): void {
    const kind = event.kind;
    if (changesDevices(kind) && this.isThisMac()) this.devicesRefresh.request();
    if (kind === "tailnet.changed" && this.isThisMac()) {
      try {
        this.set({ tailnet: decodeTailnet(event.data) });
      } catch {
        this.tailnetRefresh.request();
      }
    }
    if (changesProjects(kind)) this.projectsRefresh.request();
    if (changesAgents(kind)) this.agentsRefresh.request();
    if (changesApprovals(kind)) this.approvalsRefresh.request();
    if (changesStaff(kind)) {
      const project = projectOf(event);
      if (project !== null) this.refreshStaff(project);
      else for (const p of this.get().projects) this.refreshStaff(p.id);
      // `staff.changed` also means its brief or notes changed.
      for (const [id, watch] of this.memoryWatches) {
        if (project === null || project === id) watch.refresh.request();
      }
    }
    if (changesEnv(kind)) {
      // A global change moves every project's merge too.
      const project = projectOf(event);
      if (project !== null) this.refreshEnv(project);
      else this.refreshEnv(null);
    }
    // A decision moves the stats; a switch or a key reload, the rest.
    if (changesJudge(kind)) this.judgeRefresh.request();
    if (kind === "staff.inbox") {
      const project = projectOf(event);
      // Its count of updates waiting moved (shown while coordination is paused).
      if (project !== null && this.get().staffs[project]) this.refreshStaff(project);
      for (const [id, watch] of this.inboxWatches) {
        if (project === null || project === id) watch.refresh.request();
      }
    }
    if (kind === "agent.transcript") this.transcriptGrew(event);
    if (kind === "agent.state_changed") this.turnMoved(event);
    this.pendingEvents.push(event);
    this.scheduleFlush();
  }

  /** Events arrive in bursts (a replay, a plan approval); append them together. */
  private scheduleFlush(): void {
    if (this.flushScheduled) return;
    this.flushScheduled = true;
    queueMicrotask(() => {
      this.flushScheduled = false;
      const incoming = this.pendingEvents;
      this.pendingEvents = [];
      if (incoming.length === 0) return;
      this.set((state) => {
        let events = state.events.concat(incoming);
        if (events.length > this.keep) events = events.slice(events.length - this.keep);
        let status = state.status;
        const last = incoming[incoming.length - 1]!;
        // Events only ever append: keep the counts current without asking.
        if (status && last.seq > status.lastSeq) {
          const added = incoming.filter((e) => e.seq > status!.lastSeq).length;
          status = { ...status, events: status.events + added, lastSeq: last.seq };
        }
        return { events, status };
      });
    });
  }

  // MARK: Refreshes

  refreshProjects(): void {
    this.projectsRefresh.request();
  }

  /** Devices and the tailnet listener, which only This Mac's socket serves. */
  refreshRemoteAccess(): void {
    if (!this.isThisMac()) return;
    this.devicesRefresh.request();
    this.tailnetRefresh.request();
  }

  refreshAgents(): void {
    this.agentsRefresh.request();
    this.approvalsRefresh.request();
  }

  // MARK: The Chief of Staff

  /**
   * After the project list loads: fetch the chief of every project after a
   * (re)connect, else only of projects that are new, and forget removed ones.
   */
  private followProjects(projects: Project[]): void {
    const all = this.staffStale;
    this.staffStale = false;
    const allEnv = this.envStale;
    this.envStale = false;
    const ids = new Set(projects.map((p) => p.id));
    const { staffs, projectEnvs } = this.get();
    for (const project of projects) {
      if (all || staffs[project.id] === undefined) this.refreshStaff(project.id);
      if (allEnv || projectEnvs[project.id] === undefined) this.refreshProjectEnv(project.id);
    }
    const gone = (record: Record<number, unknown>) =>
      Object.keys(record).some((id) => !ids.has(Number(id)));
    const keep = <T>(record: Record<number, T>): Record<number, T> =>
      Object.fromEntries(Object.entries(record).filter(([id]) => ids.has(Number(id))));
    if (gone(staffs)) this.set((s) => ({ staffs: keep(s.staffs) }));
    if (gone(projectEnvs)) this.set((s) => ({ projectEnvs: keep(s.projectEnvs) }));
  }

  refreshStaff(projectId: number): void {
    let refresh = this.staffRefreshes.get(projectId);
    if (!refresh) {
      refresh = new CoalescedRefresh(
        () => this.client.staff(projectId),
        (result) => {
          if (result.ok) this.storeStaff(result.value);
          else if (isNotFound(result.error) && !this.get().projects.some((p) => p.id === projectId))
            return;
          else if (isNotFound(result.error)) this.set({ staffSupported: false });
        },
      );
      this.staffRefreshes.set(projectId, refresh);
    }
    refresh.request();
  }

  private storeStaff(staff: Staff): void {
    this.set((s) => ({ staffs: { ...s.staffs, [staff.projectId]: staff }, staffSupported: true }));
  }

  /** `agent.transcript {agent_id, next}`: read what's new, if a page shows that agent. */
  private transcriptGrew(event: LoggedEvent): void {
    const id = agentOf(event);
    if (id === null) return;
    const watch = this.transcriptWatches.get(id);
    if (!watch) return;
    const next = typeof event.data.next === "number" ? event.data.next : null;
    const have = this.get().transcripts[id]?.next ?? 0;
    if (next !== null && next === have) return;
    // A shorter transcript started over (a new session): read it from the top.
    if (next !== null && next < have) this.transcriptResets.add(id);
    watch.refresh.request();
  }

  /**
   * `agent.state_changed`: a watched agent that went idle finished a turn.
   * Its transcript is read now and once more a little later, in case the
   * reply reached the file after the turn ended.
   */
  private turnMoved(event: LoggedEvent): void {
    const id = agentOf(event);
    if (id === null || event.data.state !== "idle") return;
    const watch = this.transcriptWatches.get(id);
    if (!watch) return;
    watch.refresh.request();
    clearTimeout(this.lateReads.get(id));
    this.lateReads.set(
      id,
      setTimeout(() => {
        this.lateReads.delete(id);
        this.transcriptWatches.get(id)?.refresh.request();
      }, this.lateReadMs),
    );
  }

  /**
   * Catches up with anything the event stream missed: the conversations
   * and inboxes on screen, and the agents. The app calls it when its window
   * comes back and every so often while it's shown (`app/resync.ts`).
   */
  resync(): void {
    if (this.get().connection !== "running") return;
    for (const watch of this.transcriptWatches.values()) watch.refresh.request();
    for (const watch of this.inboxWatches.values()) watch.refresh.request();
    this.refreshAgents();
  }

  /** Keeps an agent's conversation current until the returned function is called. */
  watchTranscript(agentId: number): () => void {
    return this.watch(this.transcriptWatches, agentId, () => {
      const refresh = new CoalescedRefresh<Transcript>(
        () => {
          const reset = this.transcriptResets.delete(agentId);
          const after = reset ? 0 : (this.get().transcripts[agentId]?.next ?? 0);
          return this.client.transcript(agentId, after).then((page) => ({ ...page, reset }));
        },
        (result) => this.applyTranscript(agentId, result),
      );
      return refresh;
    });
  }

  private applyTranscript(agentId: number, result: Result<Transcript & { reset?: boolean }>): void {
    this.set((s) => {
      const current = s.transcripts[agentId];
      if (!result.ok) {
        const error = isNotFound(result.error)
          ? "This daemon can't show conversations yet. Update cos."
          : message(result.error);
        return {
          transcripts: {
            ...s.transcripts,
            [agentId]: {
              messages: current?.messages ?? [],
              next: current?.next ?? 0,
              context: current?.context ?? null,
              loaded: true,
              error,
            },
          },
        };
      }
      const page = result.value;
      const messages = page.reset
        ? page.messages
        : mergeMessages(current?.messages ?? [], page.messages);
      const transcripts = {
        ...s.transcripts,
        [agentId]: {
          messages,
          next: Math.max(page.reset ? 0 : (current?.next ?? 0), page.next),
          context: page.context ?? (page.reset ? null : (current?.context ?? null)),
          loaded: true,
          error: null,
        },
      };
      // What you sent shows once; drop it when the transcript has it.
      let pending = s.pending;
      for (const [project, staff] of Object.entries(s.staffs)) {
        const id = Number(project);
        if (staff.agentId !== agentId || !pending[id]?.length) continue;
        const left = reconcilePending(pending[id], messages);
        if (left !== pending[id]) pending = { ...pending, [id]: left };
      }
      let agentPending = s.agentPending;
      if (agentPending[agentId]?.length) {
        const left = reconcilePending(agentPending[agentId], messages);
        if (left !== agentPending[agentId]) agentPending = { ...agentPending, [agentId]: left };
      }
      return { transcripts, pending, agentPending };
    });
  }

  /** Keeps a project's chief inbox current until the returned function is called. */
  watchInbox(projectId: number): () => void {
    return this.watch(
      this.inboxWatches,
      projectId,
      () =>
        new CoalescedRefresh(
          () => this.client.staffInbox(projectId),
          (result) => {
            if (result.ok)
              this.set((s) => ({ inboxes: { ...s.inboxes, [projectId]: result.value } }));
          },
        ),
    );
  }

  /** Keeps a project's chief's memory notes current until the returned function is called. */
  watchMemory(projectId: number): () => void {
    return this.watch(
      this.memoryWatches,
      projectId,
      () =>
        new CoalescedRefresh(
          () => this.client.staffMemory(projectId),
          (result) => {
            if (result.ok) this.storeMemory(projectId, { notes: result.value, error: null });
            else if (isNotFound(result.error)) this.storeMemory(projectId, { supported: false });
            else this.storeMemory(projectId, { error: message(result.error) });
          },
        ),
    );
  }

  private storeMemory(projectId: number, change: Partial<MemoryState>): void {
    this.set((s) => {
      const current = s.memories[projectId] ?? { notes: null, supported: true, error: null };
      const next = { ...current, supported: true, ...change };
      return { memories: { ...s.memories, [projectId]: next } };
    });
  }

  /** A counted watch: the first one creates and fetches; the last one disposes. */
  private watch<T>(
    watches: Map<number, Watch<T>>,
    id: number,
    create: () => CoalescedRefresh<T>,
  ): () => void {
    let held = watches.get(id);
    if (held) held.count += 1;
    else {
      held = { count: 1, refresh: create() };
      watches.set(id, held);
      held.refresh.request();
    }
    const owned = held;
    let done = false;
    return () => {
      if (done) return;
      done = true;
      // Gone, or made again after a host switch: not this one's to release.
      const watch = watches.get(id);
      if (watch !== owned) return;
      watch.count -= 1;
      if (watch.count <= 0) {
        watch.refresh.dispose();
        watches.delete(id);
      }
    };
  }

  isWatchingTranscript(agentId: number): boolean {
    return this.transcriptWatches.has(agentId);
  }

  async startStaff(projectId: number): Promise<Staff> {
    const staff = await this.client.startStaff(projectId);
    this.storeStaff(staff);
    this.agentsRefresh.request();
    return staff;
  }

  /** A new Claude session for the chief: its chat starts over. */
  async newStaffSession(projectId: number): Promise<Staff> {
    const staff = await this.client.newStaffSession(projectId);
    this.storeStaff(staff);
    this.agentsRefresh.request();
    return staff;
  }

  /** `null` for the default model. It applies from the chief's next start. */
  async setStaffModel(projectId: number, model: string | null): Promise<Staff> {
    const staff = await this.client.setStaff(projectId, { model });
    this.storeStaff(staff);
    return staff;
  }

  /**
   * Replaces the chief's brief; it reads the new one from its next start or
   * new session. Throws if the daemon is older than editable briefs, which
   * ignores the field and answers the old one.
   */
  async setStaffBrief(projectId: number, brief: string): Promise<Staff> {
    const staff = await this.client.setStaff(projectId, { brief });
    this.storeStaff(staff);
    if (staff.brief !== brief.trim()) {
      throw new CosError(
        "bad_response",
        "This cos daemon can't change the brief yet. Update cos and restart the daemon.",
      );
    }
    return staff;
  }

  /**
   * Replaces the chief's identity, which only you write; it takes it from its
   * next start or new session. Throws if the daemon is older than identities.
   */
  async setStaffIdentity(projectId: number, identity: string): Promise<Staff> {
    const staff = await this.client.setStaff(projectId, { identity });
    this.storeStaff(staff);
    if (staff.identity !== identity.trim()) {
      throw new CosError(
        "bad_response",
        "This cos daemon can't keep an identity yet. Update cos and restart the daemon.",
      );
    }
    return staff;
  }

  /** Creates or replaces one of the chief's notes. */
  async writeStaffNote(
    projectId: number,
    name: string,
    note: { description: string; body: string },
  ): Promise<StaffNote> {
    const saved = await this.client.writeStaffNote(projectId, name, note);
    const notes = (this.get().memories[projectId]?.notes ?? []).filter(
      (n) => n.name !== saved.name,
    );
    notes.push(saved);
    notes.sort((a, b) => a.name.localeCompare(b.name));
    this.storeMemory(projectId, { notes });
    return saved;
  }

  /** Deletes one of the chief's notes. */
  async deleteStaffNote(projectId: number, name: string): Promise<void> {
    await this.client.deleteStaffNote(projectId, name);
    const notes = this.get().memories[projectId]?.notes;
    if (notes) this.storeMemory(projectId, { notes: notes.filter((n) => n.name !== name) });
  }

  /**
   * Pauses or resumes its coordination. While paused, news from its agents
   * waits in its inbox; your messages still go in.
   */
  async setStaffPaused(projectId: number, paused: boolean): Promise<Staff> {
    const staff = await this.client.setStaff(projectId, { paused });
    this.storeStaff(staff);
    return staff;
  }

  /**
   * Sends the chief a message. It shows at once as pending, and leaves when
   * the transcript has it. A failure stays, to retry or discard.
   */
  async messageStaff(
    projectId: number,
    text: string,
    attachments: SentAttachment[] = [],
  ): Promise<void> {
    const staff = this.get().staffs[projectId];
    const afterN = staff?.agentId != null ? (this.get().transcripts[staff.agentId]?.next ?? 0) : 0;
    const item: PendingMessage = {
      id: this.nextPendingId++,
      text,
      sentMs: Date.now(),
      afterN,
      state: "sending",
      attachments,
    };
    this.set((s) => ({
      pending: { ...s.pending, [projectId]: [...(s.pending[projectId] ?? []), item] },
    }));
    await this.deliver(projectId, item);
  }

  private async deliver(projectId: number, item: PendingMessage): Promise<void> {
    const update = (change: Partial<PendingMessage>) =>
      this.set((s) => ({
        pending: {
          ...s.pending,
          [projectId]: (s.pending[projectId] ?? []).map((p) =>
            p.id === item.id ? { ...p, ...change } : p,
          ),
        },
      }));
    try {
      await this.client.messageStaff(
        projectId,
        item.text,
        (item.attachments ?? []).map((a) => a.name),
      );
      update({ state: "sent", error: undefined });
    } catch (error) {
      update({ state: "failed", error: message(error) });
      return;
    }
    // A first message starts the chief; either way its transcript grows.
    this.refreshStaff(projectId);
    const agentId = this.get().staffs[projectId]?.agentId;
    if (agentId != null) this.transcriptWatches.get(agentId)?.refresh.request();
  }

  /**
   * Sends an agent a message, typed into its terminal. It shows at once as
   * pending, and leaves when the transcript has it; a failure stays, to retry
   * or discard.
   */
  async messageAgent(
    agentId: number,
    text: string,
    attachments: SentAttachment[] = [],
  ): Promise<void> {
    const item: PendingMessage = {
      id: this.nextPendingId++,
      text,
      sentMs: Date.now(),
      afterN: this.get().transcripts[agentId]?.next ?? 0,
      state: "sending",
      attachments,
    };
    this.set((s) => ({
      agentPending: {
        ...s.agentPending,
        [agentId]: [...(s.agentPending[agentId] ?? []), item],
      },
    }));
    await this.deliverToAgent(agentId, item);
  }

  private async deliverToAgent(agentId: number, item: PendingMessage): Promise<void> {
    const update = (change: Partial<PendingMessage>) =>
      this.set((s) => ({
        agentPending: {
          ...s.agentPending,
          [agentId]: (s.agentPending[agentId] ?? []).map((p) =>
            p.id === item.id ? { ...p, ...change } : p,
          ),
        },
      }));
    try {
      await this.client.sendInput(
        agentId,
        item.text,
        (item.attachments ?? []).map((a) => a.name),
      );
      update({ state: "sent", error: undefined });
    } catch (error) {
      update({ state: "failed", error: message(error) });
      return;
    }
    this.transcriptWatches.get(agentId)?.refresh.request();
  }

  retryAgentPending(agentId: number, id: number): Promise<void> {
    const item = this.get().agentPending[agentId]?.find((p) => p.id === id);
    if (!item) return Promise.resolve();
    const retried: PendingMessage = { ...item, state: "sending", error: undefined };
    this.set((s) => ({
      agentPending: {
        ...s.agentPending,
        [agentId]: (s.agentPending[agentId] ?? []).map((p) => (p.id === id ? retried : p)),
      },
    }));
    return this.deliverToAgent(agentId, retried);
  }

  discardAgentPending(agentId: number, id: number): void {
    this.set((s) => ({
      agentPending: {
        ...s.agentPending,
        [agentId]: (s.agentPending[agentId] ?? []).filter((p) => p.id !== id),
      },
    }));
  }

  /** Uploads a file for a message to the chief or an agent; see `DaemonClient.uploadAttachment`. */
  uploadAttachment(target: AttachTarget, file: Blob & { name?: string }): Promise<Attachment> {
    return this.client.uploadAttachment(target, file);
  }

  /** An attachment of an agent's, as a `data:` URL to show; cached. */
  attachmentUrl(agentId: number, name: string): Promise<string> {
    const key = `${agentId}/${name}`;
    let url = this.attachmentUrls.get(key);
    if (!url) {
      url = this.client.attachmentUrl(agentId, name);
      url.catch(() => this.attachmentUrls.delete(key));
      this.attachmentUrls.set(key, url);
    }
    return url;
  }

  retryPending(projectId: number, id: number): Promise<void> {
    const item = this.get().pending[projectId]?.find((p) => p.id === id);
    if (!item) return Promise.resolve();
    const retried: PendingMessage = { ...item, state: "sending", error: undefined };
    this.set((s) => ({
      pending: {
        ...s.pending,
        [projectId]: (s.pending[projectId] ?? []).map((p) => (p.id === id ? retried : p)),
      },
    }));
    return this.deliver(projectId, retried);
  }

  discardPending(projectId: number, id: number): void {
    this.set((s) => ({
      pending: {
        ...s.pending,
        [projectId]: (s.pending[projectId] ?? []).filter((p) => p.id !== id),
      },
    }));
  }

  /** Stops the chief, waits for its agent to exit, and starts it again on the same session. */
  async restartStaff(projectId: number, timeoutMs = 10_000): Promise<Staff> {
    const agentId = this.get().staffs[projectId]?.agentId;
    const agent = agentId != null ? this.get().agents.find((a) => a.id === agentId) : undefined;
    if (agentId != null && agent && agent.state !== "exited" && agent.state !== "interrupted") {
      await this.client.stopAgent(agentId);
      this.agentsRefresh.request();
      await this.until((s) => {
        const state = s.agents.find((a) => a.id === agentId)?.state;
        return state === undefined || state === "exited" || state === "interrupted";
      }, timeoutMs);
    }
    return this.startStaff(projectId);
  }

  /** Resolves once the state passes the test, or after the timeout. */
  private until(test: (state: DaemonState) => boolean, timeoutMs: number): Promise<void> {
    if (test(this.get())) return Promise.resolve();
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        unsubscribe();
        resolve();
      };
      const timer = setTimeout(done, timeoutMs);
      const unsubscribe = this.state.subscribe((state) => {
        if (test(state)) done();
      });
    });
  }

  // MARK: Environment variables

  /**
   * Refetches a project's environment, or, for `null`, the global list and
   * every project's (each merge includes it).
   */
  refreshEnv(projectId: number | null): void {
    if (projectId !== null) {
      this.refreshProjectEnv(projectId);
      return;
    }
    this.globalEnvRefresh.request();
    for (const project of this.get().projects) this.refreshProjectEnv(project.id);
  }

  private refreshProjectEnv(projectId: number): void {
    let refresh = this.projectEnvRefreshes.get(projectId);
    if (!refresh) {
      refresh = new CoalescedRefresh(
        () => this.client.projectEnv(projectId),
        (result) => {
          if (result.ok) {
            this.set((s) => ({
              projectEnvs: { ...s.projectEnvs, [projectId]: result.value },
              envSupported: true,
              envErrors: withoutKey(s.envErrors, envKey(projectId)),
            }));
          } else if (
            isNotFound(result.error) &&
            !this.get().projects.some((p) => p.id === projectId)
          ) {
            return;
          } else this.envFailed(projectId, result.error);
        },
      );
      this.projectEnvRefreshes.set(projectId, refresh);
    }
    refresh.request();
  }

  private envFailed(projectId: number | null, error: unknown): void {
    if (isNotFound(error)) this.set({ envSupported: false });
    else this.set((s) => ({ envErrors: { ...s.envErrors, [envKey(projectId)]: message(error) } }));
  }

  /**
   * Sets a variable for every project (`projectId` null) or one. It applies
   * to agents launched or resumed after; running ones keep their values.
   * A refused name throws the daemon's reason.
   */
  async setEnv(projectId: number | null, name: string, value: string): Promise<void> {
    const answer = await this.client.setEnv(projectId, name, value);
    this.applyEnv(projectId, name, value, answer);
    this.refreshEnv(projectId);
  }

  async removeEnv(projectId: number | null, name: string): Promise<void> {
    const answer = await this.client.removeEnv(projectId, name);
    this.applyEnv(projectId, name, null, answer);
    this.refreshEnv(projectId);
  }

  /**
   * Shows a change at once: the daemon's answer when it has the lists, else
   * the change worked out here. The refetch after confirms it.
   */
  private applyEnv(
    projectId: number | null,
    name: string,
    value: string | null,
    answer: EnvAnswer,
  ): void {
    if (answer && "project" in answer && projectId !== null) {
      this.set((s) => ({ projectEnvs: { ...s.projectEnvs, [projectId]: answer.project } }));
      return;
    }
    const put = (vars: EnvVars): EnvVars => {
      if (answer && "global" in answer) return answer.global;
      const next = { ...vars };
      if (value === null) delete next[name];
      else next[name] = value;
      return next;
    };
    this.set((s) => {
      if (projectId === null) {
        const global = put(s.globalEnv ?? {});
        const projectEnvs = Object.fromEntries(
          Object.entries(s.projectEnvs).map(([id, env]) => [
            id,
            { ...env, global, effective: { ...global, ...env.variables } },
          ]),
        );
        return { globalEnv: global, projectEnvs };
      }
      const current = s.projectEnvs[projectId] ?? {
        variables: {},
        global: s.globalEnv ?? {},
        effective: {},
      };
      const variables = put(current.variables);
      return {
        projectEnvs: {
          ...s.projectEnvs,
          [projectId]: { ...current, variables, effective: { ...current.global, ...variables } },
        },
      };
    });
  }

  // MARK: Project and workspace actions

  async addProject(path: string): Promise<Project> {
    const project = await this.client.addProject(path);
    this.set((s) => {
      const index = s.projects.findIndex((p) => p.id === project.id);
      return {
        projects:
          index >= 0
            ? s.projects.map((p, i) => (i === index ? project : p))
            : [...s.projects, project],
      };
    });
    this.refreshProjects();
    return project;
  }

  async removeProject(id: number): Promise<void> {
    await this.client.removeProject(id);
    this.set((s) => ({ projects: s.projects.filter((p) => p.id !== id) }));
    this.refreshProjects();
  }

  async createWorkspace(projectId: number, base: string, title?: string): Promise<Workspace> {
    const workspace = await this.client.createWorkspace(projectId, base, title);
    this.updateProject(projectId, (p) => ({
      ...p,
      workspaces: [...p.workspaces.filter((w) => w.id !== workspace.id), workspace],
    }));
    this.refreshProjects();
    return workspace;
  }

  async renameWorkspace(workspace: Workspace, title: string): Promise<Workspace> {
    const renamed = await this.client.renameWorkspace(workspace.id, title);
    this.updateProject(workspace.projectId, (p) => ({
      ...p,
      workspaces: p.workspaces.map((w) => (w.id === renamed.id ? renamed : w)),
    }));
    this.refreshProjects();
    return renamed;
  }

  async removeWorkspace(workspace: Workspace): Promise<void> {
    await this.client.removeWorkspace(workspace.id);
    this.updateProject(workspace.projectId, (p) => ({
      ...p,
      workspaces: p.workspaces.filter((w) => w.id !== workspace.id),
    }));
    // The daemon forgets its agents with it.
    this.set((s) => ({ agents: s.agents.filter((a) => a.workspaceId !== workspace.id) }));
    this.refreshProjects();
    this.agentsRefresh.request();
  }

  private updateProject(id: number, change: (project: Project) => Project): void {
    this.set((s) => ({ projects: s.projects.map((p) => (p.id === id ? change(p) : p)) }));
  }

  // MARK: Agent actions

  async createAgent(workspaceId: number, request: NewAgent): Promise<Agent> {
    const agent = await this.client.createAgent(workspaceId, request);
    this.upsertAgent(agent);
    this.agentsRefresh.request();
    return agent;
  }

  /** Sends `/exit`. The state follows through events. */
  async stopAgent(id: number): Promise<void> {
    await this.client.stopAgent(id);
    this.agentsRefresh.request();
  }

  /**
   * Waits until agent `id` has stopped (exited or interrupted) or is gone,
   * as the event stream reports it. Rejects after `timeoutMs`.
   */
  untilStopped(id: number, timeoutMs = 15_000): Promise<void> {
    const stopped = (state: DaemonState) => {
      const agent = state.agents.find((a) => a.id === id);
      return !agent || agent.state === "exited" || agent.state === "interrupted";
    };
    if (stopped(this.get())) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        unsubscribe();
        reject(new Error("It didn't stop in time. Try again, or stop it from its page."));
      }, timeoutMs);
      const unsubscribe = this.state.subscribe((state) => {
        if (!stopped(state)) return;
        clearTimeout(timer);
        unsubscribe();
        resolve();
      });
    });
  }

  async resumeAgent(id: number): Promise<Agent> {
    const agent = await this.client.resumeAgent(id);
    this.upsertAgent(agent);
    this.agentsRefresh.request();
    return agent;
  }

  async removeAgent(id: number): Promise<void> {
    await this.client.removeAgent(id);
    this.set((s) => ({
      agents: s.agents.filter((a) => a.id !== id),
      approvals: s.approvals.filter((a) => a.agentId !== id),
    }));
    this.agentsRefresh.request();
  }

  async trustAgent(id: number): Promise<void> {
    await this.client.trustAgent(id);
    this.set((s) => ({
      agents: s.agents.map((a) =>
        a.id === id && a.state === "needs_trust" ? { ...a, state: "starting" } : a,
      ),
    }));
    this.agentsRefresh.request();
  }

  /**
   * Allows or denies. One already answered in the terminal (409) leaves the
   * list anyway, and the error is rethrown.
   */
  async decide(approval: Approval, decision: Decision, note?: string): Promise<Approval> {
    try {
      const decided = await this.client.decide(approval.id, decision, note);
      this.dropApproval(approval.id);
      return decided;
    } catch (error) {
      if (isConflict(error)) this.dropApproval(approval.id);
      throw error;
    } finally {
      this.approvalsRefresh.request();
      this.agentsRefresh.request();
    }
  }

  private dropApproval(id: number): void {
    this.set((s) => ({ approvals: s.approvals.filter((a) => a.id !== id) }));
  }

  private upsertAgent(agent: Agent): void {
    this.set((s) => {
      const index = s.agents.findIndex((a) => a.id === agent.id);
      return {
        agents:
          index >= 0 ? s.agents.map((a, i) => (i === index ? agent : a)) : [agent, ...s.agents],
      };
    });
  }

  // MARK: Hosts

  /**
   * Switches to another machine's daemon. The shell closes every stream; the
   * state starts over, and the new host's events are followed.
   */
  async switchHost(id: string): Promise<void> {
    const list = await this.bridge.hostSelect(id);
    await this.restartOn(list);
  }

  /** Saves a host; `switchHost` to use it. */
  async addHost(name: string, address: string): Promise<Host> {
    const host = await this.bridge.hostAdd(name, address);
    const list = await this.bridge.hostsList();
    this.set({ hosts: list.hosts });
    return host;
  }

  /**
   * The devices on this Mac's tailnet, always from This Mac's daemon, even
   * while another host is in use. Not kept in the state: the add-host sheet
   * and the host menu ask when they open (the daemon caches its probes).
   */
  async tailnetPeers(port?: number): Promise<TailnetPeers> {
    let body;
    try {
      body = await this.bridge.tailnetPeers(port);
    } catch (error) {
      throw CosError.from(error);
    }
    try {
      return decodeTailnetPeers(body);
    } catch (error) {
      if (error instanceof DecodeError) {
        throw new CosError("bad_response", `Unexpected tailnet devices from cos: ${error.message}`);
      }
      throw error;
    }
  }

  /** Renames a host, This Mac included. Nothing restarts: only the label changes. */
  async renameHost(id: string, name: string): Promise<void> {
    const list = await this.bridge.hostRename(id, name);
    const current = this.get().host;
    this.set({
      hosts: list.hosts,
      host: current ? (list.hosts.find((h) => h.id === current.id) ?? current) : current,
    });
  }

  /** Forgets a host; if it was in use, goes back to this Mac. */
  async removeHost(id: string): Promise<void> {
    const wasCurrent = this.get().host?.id === id;
    const list = await this.bridge.hostRemove(id);
    if (wasCurrent || list.current !== this.get().host?.id) await this.restartOn(list);
    else this.set({ hosts: list.hosts });
  }

  private async restartOn(list: HostList): Promise<void> {
    this.stop();
    this.streamId = null;
    this.disposeRefreshes();
    // Chiefs, conversations and inboxes belong to the old host. A page still
    // holding a watch lets go of nothing: its cleanup finds the watch gone.
    for (const refresh of this.staffRefreshes.values()) refresh.dispose();
    this.staffRefreshes.clear();
    this.staffStale = true;
    for (const refresh of this.projectEnvRefreshes.values()) refresh.dispose();
    this.projectEnvRefreshes.clear();
    this.envStale = true;
    for (const watch of this.transcriptWatches.values()) watch.refresh.dispose();
    this.transcriptWatches.clear();
    for (const watch of this.inboxWatches.values()) watch.refresh.dispose();
    this.inboxWatches.clear();
    for (const watch of this.memoryWatches.values()) watch.refresh.dispose();
    this.memoryWatches.clear();
    this.transcriptResets.clear();
    for (const timer of this.lateReads.values()) clearTimeout(timer);
    this.lateReads.clear();
    this.pendingEvents = [];
    const host = list.hosts.find((h) => h.id === list.current) ?? null;
    this.state.setState(
      { ...initialState(), info: this.get().info, hosts: list.hosts, host },
      true,
    );
    this.createRefreshes();
    await this.start();
  }

  // MARK: Ports forwarded from another machine

  /**
   * Follows the shell's forwards for the host in use. They live in the
   * shell, not the daemon, so they're followed even while the host refuses
   * this Mac. An older shell without forwarding leaves the list empty.
   */
  private async followForwards(current: () => boolean): Promise<void> {
    const generation = this.generation;
    let id: number;
    try {
      id = await this.bridge.followForwards((message) => {
        if (this.generation === generation) this.set({ forwards: message.forwards });
      });
    } catch {
      return;
    }
    if (current()) this.forwardsStreamId = id;
    else void this.bridge.closeStream(id);
  }

  /**
   * Forwards `remotePort` of the host in use to this Mac (the same port,
   * `localPort`, or the next free one). A `too_old` code means its cos
   * predates forwarding.
   */
  async addForward(remotePort: number, localPort?: number): Promise<Forward> {
    let forward: Forward;
    try {
      forward = await this.bridge.forwardAdd(remotePort, localPort);
    } catch (error) {
      throw CosError.from(error);
    }
    this.set((state) => ({
      forwards: [...state.forwards.filter((f) => f.remotePort !== remotePort), forward].sort(
        (a, b) => a.remotePort - b.remotePort,
      ),
    }));
    return forward;
  }

  /** Stops forwarding a port; its open connections drop. */
  async removeForward(remotePort: number): Promise<void> {
    let list;
    try {
      list = await this.bridge.forwardRemove(remotePort);
    } catch (error) {
      throw CosError.from(error);
    }
    this.set({ forwards: list.forwards });
  }

  /** Opens a forward's `http://localhost:{localPort}/` in the default browser. */
  async openForward(remotePort: number): Promise<void> {
    try {
      await this.bridge.forwardOpen(remotePort);
    } catch (error) {
      throw CosError.from(error);
    }
  }

  // MARK: jev, the judgment model in shadow mode

  refreshJudge(): void {
    this.judgeRefresh.request();
  }

  /** Switches jev on or off for a project, showing the answer at once. */
  async setProjectJudge(projectId: number, enabled: boolean): Promise<JudgeProject> {
    const answer = await this.client.setProjectJudge(projectId, enabled);
    this.set((s) =>
      s.judge
        ? {
            judge: {
              ...s.judge,
              projects: s.judge.projects.map((p) =>
                p.projectId === answer.projectId ? { ...p, enabled: answer.enabled } : p,
              ),
            },
          }
        : {},
    );
    this.judgeRefresh.request();
    return answer;
  }

  /** Looks for the API key again (the daemon probes the login shell). */
  async reloadJudge(): Promise<JudgeOverview> {
    const judge = await this.client.reloadJudge();
    this.set({ judge, judgeSupported: true, judgeError: null });
    return judge;
  }

  // MARK: Remote access (This Mac only)

  async setTailnet(enabled: boolean, port?: number): Promise<TailnetStatus> {
    const tailnet = await this.client.setTailnet(enabled, port);
    this.set({ tailnet });
    return tailnet;
  }

  approveDevice(id: number): Promise<Device> {
    return this.decideDevice(id, () => this.client.approveDevice(id));
  }

  denyDevice(id: number): Promise<Device> {
    return this.decideDevice(id, () => this.client.denyDevice(id));
  }

  async removeDevice(id: number): Promise<void> {
    await this.client.removeDevice(id);
    this.set((s) => ({ devices: s.devices.filter((d) => d.id !== id) }));
    this.devicesRefresh.request();
  }

  private async decideDevice(id: number, decide: () => Promise<Device>): Promise<Device> {
    const device = await decide();
    this.set((s) => ({ devices: s.devices.map((d) => (d.id === id ? device : d)) }));
    this.devicesRefresh.request();
    return device;
  }
}
