// Lookups over the store's state. Pure functions, so components and tests
// share them.

import type {
  Agent,
  AgentState,
  Approval,
  Device,
  Project,
  Question,
  Run,
  Workspace,
} from "../api/models";
import { CLAUDE_CONFIG_DIR, accountOf, sortedNames } from "../api/env";
import type { Page } from "../app/ui";
import { isForYou } from "../domain/approvals";
import { isAlive } from "../domain/labels";
import { isRunFinished } from "../domain/tasks";
import type { DaemonState } from "./store";

export function project(s: DaemonState, id: number): Project | undefined {
  return s.projects.find((p) => p.id === id);
}

export function workspace(s: DaemonState, id: number | null | undefined): Workspace | undefined {
  if (id === null || id === undefined) return undefined;
  for (const p of s.projects) {
    const found = p.workspaces.find((w) => w.id === id);
    if (found) return found;
  }
  return undefined;
}

export function agent(
  s: Pick<DaemonState, "agents">,
  id: number | null | undefined,
): Agent | undefined {
  if (id === null || id === undefined) return undefined;
  return s.agents.find((a) => a.id === id);
}

/** Your agents in a workspace, newest first; a run's are on its page. */
export function userAgents(s: DaemonState, workspaceId: number): Agent[] {
  return s.agents.filter((a) => a.workspaceId === workspaceId && a.runId === null);
}

/** The approval an agent waits on, if any. */
export function pendingApproval(
  s: Pick<DaemonState, "approvals">,
  agentId: number,
): Approval | undefined {
  return s.approvals.find((a) => a.agentId === agentId && a.state === "pending");
}

/** Pending approvals that are yours to answer: not routed to a chief. */
export function approvalsForYou(s: DaemonState): Approval[] {
  return s.approvals.filter(isForYou);
}

/** Agents stuck on Claude's trust dialog. */
export function trustAgents(s: DaemonState): Agent[] {
  return s.agents.filter((a) => a.state === "needs_trust");
}

/** Agents on Claude's login screen: their CLAUDE_CONFIG_DIR isn't logged in. */
export function loginAgents(s: DaemonState): Agent[] {
  return s.agents.filter((a) => a.state === "needs_login");
}

/** Runs whose plan waits for you, oldest first. */
export function plansAwaitingApproval(s: DaemonState): Run[] {
  return s.runs
    .filter((r) => r.state === "awaiting_plan_approval")
    .sort((a, b) => a.createdAt - b.createdAt);
}

/** Devices asking to reach this Mac over Tailscale, oldest first. */
export function pendingDevices(s: DaemonState): Device[] {
  return s.devices.filter((d) => d.state === "pending");
}

/**
 * Everything blocked on you: approvals routed to you (not to a chief),
 * agents on the trust dialog or the login screen, chiefs' open questions, plans to approve, and
 * devices asking to reach this Mac.
 */
export function waitingCount(s: DaemonState): number {
  return (
    approvalsForYou(s).length +
    trustAgents(s).length +
    loginAgents(s).length +
    s.questions.length +
    plansAwaitingApproval(s).length +
    pendingDevices(s).length
  );
}

/** Whether an agent waits on you: the trust dialog, the login screen, or an approval routed to you. */
export function agentNeedsYou(s: DaemonState, a: Agent): boolean {
  if (a.state === "needs_trust" || a.state === "needs_login") return true;
  const approval = pendingApproval(s, a.id);
  if (approval) return isForYou(approval);
  return a.state === "waiting_approval";
}

/** The run with its counts: from its board when a page shows it. */
export function run(s: DaemonState, id: number): Run | undefined {
  const board = s.boards[id];
  if (board) return { ...board.run, counts: board.counts };
  return s.runs.find((r) => r.id === id);
}

/** A workspace's runs, newest first. */
export function runsIn(s: DaemonState, workspaceId: number): Run[] {
  return s.runs.filter((r) => r.workspaceId === workspaceId);
}

/** The workspace's unfinished run. There is at most one. */
export function currentRun(s: DaemonState, workspaceId: number): Run | undefined {
  return s.runs.find((r) => r.workspaceId === workspaceId && !isRunFinished(r.state));
}

