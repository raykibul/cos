// Lookups over the store's state. Pure functions, so components and tests
// share them.

import type { Agent, Approval, Device, Project, Question, Run, Workspace } from "../api/models";
import { isForYou } from "../domain/approvals";
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

export function agent(s: DaemonState, id: number | null | undefined): Agent | undefined {
  if (id === null || id === undefined) return undefined;
  return s.agents.find((a) => a.id === id);
}

/** Your agents in a workspace, newest first; a run's are on its page. */
export function userAgents(s: DaemonState, workspaceId: number): Agent[] {
  return s.agents.filter((a) => a.workspaceId === workspaceId && a.runId === null);
}

/** The approval an agent waits on, if any. */
export function pendingApproval(s: DaemonState, agentId: number): Approval | undefined {
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
 * agents on the trust dialog, chiefs' open questions, plans to approve, and
 * devices asking to reach this Mac.
 */
export function waitingCount(s: DaemonState): number {
  return (
    approvalsForYou(s).length +
    trustAgents(s).length +
    s.questions.length +
    plansAwaitingApproval(s).length +
    pendingDevices(s).length
  );
}

/** Whether an agent waits on you: the trust dialog, or an approval routed to you. */
export function agentNeedsYou(s: DaemonState, a: Agent): boolean {
  if (a.state === "needs_trust") return true;
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
