import { X } from "lucide-react";
import { useMemo } from "react";
import { type CosStore, isThisMac } from "../store/store";
import { AgentPage } from "../screens/AgentPage";
import { EventLogPage } from "../screens/EventLogPage";
import { HostRefused, HostUnreachable } from "../screens/HostUnavailable";
import { NotRunning } from "../screens/NotRunning";
import { ProjectPage } from "../screens/ProjectPage";
import { RunPage } from "../screens/RunPage";
import { SettingsPage } from "../screens/SettingsPage";
import { Sidebar } from "../screens/Sidebar";
import { WaitingPage } from "../screens/WaitingPage";
import { WorkspacePage } from "../screens/WorkspacePage";
import { Dialogs } from "../sheets/Dialogs";
import { AppContext, useApp, useCos, useUi } from "./context";
import { type UiStore, createUiStore } from "./ui";

export function App({ cos, ui }: { cos: CosStore; ui?: UiStore }) {
  const value = useMemo(() => ({ cos, ui: ui ?? createUiStore() }), [cos, ui]);
  return (
    <AppContext.Provider value={value}>
      <Shell />
    </AppContext.Provider>
  );
}

function Shell() {
  return (
    // The window has no title bar. The sidebar's top (under the traffic
    // lights) and each page's header bar are drag regions, so the top 28 px
    // drag the window everywhere except over buttons.
    <div className="flex h-full bg-page text-ink">
      <Sidebar />
      <main className="relative flex min-w-0 flex-1 flex-col border-l border-line bg-page">
        <Content />
      </main>
      <Dialogs />
      <Toasts />
    </div>
  );
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
    case "workspace":
      return <WorkspacePage key={page.id} workspaceId={page.id} />;
    case "agent":
      return <AgentPage key={page.id} agentId={page.id} />;
    case "run":
      return <RunPage key={page.id} runId={page.id} />;
  }
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