export function questionsFor(s: DaemonState, runId: number): Question[] {
  return s.questions.filter((q) => q.runId === runId);
}

/** Agents a run started: its chief, workers and reviewers. */
export function agentsForRun(s: DaemonState, runId: number): Agent[] {
  return s.agents.filter((a) => a.runId === runId);
}

/**
 * Every approval of a run's agents, newest first, including the ones rules
 * allowed. Pending ones come from the live list too, in case the full list lags.
 */
export function approvalsForRun(s: DaemonState, runId: number): Approval[] {
  const ids = new Set(agentsForRun(s, runId).map((a) => a.id));
  const seen = new Set<number>();
  return [...s.approvals, ...s.allApprovals]
    .filter((a) => ids.has(a.agentId))
    .filter((a) => (seen.has(a.id) ? false : (seen.add(a.id), true)))
    .sort((a, b) => b.createdMs - a.createdMs);
}

/** How many things in a run wait on you: its plan, its questions, and its agents' approvals routed to you. */
export function waitingCountForRun(s: DaemonState, runId: number): number {
  const ids = new Set(agentsForRun(s, runId).map((a) => a.id));
  const plan = run(s, runId)?.state === "awaiting_plan_approval" ? 1 : 0;
  return (
    plan +
    questionsFor(s, runId).length +
    approvalsForYou(s).filter((a) => ids.has(a.agentId)).length
  );
}

// MARK: The Chief of Staff and workspaces

/** A project's workspaces, newest first. */
export function workspacesNewestFirst(project: Project): Workspace[] {
  return [...project.workspaces].sort((a, b) => b.createdAt - a.createdAt || b.id - a.id);
}

/** The chief's runtime agent, once it has started. */
export function staffAgent(s: DaemonState, projectId: number): Agent | undefined {
  return agent(s, s.staffs[projectId]?.agentId);
}

/**
 * The chief's state: its agent's, which events keep current, else the
 * staff record's; null before it first starts.
 */
export function staffState(s: DaemonState, projectId: number): AgentState | null {
  const staff = s.staffs[projectId];
  if (!staff || staff.agentId === null) return null;
  return staffAgent(s, projectId)?.state ?? staff.state;
}

/** Agents the chief started. */
export function spawnedAgents(
  s: Pick<DaemonState, "agents">,
  staffAgentId: number | null | undefined,
): Agent[] {
  if (staffAgentId === null || staffAgentId === undefined) return [];
  return s.agents.filter((a) => a.spawnedBy === staffAgentId);
}

/** Something one of the chief's agents (or the chief) waits on you for. */
export interface ChiefRequest {
  agent: Agent;
  /** Its approval routed to you; none for the trust dialog or the login screen (see `agent.state`). */
  approval: Approval | undefined;
}

/** The chief's own requests and those of the agents it started that wait on you, oldest first. */
export function chiefRequests(
  s: Pick<DaemonState, "agents" | "approvals" | "staffs">,
  projectId: number,
): ChiefRequest[] {
  const chiefId = s.staffs[projectId]?.agentId;
  if (chiefId === null || chiefId === undefined) return [];
  const chief = agent(s, chiefId);
  const agents = [...(chief ? [chief] : []), ...spawnedAgents(s, chiefId)];
  const requests: ChiefRequest[] = [];
  for (const a of agents) {
    const approval = pendingApproval(s, a.id);
    if (approval && isForYou(approval)) requests.push({ agent: a, approval });
    else if (a.state === "needs_trust" || a.state === "needs_login")
      requests.push({ agent: a, approval: undefined });
  }
  return requests.sort(
    (x, y) =>
      (x.approval?.createdMs ?? x.agent.lastActivityMs ?? 0) -
      (y.approval?.createdMs ?? y.agent.lastActivityMs ?? 0),
  );
}

/** How a set of agents is doing, for a status dot: waiting on you, working, idle or nothing. */
export type Activity = "attention" | "working" | "idle" | "none";

export function activityOf(s: DaemonState, agents: Agent[]): Activity {
  if (agents.some((a) => agentNeedsYou(s, a))) return "attention";
  if (agents.some((a) => a.state === "working" || a.state === "starting")) return "working";
  if (agents.some((a) => isAlive(a.state))) return "idle";
  return "none";
}

