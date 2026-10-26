// Ctrl-J terminals: each opens a new shell in a tab, in the project's folder
// (or the session's or workspace's folder on those pages). Closing its tab
// ends the shell (App.tsx).

import { shellPlace } from "../store/selectors";
import type { CosStore } from "../store/store";
import type { Page, UiStore } from "./ui";

/**
 * Starts a shell where the page is, or in `place`, and opens it in a tab
 * next to the page. Terminals are numbered per project: one more than the
 * highest open.
 */
export function openShell(
  cos: CosStore,
  ui: UiStore,
  place?: { cwd: string; projectId: number | null },
): void {
  const { page, project } = ui.store.getState();
  const where = place ?? shellPlace(cos.get(), page, project);
  if (!where) {
    ui.toast("Nowhere to open a terminal", "Add a project first; Ctrl-J opens one in its folder.");
    return;
  }
  void ui.perform("Couldn't open a terminal", async () => {
    const shell = await cos.client.startShell(where.cwd, where.projectId);
    const n = nextShellNumber(ui.store.getState().tabs, shell.projectId);
    ui.navigate({ kind: "shell", id: shell.id, cwd: shell.cwd, projectId: shell.projectId, n });
  });
}

/** One more than the highest number of the project's open terminal tabs. */
export function nextShellNumber(tabs: Page[], projectId: number | null): number {
  let n = 0;
  for (const tab of tabs) {
    if (tab.kind === "shell" && tab.projectId === projectId) n = Math.max(n, tab.n ?? 1);
  }
  return n + 1;
}

/** A project's open terminal tabs, in tab order. */
export function projectShells(tabs: Page[], projectId: number): Extract<Page, { kind: "shell" }>[] {
  return tabs.filter(
    (tab): tab is Extract<Page, { kind: "shell" }> =>
      tab.kind === "shell" && tab.projectId === projectId,
  );
}
