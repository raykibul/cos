// What the window shows and asks: the open tabs and the selected one, the
// open dialog, the ⌘K switcher and toasts. Shared by the sidebar's menus and
// the pages, so both open the same dialogs.
//
// Each project has its own tabs. The tab bar shows the project you're in;
// opening a page of another project moves you there, to the tabs you left
// it with. Pages of no project (Waiting on you, the event log, Settings)
// open among the tabs of the project you're in.

import { createStore } from "zustand/vanilla";
import { messageOf } from "../api/errors";

export type Page =
  | { kind: "waiting" }
  | { kind: "events" }
  | { kind: "project"; id: number }
  /** A project's Chief of Staff; `id` is the project's. */
  | { kind: "chief"; id: number }
  | { kind: "workspace"; id: number }
  | { kind: "agent"; id: number }
  /** A project's environment variables for its agents; `id` is the project's. */
  | { kind: "projectEnv"; id: number }
  /** A project's Chief of Staff's brief and memory notes; `id` is the project's. */
  | { kind: "chiefMemory"; id: number }
  /** Settings: your machines, environment variables for all projects, and remote access to this one. */
  | { kind: "settings" }
  /** A shell the daemon runs in `cwd` (`id` is the shell's), opened with Ctrl-J. */
  | { kind: "shell"; id: number; cwd: string; projectId: number | null; n?: number };

export type DialogState =
  | { kind: "addProject" }
  | { kind: "removeProject"; projectId: number }
  | { kind: "newWorkspace"; projectId: number }
  | { kind: "renameWorkspace"; workspaceId: number }
  | { kind: "removeWorkspace"; workspaceId: number }
  | { kind: "newAgent"; workspaceId: number }
  /** A new session: an agent in a new workspace, or one another session works in. */
  | { kind: "newSession"; projectId: number }
  | { kind: "removeAgent"; agentId: number }
  /**
   * Adds or changes an environment variable, for all projects (`projectId`
   * null) or one. `edit` changes one the scope has; `override` gives a
   * project its own value for a global one. Both fix the name.
   */
  | {
      kind: "envVar";
      projectId: number | null;
      name?: string;
      mode?: "add" | "edit" | "override";
    }
  | { kind: "addHost" }
  | { kind: "renameHost"; hostId: string }
  | { kind: "removeHost"; hostId: string };

export interface Toast {
  id: number;
  title: string;
  message: string;
}

/** The most tabs kept open; opening another drops the oldest one you aren't on. */
export const MAX_TABS = 12;

/** A project's tabs, left to right, and the one selected. */
export interface TabSet {
  tabs: Page[];
  page: Page;
}

export interface UiState {
  /** The selected tab's page. */
  page: Page;
  /** The project's open tabs, left to right. Always holds `page`. */
  tabs: Page[];
  /**
   * The project you're in, whose tabs show and which the sidebar shows: the
   * one the page is in, else the last one you were in. Null until you open
   * one; the sidebar then shows the first, and its tabs are these.
   */
  project: number | null;
  /** The other projects' tabs, as you left them, by project. */
  saved: Record<number, TabSet>;
  /** Whether the ⌘K switcher is open. */
  switcher: boolean;
  /**
   * Counts page opens (a click, ⌘K, a shortcut), so the page puts the cursor
   * in its input each time, even when it was already showing.
   */
  opened: number;
  dialog: DialogState | null;
  toasts: Toast[];
  /** Whether the chief page shows its agent's terminal in a split. */
  chiefTerminal: boolean;
  /**
   * A session the chief started, shown beside its chat (opened from its
   * card); null for none. It and the chief's terminal share the right side.
   */
  chiefSession: number | null;
  /**
   * Whether that session shows its terminal rather than its transcript.
   * Kept as you move between sessions.
   */
  sessionTerminal: boolean;
  /**
   * The session just shown beside the chief, whose message box takes the
   * cursor once it's there; null once it has.
   */
  focusSession: number | null;
  /** Whether an agent's page shows its terminal beside the conversation. */
  agentTerminal: boolean;
  /**
   * Whether the sidebar shows: `auto` hides it in a narrow window, and ⌘B
   * (or the button in the tab bar) picks shown or hidden for good.
   */
  sidebar: SidebarMode;
}

export type SidebarMode = "auto" | "shown" | "hidden";

/** Narrower than this, an `auto` sidebar hides so the page keeps its room. */
export const SIDEBAR_AUTO_MIN_WIDTH = 1040;
export const SIDEBAR_KEY = "cos.sidebar";

/** The sidebar choice saved last; `auto` when there is none or storage refuses. */
export function savedSidebar(): SidebarMode {
  try {
    const value = window.localStorage.getItem(SIDEBAR_KEY);
    return value === "shown" || value === "hidden" ? value : "auto";
  } catch {
    return "auto";
  }
}

