// The daemon's state for the UI, kept current from `follow_events`: a port
// of CosKit's DaemonMonitor (and DaemonMonitor+Runs). Each event kind
// refetches what it changes, through coalesced refreshes, and actions apply
// the daemon's answer at once and then refresh.

import { createStore, type StoreApi } from "zustand/vanilla";
import { DaemonClient, type Decision, type NewAgent, type NewRun } from "../api/client";
import { CosError, isConflict, isNotFound } from "../api/errors";
import {
  type Agent,
  type Approval,
  type Board,
  type LoggedEvent,
  type Project,
  type Question,
  type Run,
  type RunLimits,
  type RunTask,
  type Status,
  type Workspace,
  decodeEvent,
  decodeStatus,
} from "../api/models";
import { type InboxItem, type Staff, type Transcript, type TranscriptMessage } from "../api/staff";
import type { Bridge, DaemonInfo, EventMessage } from "../bridge/types";
import {
  agentOf,
  changesAgents,
  changesApprovals,
  changesProjects,
  changesQuestions,
  changesRuns,
  changesStaff,
  projectOf,
  runRefreshFor,
} from "../domain/events";
import { type PendingMessage, mergeMessages, reconcilePending } from "../domain/transcript";
import { CoalescedRefresh, type Result } from "./coalesce";

export type Connection = "connecting" | "running" | "not_running" | "reconnecting";

export type ReportState =
  | { kind: "not_written" }
  | { kind: "markdown"; markdown: string }
  | { kind: "failed"; message: string };

export interface DaemonState {
  connection: Connection;
  /** Why the stream dropped, while reconnecting or not running. */
  connectionReason: string | null;
  status: Status | null;
  info: DaemonInfo | null;
  /** The most recent events, oldest first. */
  events: LoggedEvent[];
  /** In the order they were added, each with its workspaces. Kept while the daemon is away. */
  projects: Project[];
  projectsLoaded: boolean;
  projectsError: string | null;
  /** Every agent, newest first. */
  agents: Agent[];
  /** Pending approvals, oldest first, including ones routed to a chief. */
  approvals: Approval[];
  /** Whether the pending approvals have been fetched once. */
  approvalsLoaded: boolean;
  /** False when the daemon answers 404 for the agents routes. */
  agentsSupported: boolean;
  agentsError: string | null;
  /** Every run, newest first, with task counts. */
  runs: Run[];
  runsSupported: boolean;
  runsError: string | null;
  /** Open questions from chiefs, oldest first. */
  questions: Question[];
  questionsLoaded: boolean;
  questionsSupported: boolean;
  /** Boards of the runs a page watches. */
  boards: Record<number, Board>;
  boardErrors: Record<number, string>;
  limits: Record<number, RunLimits>;
  limitsSupported: boolean;
  reports: Record<number, ReportState>;
  /** Every approval, newest first, while a run page shows its activity. */
  allApprovals: Approval[];
  /** Each project's Chief of Staff, by project id. */
  staffs: Record<number, Staff>;
  /** False when the daemon answers 404 for the staff routes. */
  staffSupported: boolean;
  /** What the chief's agents reported, newest first, by project id, while a page shows it. */
  inboxes: Record<number, InboxItem[]>;
  /** Conversations read from transcripts, by agent id, while a page shows them. */
  transcripts: Record<number, TranscriptState>;
  /** What you sent each project's chief that its transcript doesn't show yet. */
  pending: Record<number, PendingMessage[]>;
}

export interface TranscriptState {
  messages: TranscriptMessage[];
  /** What to ask for next (`after=next`). */
  next: number;
  loaded: boolean;
  error: string | null;
}

export function initialState(): DaemonState {
  return {
    connection: "connecting",
    connectionReason: null,
    status: null,
    info: null,
    events: [],
    projects: [],
    projectsLoaded: false,
    projectsError: null,
    agents: [],
    approvals: [],
    approvalsLoaded: false,
    agentsSupported: true,
    agentsError: null,
    runs: [],
    runsSupported: true,
    runsError: null,
    questions: [],
    questionsLoaded: false,
    questionsSupported: true,
    boards: {},
    boardErrors: {},
    limits: {},
    limitsSupported: true,
    reports: {},
    allApprovals: [],
    staffs: {},
    staffSupported: true,
    inboxes: {},
    transcripts: {},
    pending: {},
  };
}

interface Watch<T> {
  count: number;
  refresh: CoalescedRefresh<T>;
}

interface RunWatch {
  count: number;
  board: CoalescedRefresh<Board>;
  limits: CoalescedRefresh<RunLimits>;
  report: CoalescedRefresh<string>;
}

