// What a tab across the top says about its page: a title, a kind for its
// icon, and, for agents, their state for a dot. Plain values only,
// so a component can select it without re-rendering on every event.

import { type AgentState, workspaceName } from "../api/models";
import { type StateLabel, agentLabel } from "../domain/labels";
import { agent, project, workspace } from "../store/selectors";
import type { DaemonState } from "../store/store";
import type { Page } from "./ui";

export interface TabInfo {
  title: string;
  /** The tab's tooltip: the title with where it lives. */
  hint: string;
  kind: Page["kind"];
  agentState: AgentState | null;
}

/** An agent's state, for the tab's dot. */
export function tabState(info: TabInfo): StateLabel | null {
  if (info.agentState !== null) return agentLabel(info.agentState);
  return null;
}

export function tabInfo(s: DaemonState, page: Page): TabInfo {
  const base = { kind: page.kind, agentState: null };
  switch (page.kind) {
    case "waiting":
      return { ...base, title: "Waiting on you", hint: "Waiting on you" };
    case "events":
      return { ...base, title: "Event log", hint: "Event log" };
    case "settings":
      return { ...base, title: "Settings", hint: "Settings" };
    case "chief": {
      const name = project(s, page.id)?.name ?? "Removed project";
      return { ...base, title: name, hint: `Chief of Staff · ${name}` };
    }
    case "project": {
      const name = project(s, page.id)?.name ?? "Removed project";
      return { ...base, title: `${name} workspaces`, hint: `All workspaces in ${name}` };
    }
    case "projectEnv": {
      const name = project(s, page.id)?.name ?? "Removed project";
      return { ...base, title: `${name} environment`, hint: `Environment for ${name}'s agents` };
    }
    case "chiefMemory": {
      const name = project(s, page.id)?.name ?? "Removed project";
      return {
        ...base,
        title: `${name} identity & memory`,
        hint: `The identity, brief and memory notes of ${name}'s Chief of Staff`,
      };
    }
    case "workspace": {
      const ws = workspace(s, page.id);
      const name = ws ? workspaceName(ws) : "Removed workspace";
      const owner = ws ? project(s, ws.projectId)?.name : undefined;
      return { ...base, title: name, hint: owner ? `${owner} · ${name}` : name };
    }
    case "shell": {
      const name = page.cwd.split("/").filter(Boolean).pop() ?? "/";
      const owner = page.projectId !== null ? project(s, page.projectId)?.name : undefined;
      return {
        ...base,
        title: `${name} terminal`,
        hint: `Terminal in ${page.cwd}${owner && owner !== name ? ` · ${owner}` : ""}`,
      };
    }
    case "agent": {
      const a = agent(s, page.id);
      if (!a) return { ...base, title: "Removed agent", hint: "Removed agent" };
      const chiefOf = Object.entries(s.staffs).find(([, staff]) => staff.agentId === a.id)?.[0];
      const name = chiefOf !== undefined ? project(s, Number(chiefOf))?.name : undefined;
      const title = name ? `${name} chief's terminal` : a.title;
      const ws = workspace(s, a.workspaceId);
      return {
        ...base,
        title,
        hint: ws ? `${workspaceName(ws)} · ${title}` : title,
        agentState: a.state,
      };
    }
  }
}