function saveSidebar(mode: SidebarMode): void {
  try {
    if (mode === "auto") window.localStorage.removeItem(SIDEBAR_KEY);
    else window.localStorage.setItem(SIDEBAR_KEY, mode);
  } catch {
    // Not kept for next time; the toggle still works.
  }
}

/** Whether the sidebar shows, for its mode and whether the window is narrow. */
export function sidebarShown(mode: SidebarMode, narrow: boolean): boolean {
  return mode === "auto" ? !narrow : mode === "shown";
}

export function samePage(a: Page, b: Page): boolean {
  return a.kind === b.kind && ("id" in a ? a.id : null) === ("id" in b ? b.id : null);
}

let toastId = 0;

/**
 * The tabs after opening `page`: the tab already showing it, else a new one
 * right of `from`. Past MAX_TABS the leftmost tab other than the new one goes.
 */
export function openTab(tabs: Page[], from: Page, page: Page): Page[] {
  if (tabs.some((t) => samePage(t, page))) return tabs;
  const at = tabs.findIndex((t) => samePage(t, from));
  const next = [...tabs];
  next.splice(at < 0 ? next.length : at + 1, 0, page);
  while (next.length > MAX_TABS) {
    const drop = next.findIndex((t) => !samePage(t, page));
    next.splice(drop, 1);
  }
  return next;
}

/** The tabs without `page`, and the tab to select: its right neighbour, else its left one. */
export function closeTab(tabs: Page[], current: Page, page: Page): { tabs: Page[]; page: Page } {
  const at = tabs.findIndex((t) => samePage(t, page));
  if (at < 0) return { tabs, page: current };
  const rest = tabs.filter((_, i) => i !== at);
  // Closing the last tab leaves Waiting on you, the app's home.
  if (rest.length === 0) return { tabs: [home], page: home };
  if (!samePage(current, page)) return { tabs: rest, page: current };
  return { tabs: rest, page: rest[Math.min(at, rest.length - 1)]! };
}

const home: Page = { kind: "waiting" };

/** Every open tab, the other projects' too. */
export function allTabs(s: Pick<UiState, "tabs" | "saved">): Page[] {
  return [...s.tabs, ...Object.values(s.saved).flatMap((set) => set.tabs)];
}

/**
 * The project a page is in, as far as the page itself says: a project's
 * pages and terminals say, agents and workspaces need the daemon's state
 * (`setProjectOf`). Null for a page of no project, or one not known yet.
 */
export function pageProject(page: Page): number | null {
  switch (page.kind) {
    case "chief":
    case "project":
    case "projectEnv":
    case "chiefMemory":
      return page.id;
    case "shell":
      return page.projectId;
    default:
      return null;
  }
}

/**
 * Moves to project `to`'s tabs with `page` selected, opened there if it's
 * new. The tabs you leave are kept for when you come back; with `move`,
 * `page` leaves them (it belongs to `to`), else it stays in both. Tabs of
 * no project (before you were in one) are dropped, unless `to` has none yet:
 * then they become its tabs.
 */
export function enterProject(
  s: Pick<UiState, "project" | "tabs" | "page" | "saved">,
  to: number,
  page: Page,
  move: boolean,
): Pick<UiState, "project" | "tabs" | "page" | "saved"> {
  const mine = s.saved[to];
  const saved = { ...s.saved };
  delete saved[to];
  if (s.project === null && !mine) {
    return { project: to, tabs: openTab(s.tabs, s.page, page), page, saved };
  }
  if (s.project !== null && s.project !== to) {
    saved[s.project] = move ? closeTab(s.tabs, s.page, page) : { tabs: s.tabs, page: s.page };
  }
  const tabs = mine ? openTab(mine.tabs, mine.page, page) : [page];
  return { project: to, tabs, page, saved };
}