export interface StoreOptions {
  /** How many events to keep in memory. */
  keep?: number;
  /** How many events before the newest to start with. */
  backlog?: number;
  /**
   * How long run refreshes wait, so a burst of events (approving a plan logs
   * one per task) costs one request.
   */
  runsDebounceMs?: number;
}

function message(error: unknown): string {
  return CosError.from(error).message;
}

export class CosStore {
  readonly state: StoreApi<DaemonState>;
  readonly client: DaemonClient;
  private readonly keep: number;
  private readonly backlog: number;
  private readonly runsDebounceMs: number;
  private streamId: number | null = null;
  private started = false;
  private pendingEvents: LoggedEvent[] = [];
  private flushScheduled = false;
  private readonly watches = new Map<number, RunWatch>();

  private readonly projectsRefresh: CoalescedRefresh<Project[]>;
  private readonly agentsRefresh: CoalescedRefresh<Agent[]>;
  private readonly approvalsRefresh: CoalescedRefresh<Approval[]>;
  private readonly runsRefresh: CoalescedRefresh<Run[]>;
  private readonly questionsRefresh: CoalescedRefresh<Question[]>;
  private readonly allApprovalsRefresh: CoalescedRefresh<Approval[]>;
  private readonly staffRefreshes = new Map<number, CoalescedRefresh<Staff>>();
  /** Set on each (re)connect: the next project list refetches every chief. */
  private staffStale = true;
  private readonly transcriptWatches = new Map<number, Watch<Transcript>>();
  /** Agents whose transcript restarted: the next fetch reads it from the top. */
  private readonly transcriptResets = new Set<number>();
  private readonly inboxWatches = new Map<number, Watch<InboxItem[]>>();
  private nextPendingId = 1;

