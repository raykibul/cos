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
  | { kind: "workspace"; id: number }
  | { kind: "agent"; id: number }
  | { kind: "run"; id: number }
  /** Settings: your Macs, and remote access to this one. */
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
  | { kind: "addHost" }
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
  /** Sidebar rows folded away, by key (`p1`, `w3`). */
  collapsed: Record<string, boolean>;
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
    collapsed: {},
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
    toggle(key: string) {
      store.setState((s) => ({ collapsed: { ...s.collapsed, [key]: !s.collapsed[key] } }));
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