export function createUiStore(initial: Partial<UiState> = {}) {
  const page = initial.page ?? home;
  const store = createStore<UiState>()(() => ({
    dialog: null,
    toasts: [],
    chiefTerminal: false,
    chiefSession: null,
    sessionTerminal: false,
    focusSession: null,
    agentTerminal: false,
    sidebar: "auto" as SidebarMode,
    saved: {},
    switcher: false,
    opened: 0,
    ...initial,
    project: initial.project ?? pageProject(page),
    page,
    tabs: initial.tabs && initial.tabs.some((t) => samePage(t, page)) ? initial.tabs : [page],
  }));

  let projectOf: (page: Page) => number | null = pageProject;

  const actions = {
    /**
     * Tells the tabs which project a page is in, with the daemon's state
     * (see `pageProject`).
     */
    setProjectOf(resolve: (page: Page) => number | null) {
      projectOf = resolve;
    },
    /**
     * Shows a page: selects the tab that has it, or opens a new tab. A page
     * of another project moves you to that project's tabs.
     */
    navigate(page: Page) {
      store.setState((s) => {
        const to = projectOf(page);
        if (to === null || to === s.project) {
          return { page, tabs: openTab(s.tabs, s.page, page), opened: s.opened + 1 };
        }
        return { ...enterProject(s, to, page, true), opened: s.opened + 1 };
      });
    },
    /**
     * Goes to a project: back to the tab you left it on, or to its Chief of
     * Staff the first time, or when you're already there.
     */
    openProject(projectId: number) {
      const s = store.getState();
      const back = s.project !== projectId ? s.saved[projectId] : undefined;
      if (!back) {
        actions.navigate({ kind: "chief", id: projectId });
        return;
      }
      store.setState({ ...enterProject(s, projectId, back.page, false), opened: s.opened + 1 });
    },
    /** Closes a tab, in whichever project has it; the selected one when none is given. */
    closeTab(page?: Page) {
      store.setState((s) => {
        const target = page ?? s.page;
        if (s.tabs.some((t) => samePage(t, target))) return closeTab(s.tabs, s.page, target);
        for (const [id, set] of Object.entries(s.saved)) {
          if (!set.tabs.some((t) => samePage(t, target))) continue;
          return { saved: { ...s.saved, [id]: closeTab(set.tabs, set.page, target) } };
        }
        return {};
      });
    },
    /**
     * Closes a removed project's tabs. If you were in it, the tabs left
     * become those of the next project you're in.
     */
    forgetProject(projectId: number) {
      store.setState((s) => {
        const saved = { ...s.saved };
        delete saved[projectId];
        let set: TabSet = { tabs: s.tabs, page: s.page };
        for (const tab of s.tabs) {
          if (projectOf(tab) === projectId) set = closeTab(set.tabs, set.page, tab);
        }
        return { ...set, saved, project: s.project === projectId ? null : s.project };
      });
    },
    /** Selects the tab `step` places right (or left, when negative), wrapping around. */
    cycleTab(step: number) {
      store.setState((s) => {
        const at = s.tabs.findIndex((t) => samePage(t, s.page));
        const next = s.tabs[(at + step + s.tabs.length * 2) % s.tabs.length];
        return next ? { page: next, opened: s.opened + 1 } : {};
      });
    },
    /**
     * Moves to the project the sidebar shows, when the page alone didn't say
     * (an agent's, before its project was known; the first project, at
     * start). The page goes along: into that project's tabs if it's one of
     * its pages, else (a page of no project) it shows there too.
     */
    showProject(projectId: number) {
      const s = store.getState();
      if (s.project === projectId) return;
      store.setState(enterProject(s, projectId, s.page, projectOf(s.page) === projectId));
    },
    openSwitcher(open = true) {
      store.setState({ switcher: open });
    },
    open(dialog: DialogState) {
      store.setState({ dialog });
    },
    close() {
      store.setState({ dialog: null });
    },
    toggleChiefTerminal(open?: boolean) {
      store.setState((s) => {
        const chiefTerminal = open ?? !s.chiefTerminal;
        return { chiefTerminal, chiefSession: chiefTerminal ? null : s.chiefSession };
      });
    },
    /** Shows a session the chief started beside its chat; null closes it. */
    showChiefSession(agentId: number | null) {
      store.setState((s) => ({
        chiefSession: agentId,
        focusSession: agentId,
        chiefTerminal: agentId !== null ? false : s.chiefTerminal,
      }));
    },
    /** The session beside the chief has taken the cursor. */
    sessionFocused() {
      if (store.getState().focusSession !== null) store.setState({ focusSession: null });
    },
    /** Shows the session beside the chief as its terminal, or as its transcript. */
    showSessionTerminal(open?: boolean) {
      store.setState((s) => ({ sessionTerminal: open ?? !s.sessionTerminal }));
    },
    toggleAgentTerminal(open?: boolean) {
      store.setState((s) => ({ agentTerminal: open ?? !s.agentTerminal }));
    },
    /** Shows the sidebar if it's hidden (given how narrow the window is), else hides it; the choice is kept. */
    toggleSidebar(narrow: boolean) {
      const mode: SidebarMode = sidebarShown(store.getState().sidebar, narrow) ? "hidden" : "shown";
      saveSidebar(mode);
      store.setState({ sidebar: mode });
    },
    toast(title: string, message: string) {
      const id = ++toastId;
      store.setState((s) => ({ toasts: [...s.toasts.slice(-3), { id, title, message }] }));
      setTimeout(() => actions.dismiss(id), 7000);
    },
    dismiss(id: number) {
      store.setState((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    },
    /** Runs a daemon action and shows its error, which the daemon writes to be shown as-is. */
    perform(failureTitle: string, action: () => Promise<unknown>): Promise<void> {
      return action().then(
        () => undefined,
        (error: unknown) => actions.toast(failureTitle, messageOf(error)),
      );
    },
  };

  return { store, ...actions };
}

export type UiStore = ReturnType<typeof createUiStore>;