  constructor(
    readonly bridge: Bridge,
    options: StoreOptions = {},
  ) {
    this.state = createStore<DaemonState>()(() => initialState());
    this.client = new DaemonClient(bridge);
    this.keep = options.keep ?? 1000;
    this.backlog = options.backlog ?? 200;
    this.runsDebounceMs = options.runsDebounceMs ?? 120;
    const client = this.client;

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
    this.runsRefresh = new CoalescedRefresh(
      () => client.runs(),
      (result) => {
        if (result.ok) this.set({ runs: result.value, runsSupported: true, runsError: null });
        else if (isNotFound(result.error))
          this.set({ runs: [], runsSupported: false, runsError: null });
        else this.set({ runsError: message(result.error) });
      },
      this.runsDebounceMs,
    );
    this.questionsRefresh = new CoalescedRefresh(
      () => client.questions("open"),
      (result) => {
        if (result.ok) {
          this.set({
            questions: result.value
              .filter((q) => q.state === "open")
              .sort((a, b) => a.createdMs - b.createdMs),
            questionsLoaded: true,
            questionsSupported: true,
          });
        } else if (isNotFound(result.error)) {
          this.set({ questions: [], questionsLoaded: true, questionsSupported: false });
        }
      },
      this.runsDebounceMs,
    );
    this.allApprovalsRefresh = new CoalescedRefresh(
      () => client.approvals(null),
      (result) => {
        if (result.ok) {
          this.set({ allApprovals: [...result.value].sort((a, b) => b.createdMs - a.createdMs) });
        }
      },
      this.runsDebounceMs,
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
    this.bridge.daemonInfo().then(
      (info) => this.set({ info }),
      () => undefined,
    );
    // Start a backlog before the newest event, so the log has some history
    // without replaying all of it.
    const after = await this.client.status().then(
      (status) => Math.max(0, status.lastSeq - this.backlog),
      () => null,
    );
    if (!this.started) return;
    try {
      const id = await this.bridge.followEvents(after, (message) => this.handle(message));
      if (this.started) this.streamId = id;
      else void this.bridge.closeStream(id);
    } catch (error) {
      const failure = CosError.from(error);
      this.set({
        connection: failure.kind === "not_running" ? "not_running" : "reconnecting",
        connectionReason: failure.message,
      });
    }
  }

  stop(): void {
    this.started = false;
    if (this.streamId !== null) void this.bridge.closeStream(this.streamId);
    this.streamId = null;
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
        this.set({ status, connection: "running", connectionReason: null });
        this.staffStale = true;
        this.refreshProjects();
        this.refreshAgents();
        this.refreshRuns();
        // What changed while away: the conversations and inboxes on screen.
        for (const watch of this.transcriptWatches.values()) watch.refresh.request();
        for (const watch of this.inboxWatches.values()) watch.refresh.request();
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
          connection: message.notRunning ? "not_running" : "reconnecting",
          connectionReason: message.reason,
        });
        break;
    }
  }

  private applyEvent(event: LoggedEvent): void {
    const kind = event.kind;
    if (changesProjects(kind)) this.projectsRefresh.request();
    if (changesAgents(kind)) this.agentsRefresh.request();
    if (changesApprovals(kind)) this.approvalsRefresh.request();
    if (changesRuns(kind)) this.runsRefresh.request();
    if (changesQuestions(kind)) this.questionsRefresh.request();
    if (changesStaff(kind)) {
      const project = projectOf(event);
      if (project !== null) this.refreshStaff(project);
      else for (const p of this.get().projects) this.refreshStaff(p.id);
    }
    if (kind === "staff.inbox") {
      const project = projectOf(event);
      for (const [id, watch] of this.inboxWatches) {
        if (project === null || project === id) watch.refresh.request();
      }
    }
    if (kind === "agent.transcript") this.transcriptGrew(event);
    if (this.watches.size > 0) {
      for (const [id, watch] of this.watches) {
        const refresh = runRefreshFor(event, id);
        if (refresh.board) watch.board.request();
        if (refresh.limits) watch.limits.request();
        if (refresh.report) watch.report.request();
      }
      if (kind.startsWith("approval.")) this.allApprovalsRefresh.request();
    }
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

  refreshAgents(): void {
    this.agentsRefresh.request();
    this.approvalsRefresh.request();
  }

  /** The run list, open questions, and every watched run. */
  refreshRuns(): void {
    this.runsRefresh.request();
    this.questionsRefresh.request();
    for (const [id, watch] of this.watches) {
      watch.board.request();
      watch.limits.request();
      if (this.get().reports[id] !== undefined) watch.report.request();
    }
    if (this.watches.size > 0) this.allApprovalsRefresh.request();
  }

  // MARK: Watching a run

  /**
   * Keeps a run's board, limits, report and approvals current until the
   * returned function is called. Calls are counted, so two pages can watch
   * one run.
   */
  watchRun(id: number): () => void {
    const existing = this.watches.get(id);
    if (existing) {
      existing.count += 1;
    } else {
      const client = this.client;
      const watch: RunWatch = {
        count: 1,
        board: new CoalescedRefresh(
          () => client.board(id),
          (result) => {
            if (result.ok) this.storeBoard(result.value);
            else
              this.set((s) => ({ boardErrors: { ...s.boardErrors, [id]: message(result.error) } }));
          },
          this.runsDebounceMs,
        ),
        limits: new CoalescedRefresh(
          () => client.limits(id),
          (result) => {
            if (result.ok) {
              this.set((s) => ({
                limits: { ...s.limits, [id]: result.value },
                limitsSupported: true,
              }));
            } else if (isNotFound(result.error)) {
              // No limits route, or no such run; the board says which.
              this.set({ limitsSupported: false });
            }
          },
          this.runsDebounceMs,
        ),
        report: new CoalescedRefresh(
          () => client.report(id),
          (result) => {
            const report: ReportState = result.ok
              ? { kind: "markdown", markdown: result.value }
              : isNotFound(result.error)
                ? { kind: "not_written" }
                : { kind: "failed", message: message(result.error) };
            this.set((s) => ({ reports: { ...s.reports, [id]: report } }));
          },
          this.runsDebounceMs,
        ),
      };
      this.watches.set(id, watch);
      watch.board.request();
      watch.limits.request();
      this.allApprovalsRefresh.request();
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      this.unwatchRun(id);
    };
  }

  private unwatchRun(id: number): void {
    const watch = this.watches.get(id);
    if (!watch) return;
    watch.count -= 1;
    if (watch.count <= 0) {
      watch.board.dispose();
      watch.limits.dispose();
      watch.report.dispose();
      this.watches.delete(id);
    }
  }

  isWatching(id: number): boolean {
    return this.watches.has(id);
  }

  /** Keeps a board and the run list in step, and fetches the report once there may be one. */
  private storeBoard(board: Board): void {
    const id = board.run.id;
    this.set((s) => {
      const run: Run = { ...board.run, counts: board.counts };
      const index = s.runs.findIndex((r) => r.id === id);
      const runs = index >= 0 ? s.runs.map((r, i) => (i === index ? run : r)) : [run, ...s.runs];
      const boardErrors = { ...s.boardErrors };
      delete boardErrors[id];
      return { boards: { ...s.boards, [id]: board }, boardErrors, runs };
    });
    const mayHaveReport =
      board.run.state === "done" ||
      board.run.state === "reporting" ||
      board.run.reportPath !== null;
    const watch = this.watches.get(id);
    if (mayHaveReport && this.get().reports[id] === undefined && watch) watch.report.request();
  }

  // MARK: The Chief of Staff

  /**
   * After the project list loads: fetch the chief of every project after a
   * (re)connect, else only of projects that are new, and forget removed ones.
   */
  private followProjects(projects: Project[]): void {
    const all = this.staffStale;
    this.staffStale = false;
    const ids = new Set(projects.map((p) => p.id));
    const staffs = this.get().staffs;
    for (const project of projects) {
      if (all || staffs[project.id] === undefined) this.refreshStaff(project.id);
    }
    if (Object.keys(staffs).some((id) => !ids.has(Number(id)))) {
      this.set((s) => ({
        staffs: Object.fromEntries(Object.entries(s.staffs).filter(([id]) => ids.has(Number(id)))),
      }));
    }
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
      return { transcripts, pending };
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

  /** A counted watch: the first one creates and fetches; the last one disposes. */
  private watch<T>(
    watches: Map<number, Watch<T>>,
    id: number,
    create: () => CoalescedRefresh<T>,
  ): () => void {
    const existing = watches.get(id);
    if (existing) existing.count += 1;
    else {
      const watch = { count: 1, refresh: create() };
      watches.set(id, watch);
      watch.refresh.request();
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      const watch = watches.get(id);
      if (!watch) return;
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

  /** `null` for the default model. It applies from the chief's next start. */
  async setStaffModel(projectId: number, model: string | null): Promise<Staff> {
    const staff = await this.client.setStaff(projectId, { model });
    this.storeStaff(staff);
    return staff;
  }

  /**
   * Sends the chief a message. It shows at once as pending, and leaves when
   * the transcript has it. A failure stays, to retry or discard.
   */
  async messageStaff(projectId: number, text: string): Promise<void> {
    const staff = this.get().staffs[projectId];
    const afterN = staff?.agentId != null ? (this.get().transcripts[staff.agentId]?.next ?? 0) : 0;
    const item: PendingMessage = {
      id: this.nextPendingId++,
      text,
      sentMs: Date.now(),
      afterN,
      state: "sending",
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
      await this.client.messageStaff(projectId, item.text);
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
    this.refreshProjects();
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
      if (this.watches.size > 0) this.allApprovalsRefresh.request();
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

  // MARK: Run actions

  async createRun(request: NewRun): Promise<Run> {
    const run = await this.client.createRun(request);
    this.set((s) => ({ runs: [run, ...s.runs.filter((r) => r.id !== run.id)] }));
    this.runsRefresh.request();
    return run;
  }

  async approvePlan(runId: number): Promise<Board> {
    const board = await this.client.approvePlan(runId);
    this.storeBoard(board);
    this.runsRefresh.request();
    return board;
  }

  async cancelRun(runId: number, reason?: string): Promise<Board> {
    const board = await this.client.cancelRun(runId, reason);
    this.storeBoard(board);
    this.runsRefresh.request();
    this.questionsRefresh.request();
    return board;
  }

  /** Retry (`ready`), fail or cancel a task. */
  async updateTask(task: RunTask, state: string, reason?: string): Promise<Board> {
    const board = await this.client.updateTask(task.id, state, reason);
    this.storeBoard(board);
    this.runsRefresh.request();
    return board;
  }

  /** One answered elsewhere meanwhile (409) leaves the list anyway; the error is rethrown. */
  async answer(question: Question, text: string): Promise<Question> {
    try {
      const answered = await this.client.answer(question.id, text);
      this.dropQuestion(question.id);
      return answered;
    } catch (error) {
      if (isConflict(error)) this.dropQuestion(question.id);
      throw error;
    } finally {
      this.questionsRefresh.request();
    }
  }

  private dropQuestion(id: number): void {
    this.set((s) => ({ questions: s.questions.filter((q) => q.id !== id) }));
  }

  messageChief(runId: number, text: string): Promise<void> {
    return this.client.messageChief(runId, text);
  }

  async setLimits(runId: number, preset: string, approvalCap: number): Promise<RunLimits> {
    const limits = await this.client.setLimits(runId, { preset, approvalCap });
    this.set((s) => ({ limits: { ...s.limits, [runId]: limits } }));
    this.watches.get(runId)?.board.request();
    this.runsRefresh.request();
    return limits;
  }
}
