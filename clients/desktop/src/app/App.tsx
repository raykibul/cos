import { X } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { shellPlace, shownProject } from "../store/selectors";
import { type CosStore, isThisMac } from "../store/store";
import { ChiefPage } from "../pages/chief/ChiefPage";
import { AgentPage } from "../screens/AgentPage";
import { EventLogPage } from "../screens/EventLogPage";
import { HostRefused, HostUnreachable } from "../screens/HostUnavailable";
import { NotRunning } from "../screens/NotRunning";
import { ProjectEnvPage } from "../screens/ProjectEnvPage";
import { ProjectPage } from "../screens/ProjectPage";
import { QuickSwitch } from "../screens/QuickSwitch";
import { Rail } from "../screens/Rail";
import { SettingsPage } from "../screens/SettingsPage";
import { ShellPage } from "../screens/ShellPage";
import { Sidebar } from "../screens/Sidebar";
import { StatusBar } from "../screens/StatusBar";
import { TabBar } from "../screens/TabBar";
import { WaitingPage } from "../screens/WaitingPage";
import { WorkspacePage } from "../screens/WorkspacePage";
import { Dialogs } from "../sheets/Dialogs";
import { cx } from "../ui/cx";
import { AppContext, useApp, useCos, useUi } from "./context";
import { useSidebar } from "./layout";
import { type Page, type UiStore, createUiStore } from "./ui";

export function App({ cos, ui }: { cos: CosStore; ui?: UiStore }) {
  const value = useMemo(() => ({ cos, ui: ui ?? createUiStore() }), [cos, ui]);
  return (
    <AppContext.Provider value={value}>
      <Shell />
    </AppContext.Provider>
  );
}

function Shell() {
  const { ui } = useApp();
  const page = useUi((s) => s.page);
  const remembered = useUi((s) => s.project);
  const switcher = useUi((s) => s.switcher);
  const project = useCos((s) => shownProject(s, page, remembered));
  const projectId = project?.id ?? null;

  // The sidebar keeps showing the last project you were in.
  useEffect(() => {
    if (projectId !== null) ui.showProject(projectId);
  }, [ui, projectId]);

  const sidebar = useSidebar();
  useShortcuts(sidebar.narrow);
  useShellsEndWithTheirTabs();

  return (
    // The window has no title bar. The rail's top (under the traffic
    // lights), the sidebar's top, the tab bar and each page's header bar are
    // drag regions, so the window drags from the top everywhere except over
    // buttons and tabs.
    <div className="flex h-full bg-page text-ink">
      <Rail shown={projectId} />
      {sidebar.shown ? <Sidebar project={project} /> : null}
      <main
        className={cx(
          "relative flex min-w-0 flex-1 flex-col bg-page",
          sidebar.shown && "border-l border-line",
        )}
      >
        <TabBar />
        <div className="flex min-h-0 flex-1 flex-col">
          <Content />
        </div>
        <StatusBar />
      </main>
      <Dialogs />
      {switcher ? <QuickSwitch /> : null}
      <Toasts />
    </div>
  );
}

/**
 * The window's shortcuts: ⌘K the switcher, ⌘1 to ⌘9 each project's chief,
 * ⌘W closes the tab, ⌃Tab or ⌘⇧] and ⌘⇧[ move between tabs, ⌘B shows or
 * hides the sidebar, and ⌃J opens a terminal tab where the page is (see
 * `shellPlace`). They're caught before the page (a terminal, say) sees them,
 * and wait while a dialog is open.
 */
