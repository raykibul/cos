// What the window shows and asks: the selected page, the open dialog and
// toasts. Shared by the sidebar's menus and the pages, so both open the same
// dialogs.

import { createStore } from "zustand/vanilla";
import { messageOf } from "../api/errors";
import type { TaskActionKind } from "../domain/tasks";

export type Page =
  | { kind: "waiting" }
  | { kind: "events" }
  | { kind: "project"; id: number }
  /** A project's Chief of Staff; `id` is the project's. */
  | { kind: "chief"; id: number }
  | { kind: "workspace"; id: number }
  | { kind: "agent"; id: number }
  | { kind: "run"; id: number }
  /** A project's environment variables for its agents; `id` is the project's. */
  | { kind: "projectEnv"; id: number }
  /** Settings: your machines, environment variables for all projects, and remote access to this one. */
  | { kind: "settings" };

export type DialogState =
  | { kind: "addProject" }
  | { kind: "removeProject"; projectId: number }
  | { kind: "newWorkspace"; projectId: number }
  | { kind: "renameWorkspace"; workspaceId: number }
  | { kind: "removeWorkspace"; workspaceId: number }
  | { kind: "newAgent"; workspaceId: number }
  | { kind: "removeAgent"; agentId: number }
  | { kind: "newRun"; workspaceId: number }
  | { kind: "cancelRun"; runId: number }
  | { kind: "taskAction"; runId: number; taskId: number; action: TaskActionKind }
  | { kind: "limits"; runId: number }
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

export interface UiState {
  page: Page;
  dialog: DialogState | null;
  toasts: Toast[];
  /**
   * Sidebar projects unfolded (true) or folded (false) by hand, by project
   * id. Unset, a project is unfolded while the page is inside it.
   */
  expanded: Record<number, boolean>;
  /** Whether the chief page shows its agent's terminal in a split. */
  chiefTerminal: boolean;
}

export function samePage(a: Page, b: Page): boolean {
  return a.kind === b.kind && ("id" in a ? a.id : null) === ("id" in b ? b.id : null);
}

let toastId = 0;

export function createUiStore(initial: Partial<UiState> = {}) {
  const store = createStore<UiState>()(() => ({
    page: { kind: "waiting" },
    dialog: null,
    toasts: [],
    expanded: {},
    chiefTerminal: false,
    ...initial,
  }));

  const actions = {
    navigate(page: Page) {
      store.setState({ page });
    },
    open(dialog: DialogState) {
      store.setState({ dialog });
    },
    close() {
      store.setState({ dialog: null });
    },
    /** Unfolds or folds a project in the sidebar. */
    setExpanded(projectId: number, expanded: boolean) {
      store.setState((s) => ({ expanded: { ...s.expanded, [projectId]: expanded } }));
    },
    toggleChiefTerminal(open?: boolean) {
      store.setState((s) => ({ chiefTerminal: open ?? !s.chiefTerminal }));
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