/** Every agent in a workspace, yours and a run's. */
export function workspaceAgents(s: DaemonState, workspaceId: number): Agent[] {
  return s.agents.filter((a) => a.workspaceId === workspaceId);
}

/** A project's chief row: the chief itself, and anything in its workspaces waiting on you. */
export function projectActivity(s: DaemonState, project: Project): Activity {
  const ids = new Set(project.workspaces.map((w) => w.id));
  const agents = s.agents.filter((a) => a.workspaceId !== null && ids.has(a.workspaceId));
  const chief = staffAgent(s, project.id);
  if (chief) agents.push(chief);
  const activity = activityOf(s, agents);
  // The chief's own idleness is the row's resting state; it says nothing.
  return activity === "idle" ? "none" : activity;
}

/** The workspace a page shows, directly or through one of its agents or runs. */
export function workspaceOfPage(s: DaemonState, page: Page): number | null {
  switch (page.kind) {
    case "workspace":
      return page.id;
    case "agent":
      return agent(s, page.id)?.workspaceId ?? null;
    case "run":
      return run(s, page.id)?.workspaceId ?? null;
    default:
      return null;
  }
}

/** The project a page belongs to, for unfolding it in the sidebar. */
export function projectOfPage(s: DaemonState, page: Page): number | null {
  if (page.kind === "chief" || page.kind === "project" || page.kind === "projectEnv")
    return page.id;
  const ws = workspace(s, workspaceOfPage(s, page));
  if (ws) return ws.projectId;
  if (page.kind === "agent") {
    for (const [project, staff] of Object.entries(s.staffs)) {
      if (staff.agentId === page.id) return Number(project);
    }
  }
  return null;
}

// MARK: Environment variables

/** A project's own variable names, CLAUDE_CONFIG_DIR first. */
export function projectEnvNames(s: DaemonState, projectId: number): string[] {
  return sortedNames(Object.keys(s.projectEnvs[projectId]?.variables ?? {}));
}

/** The project an agent runs for: its workspace's, or the one whose Chief of Staff it is. */
export function projectOfAgent(s: DaemonState, a: Agent): number | null {
  const ws = workspace(s, a.workspaceId);
  if (ws) return ws.projectId;
  for (const [project, staff] of Object.entries(s.staffs)) {
    if (staff.agentId === a.id) return Number(project);
  }
  return null;
}

/**
 * The CLAUDE_CONFIG_DIR cos gives an agent's project, as stored (`~/…`), or
 * null when cos sets none (the daemon's own, or Claude's default, applies).
 */
export function configDirOf(s: DaemonState, a: Agent): string | null {
  const project = projectOfAgent(s, a);
  const env = project !== null ? s.projectEnvs[project]?.effective : s.globalEnv;
  return env?.[CLAUDE_CONFIG_DIR] ?? null;
}

/** The Claude account a project's agents use, from CLAUDE_CONFIG_DIR, its own or inherited. */
export interface ClaudeAccount {
  /** The value's last path segment: `.claude-personal`. */
  account: string;
  value: string;
  inherited: boolean;
}

export function claudeAccount(s: DaemonState, projectId: number): ClaudeAccount | null {
  const env = s.projectEnvs[projectId];
  const value = env?.effective[CLAUDE_CONFIG_DIR];
  const account = accountOf(value);
  if (!env || value === undefined || !account) return null;
  return { account, value, inherited: !(CLAUDE_CONFIG_DIR in env.variables) };
}

/** The current run and the latest two finished ones, plus the selected one. */
export function sidebarRuns(
  s: DaemonState,
  workspaceId: number,
  selectedRun: number | null,
): Run[] {
  const runs = runsIn(s, workspaceId);
  const shown = [
    ...runs.filter((r) => !isRunFinished(r.state)),
    ...runs.filter((r) => isRunFinished(r.state)).slice(0, 2),
  ];
  if (selectedRun !== null && !shown.some((r) => r.id === selectedRun)) {
    const selected = runs.find((r) => r.id === selectedRun);
    if (selected) shown.push(selected);
  }
  return shown;
}
