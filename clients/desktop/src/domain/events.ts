// Which event kinds change what. Matched by prefix, so kinds added later
// still trigger a refresh. Ported from CosKit (Agents.swift,
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
  // A transcript growing (once a second while Claude talks) changes no agent.
  if (kind === "agent.transcript") return false;
  return (
    kind.startsWith("agent.") ||
    kind.startsWith("approval.") ||
    kind === "workspace.removed" ||
    // Starting the Chief of Staff starts its agent.
    kind === "staff.started"
  );
}

/**
 * A project's Chief of Staff record: its agent appears when it starts. Its
 * state and counts follow from the agents list, so agent events don't
 * refetch it.
 */
export function changesStaff(kind: string): boolean {
  return kind === "staff.started" || kind === "staff.changed" || kind === "staff.stopped";
}

/** `env.changed {project_id|null, name, action}`: a project's list, or the global one. */
export function changesEnv(kind: string): boolean {
  return kind.startsWith("env.");
}

/** `judge.decided` (a new decision) and `judge.changed` (a project switched, the key reloaded). */
export function changesJudge(kind: string): boolean {
  return kind.startsWith("judge.");
}

/** The project a staff or env event is about. */
export function projectOf(event: LoggedEvent): number | null {
  return optNum(event.data.project_id);
}

/** The agent an event is about. */
export function agentOf(event: LoggedEvent): number | null {
  return optNum(event.data.agent_id);
}

export function changesApprovals(kind: string): boolean {
  return kind.startsWith("approval.") || kind === "agent.exited" || kind === "agent.removed";
}

function scalar(value: unknown): string {
  if (value === null || value === undefined) return "–";
  if (typeof value === "string") return value.length > 80 ? `${value.slice(0, 77)}…` : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `[${value.map(scalar).join(", ")}]`;
  return "{…}";
}

/** `agent_id=7 state=working`. */
export function summarize(event: LoggedEvent): string {
  return Object.entries(event.data)
    .map(([key, value]) => `${key}=${scalar(value)}`)
    .join("  ");
}