function useShortcuts(narrow: boolean) {
  const { cos, ui } = useApp();
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const { dialog, switcher } = ui.store.getState();
      const command = event.metaKey && !event.ctrlKey && !event.altKey;
      const key = event.key.toLowerCase();
      let handled = true;
      if (command && !event.shiftKey && key === "k" && !dialog) {
        ui.openSwitcher(!switcher);
      } else if (dialog || switcher) {
        handled = false;
      } else if (command && !event.shiftKey && /^digit[1-9]$/.test(event.code.toLowerCase())) {
        const project = cos.get().projects[Number(event.code.slice(5)) - 1];
        if (project) ui.navigate({ kind: "chief", id: project.id });
      } else if (command && !event.shiftKey && key === "w") {
        ui.closeTab();
      } else if (command && !event.shiftKey && event.code === "KeyB") {
        ui.toggleSidebar(narrow);
      } else if (
        event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !event.shiftKey &&
        event.code === "KeyJ"
      ) {
        openShell(cos, ui);
      } else if (event.ctrlKey && !event.metaKey && key === "tab") {
        ui.cycleTab(event.shiftKey ? -1 : 1);
      } else if (command && event.shiftKey && (event.code === "BracketRight" || key === "}")) {
        ui.cycleTab(1);
      } else if (command && event.shiftKey && (event.code === "BracketLeft" || key === "{")) {
        ui.cycleTab(-1);
      } else {
        handled = false;
      }
      if (handled) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [cos, ui, narrow]);
}

function Content() {
  const page = useUi((s) => s.page);
  const connection = useCos((s) => s.connection);
  const remote = useCos((s) => !isThisMac(s.host));
  // Settings works without a daemon: it's where hosts are managed.
  if (page.kind === "settings") return <SettingsPage />;
  if (connection === "not_running") return remote ? <HostUnreachable /> : <NotRunning />;
  if (connection === "refused") return <HostRefused />;
  switch (page.kind) {
    case "waiting":
      return <WaitingPage />;
    case "events":
      return <EventLogPage />;
    case "project":
      return <ProjectPage key={page.id} projectId={page.id} />;
    case "chief":
      return <ChiefPage key={page.id} projectId={page.id} />;
    case "projectEnv":
      return <ProjectEnvPage key={page.id} projectId={page.id} />;
    case "workspace":
      return <WorkspacePage key={page.id} workspaceId={page.id} />;
    case "agent":
      return <AgentPage key={page.id} agentId={page.id} />;
    case "shell":
      return <ShellPage key={page.id} page={page} />;
  }
}

/** Starts a shell where the page is and opens it in a tab next to it. */
function openShell(cos: CosStore, ui: UiStore): void {
  const { page, project } = ui.store.getState();
  const place = shellPlace(cos.get(), page, project);
  if (!place) {
    ui.toast("Nowhere to open a terminal", "Add a project first; Ctrl-J opens one in its folder.");
    return;
  }
  void ui.perform("Couldn't open a terminal", async () => {
    const shell = await cos.client.startShell(place.cwd, place.projectId);
    ui.navigate({ kind: "shell", id: shell.id, cwd: shell.cwd, projectId: shell.projectId });
  });
}

/** A closed terminal tab ends its shell, so none is left running unseen. */
function useShellsEndWithTheirTabs() {
  const { cos } = useApp();
  const tabs = useUi((s) => s.tabs);
  const open = useRef<Page[]>(tabs);
  useEffect(() => {
    const before = open.current;
    open.current = tabs;
    for (const tab of before) {
      if (tab.kind !== "shell") continue;
      if (tabs.some((t) => t.kind === "shell" && t.id === tab.id)) continue;
      // Already gone (it exited, or the daemon restarted): nothing to end.
      void cos.client.closeShell(tab.id).catch(() => undefined);
    }
  }, [cos, tabs]);
}

function Toasts() {
  const { ui } = useApp();
  const toasts = useUi((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-[70] flex w-[360px] flex-col gap-2">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="status"
          className="pointer-events-auto flex gap-3 rounded-xl border border-line bg-raised px-4 py-3 shadow-pop animate-pop-in"
        >
          <div className="mt-1.5 size-2 shrink-0 rounded-full bg-tone-red" />
          <div className="min-w-0 flex-1">
            <p className="text-[13px] font-medium text-ink">{toast.title}</p>
            <p className="mt-0.5 text-[12.5px] leading-snug text-ink-2 select-text">
              {toast.message}
            </p>
          </div>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={() => ui.dismiss(toast.id)}
            className="flex size-6 shrink-0 items-center justify-center rounded-md text-ink-3 hover:bg-subtle hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
