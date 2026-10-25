// What the window shows and asks: the open tabs and the selected one, the
// open dialog, the ⌘K switcher and toasts. Shared by the sidebar's menus and
// the pages, so both open the same dialogs.

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
  /** Settings: your machines, environment variables for all projects, and remote access to this one. */
  | { kind: "settings" }
  /** A shell the daemon runs in `cwd` (`id` is the shell's), opened with Ctrl-J. */
  | { kind: "shell"; id: number; cwd: string; projectId: number | null };

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

export interface UiState {
  /** The selected tab's page. */
  page: Page;
  /** The open tabs, left to right. Always holds `page`. */
  tabs: Page[];
  /**
   * The project the sidebar shows: the one the page is in, else the last one
   * you were in. Null until you open one; the sidebar then shows the first.
   */
  project: number | null;
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

export function createUiStore(initial: Partial<UiState> = {}) {
  const page = initial.page ?? home;
  const store = createStore<UiState>()(() => ({
    dialog: null,
    toasts: [],
    chiefTerminal: false,
    agentTerminal: false,
    sidebar: "auto" as SidebarMode,
    project: null,
    switcher: false,
    opened: 0,
    ...initial,
    page,
    tabs: initial.tabs && initial.tabs.some((t) => samePage(t, page)) ? initial.tabs : [page],
  }));

  const actions = {
    /** Shows a page: selects the tab that has it, or opens a new tab. */
    navigate(page: Page) {
      store.setState((s) => ({
        page,
        tabs: openTab(s.tabs, s.page, page),
        opened: s.opened + 1,
      }));
    },
    /** Closes a tab; the selected one when none is given. */
    closeTab(page?: Page) {
      store.setState((s) => closeTab(s.tabs, s.page, page ?? s.page));
    },
    /** Selects the tab `step` places right (or left, when negative), wrapping around. */
    cycleTab(step: number) {
      store.setState((s) => {
        const at = s.tabs.findIndex((t) => samePage(t, s.page));
        const next = s.tabs[(at + step + s.tabs.length * 2) % s.tabs.length];
        return next ? { page: next, opened: s.opened + 1 } : {};
      });
    },
    /** Remembers the project the sidebar shows. */
    showProject(projectId: number) {
      if (store.getState().project !== projectId) store.setState({ project: projectId });
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
      store.setState((s) => ({ chiefTerminal: open ?? !s.chiefTerminal }));
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
