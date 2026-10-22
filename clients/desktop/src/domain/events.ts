// Which event kinds change what. Matched by prefix, so kinds added later
// still trigger a refresh. Ported from CosKit (Agents.swift, Runs.swift,
// Projects.swift).

import type { LoggedEvent } from "../api/models";
import { optNum } from "../api/decode";

export function changesProjects(kind: string): boolean {
  return kind.startsWith("project.") || kind.startsWith("workspace.");
}

/** `device.requested`, `.approved`, `.denied` and `.removed`. */
export function changesDevices(kind: string): boolean {
  return kind.startsWith("device.");
}

/**
 * An approval changes its agent's state and `pending_approval_id`; a removed
 * workspace takes its agents with it.
 */
export function changesAgents(kind: string): boolean {
  return kind.startsWith("agent.") || kind.startsWith("approval.") || kind === "workspace.removed";
}

export function changesApprovals(kind: string): boolean {
  return kind.startsWith("approval.") || kind === "agent.exited" || kind === "agent.removed";
}

export function changesRuns(kind: string): boolean {
  return (
    kind.startsWith("run.") ||
    kind.startsWith("plan.") ||
    kind.startsWith("task.") ||
    kind === "workspace.removed"
  );
}

export function changesQuestions(kind: string): boolean {
  return kind.startsWith("question.") || kind === "run.state_changed";
}

/** The run an event belongs to: the log's `run_id`, else `data.run_id`. */
export function runOf(event: LoggedEvent): number | null {
  if (event.runId !== null) {
    const id = optNum(event.runId);
    if (id !== null) return id;
  }
  return optNum(event.data.run_id);
}

/** What a page watching run `watched` must refetch after `event`. */
export interface RunRefresh {
  board: boolean;
  limits: boolean;
  report: boolean;
}

export function runRefreshFor(event: LoggedEvent, watched: number): RunRefresh {
  const run = runOf(event);
  const mine = run === null || run === watched;
  const approval = event.kind.startsWith("approval.");
  // A run's agents starting fill in `chief_agent_id` and tasks' `agent_id`.
  const agentStarted = event.kind === "agent.started";
  return {
    board: (changesRuns(event.kind) && mine) || agentStarted,
    limits: approval || (event.kind.startsWith("run.") && mine),
    report: mine && (event.kind === "run.report_written" || event.kind === "run.state_changed"),
  };
}

function scalar(value: unknown): string {
  if (value === null || value === undefined) return "–";
  if (typeof value === "string") return value.length > 80 ? `${value.slice(0, 77)}…` : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `[${value.map(scalar).join(", ")}]`;
  return "{…}";
}

/** `agent_id=7 state=working`, skipping what the row shows anyway. */
export function summarize(event: LoggedEvent): string {
  return Object.entries(event.data)
    .filter(([key]) => key !== "run_id")
    .map(([key, value]) => `${key}=${scalar(value)}`)
    .join("  ");
}
