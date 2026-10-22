// What the ⌘K switcher lists: every project's chief, what waits on you,
// your agents, the workspaces, and the app's own pages. Pure, so the list
// and its filtering are tested without the dialog.

import { workspaceName } from "../api/models";
import type { Page } from "../app/ui";
import { staffLabel } from "../pages/chief/staffState";
import {
  agent,
  approvalsForYou,
  loginAgents,
  pendingDevices,
  plansAwaitingApproval,
  project,
  projectOfAgent,
  projectWaitingCount,
  run,
  staffState,
  trustAgents,
  workspace,
} from "../store/selectors";
import type { DaemonState } from "../store/store";
import { approvalDetail } from "./approvals";
import { agentLabel, isAlive, pluralize } from "./labels";
import { projectShortcut } from "./rail";

export type SwitchSection = "Chiefs" | "Waiting on you" | "Agents" | "Workspaces" | "Go to";

export const sections: SwitchSection[] = [
  "Chiefs",
  "Waiting on you",
  "Agents",
  "Workspaces",
  "Go to",
];

export interface SwitchItem {
  /** Unique in the list. */
  key: string;
  section: SwitchSection;
  title: string;
  detail: string;
  page: Page;
  /** The project whose chief this is: Tab writes to it without opening it. */
  chiefOf: number | null;
  shortcut: string | null;
}

/** The project's name, when an agent or a workspace belongs to one. */
function projectName(s: DaemonState, projectId: number | null): string | null {
  return projectId !== null ? (project(s, projectId)?.name ?? null) : null;
}

function joined(...parts: (string | null | undefined)[]): string {
  return parts.filter((p): p is string => !!p).join(" · ");
}

export function switchItems(s: DaemonState): SwitchItem[] {
  const items: SwitchItem[] = [];

  s.projects.forEach((p, index) => {
    const waiting = projectWaitingCount(s, p);
    items.push({
      key: `chief:${p.id}`,
      section: "Chiefs",
      title: `Chief of Staff · ${p.name}`,
      detail: joined(
        staffLabel(staffState(s, p.id)).title,
        waiting > 0 ? `${waiting} waiting on you` : null,
      ),
      page: { kind: "chief", id: p.id },
      chiefOf: p.id,
      shortcut: projectShortcut(index),
    });
  });

  const waiting = (key: string, title: string, detail: string, page: Page) =>
    items.push({
      key,
      section: "Waiting on you",
      title,
      detail,
      page,
      chiefOf: null,
      shortcut: null,
    });
  for (const approval of approvalsForYou(s)) {
    const a = agent(s, approval.agentId);
    waiting(
      `approval:${approval.id}`,
      `${a?.title ?? "An agent"} wants to use ${approval.toolName}`,
      joined(approvalDetail(approval), a ? projectName(s, projectOfAgent(s, a)) : null),
      { kind: "agent", id: approval.agentId },
    );
  }
  for (const a of trustAgents(s)) {
    waiting(
      `trust:${a.id}`,
      `${a.title} needs you to trust its folder`,
      joined(projectName(s, projectOfAgent(s, a))),
      { kind: "agent", id: a.id },
    );
  }
  for (const a of loginAgents(s)) {
    waiting(
      `login:${a.id}`,
      `${a.title} needs a Claude login`,
      joined(projectName(s, projectOfAgent(s, a))),
      { kind: "agent", id: a.id },
    );
  }
  for (const r of plansAwaitingApproval(s)) {
    const ws = workspace(s, r.workspaceId);
    waiting(
      `plan:${r.id}`,
      `Plan ready: ${r.goal}`,
      joined(ws ? workspaceName(ws) : null, ws ? projectName(s, ws.projectId) : null),
      { kind: "run", id: r.id },
    );
  }
  for (const q of s.questions) {
    const r = q.runId !== null ? run(s, q.runId) : undefined;
    waiting(
      `question:${q.id}`,
      q.question,
      r ? `Asked in the run "${r.goal}"` : "An open question",
      r ? { kind: "run", id: r.id } : { kind: "waiting" },
    );
  }
  const devices = pendingDevices(s);
  if (devices.length > 0) {
    waiting(
      "devices",
      `${pluralize(devices.length, "device")} asking to reach this Mac`,
      devices.map((d) => d.name).join(", "),
      { kind: "waiting" },
    );
  }

  for (const a of s.agents) {
    if (a.runId !== null || a.workspaceId === null || !isAlive(a.state)) continue;
    const ws = workspace(s, a.workspaceId);
    items.push({
      key: `agent:${a.id}`,
      section: "Agents",
      title: a.title,
      detail: joined(
        agentLabel(a.state).title,
        ws ? workspaceName(ws) : null,
        ws ? projectName(s, ws.projectId) : null,
      ),
      page: { kind: "agent", id: a.id },
      chiefOf: null,
      shortcut: null,
    });
  }

  for (const p of s.projects) {
    for (const ws of p.workspaces) {
      items.push({
        key: `workspace:${ws.id}`,
        section: "Workspaces",
        title: workspaceName(ws),
        detail: joined(p.name, ws.branch),
        page: { kind: "workspace", id: ws.id },
        chiefOf: null,
        shortcut: null,
      });
    }
  }

  const go = (key: string, title: string, detail: string, page: Page) =>
    items.push({ key, section: "Go to", title, detail, page, chiefOf: null, shortcut: null });
  go("waiting", "Waiting on you", "Everything blocked on you", { kind: "waiting" });
  go("events", "Event log", "Everything the daemon logged", { kind: "events" });
  go("settings", "Settings", "Machines, environment and remote access", { kind: "settings" });

  return items;
}

/** How many of a section show before you type anything. */
const restingLimit: Record<SwitchSection, number> = {
  Chiefs: 9,
  "Waiting on you": 5,
  Agents: 5,
  Workspaces: 0,
  "Go to": 3,
};

/**
 * The items that match every word of `query` (in their title, detail or
 * section), titles that start with the query first. An empty query shows a
 * few of each section and no workspaces.
 */
export function filterItems(items: SwitchItem[], query: string): SwitchItem[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) {
    const shown: Partial<Record<SwitchSection, number>> = {};
    return items.filter((item) => {
      const n = shown[item.section] ?? 0;
      shown[item.section] = n + 1;
      return n < restingLimit[item.section];
    });
  }
  const q = query.trim().toLowerCase();
  const matches = items.filter((item) => {
    const text = `${item.title} ${item.detail} ${item.section}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
  const rank = (item: SwitchItem) => {
    const title = item.title.toLowerCase();
    const bare = title.replace(/^chief of staff · /, "");
    return title.startsWith(q) || bare.startsWith(q) ? 0 : 1;
  };
  return sections.flatMap((section) =>
    matches
      .filter((item) => item.section === section)
      .map((item, index) => ({ item, index }))
      .sort((a, b) => rank(a.item) - rank(b.item) || a.index - b.index)
      .map(({ item }) => item),
  );
}
