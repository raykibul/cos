import { Inbox, Plus, ScrollText, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useApp, useCanChange, useCos, useNow, useUi } from "../app/context";
import { waitingCount } from "../store/selectors";
import { cx } from "../ui/cx";
import { duration } from "../ui/format";
import { ProjectSection } from "./sidebar/ProjectSection";

export function Sidebar() {
  const { ui } = useApp();
  const projects = useCos((s) => s.projects);
  const connection = useCos((s) => s.connection);
  const projectsLoaded = useCos((s) => s.projectsLoaded);
  const waiting = useCos(waitingCount);
  const page = useUi((s) => s.page);
  const canChange = useCanChange();

  return (
    <aside className="flex w-[264px] shrink-0 flex-col bg-sidebar select-none" aria-label="Sidebar">
      {/* Under the traffic lights; drags the window. The host picker goes here. */}
      <div data-tauri-drag-region className="h-[52px] shrink-0" />
      <nav className="flex flex-col gap-px px-2.5">
        <NavRow
          icon={Inbox}
          label="Waiting on you"
          selected={page.kind === "waiting"}
          onClick={() => ui.navigate({ kind: "waiting" })}
          trailing={
            waiting > 0 ? (
              <span
                aria-label={`${waiting} waiting`}
                className="ml-auto min-w-5 rounded-full bg-accent px-1.5 text-center text-[11px] leading-[18px] font-semibold text-white"
              >
                {waiting}
              </span>
            ) : null
          }
        />
        <NavRow
          icon={ScrollText}
          label="Event log"
          selected={page.kind === "events"}
          onClick={() => ui.navigate({ kind: "events" })}
        />
      </nav>

      <div className="mt-5 flex items-center px-4 pb-1">
        <span className="text-[11.5px] font-semibold tracking-wide text-ink-3 uppercase">
          Projects
        </span>
        <button
          type="button"
          aria-label="Add project"
          title="Add project"
          disabled={!canChange}
          onClick={() => ui.open({ kind: "addProject" })}
          className="ml-auto flex size-6 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2/70 hover:text-ink disabled:opacity-40"
        >
          <Plus className="size-3.5" strokeWidth={2} />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-4">
        {projects.length === 0 ? (
          <p className="px-2 py-2 text-[12.5px] leading-snug text-ink-3">
            {connection === "running" && projectsLoaded
              ? "No projects yet. Add a git repo to get started."
              : connection === "running"
                ? "Loading…"
                : "No projects to show."}
          </p>
        ) : (
          <ul className="flex flex-col" role="tree" aria-label="Projects">
            {projects.map((project) => (
              <ProjectSection key={project.id} project={project} />
            ))}
          </ul>
        )}
      </div>

      <SidebarFooter />
    </aside>
  );
}

function NavRow({
  icon: Icon,
  label,
  selected,
  onClick,
  trailing,
}: {
  icon: LucideIcon;
  label: string;
  selected: boolean;
  onClick: () => void;
  trailing?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected ? "page" : undefined}
      className={cx(
        "flex h-8 items-center gap-2.5 rounded-lg px-2 text-left text-[13.5px] transition-colors",
        selected ? "bg-subtle-2 font-medium text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
      )}
    >
      <Icon className="size-4 shrink-0" strokeWidth={1.8} aria-hidden />
      <span className="truncate">{label}</span>
      {trailing}
    </button>
  );
}

function SidebarFooter() {
  const { ui } = useApp();
  const connection = useCos((s) => s.connection);
  const status = useCos((s) => s.status);
  const canChange = useCanChange();
  const now = useNow(60_000);
  const title =
    connection === "running"
      ? "cos is running"
      : connection === "not_running"
        ? "cos is not running"
        : connection === "reconnecting"
          ? "Reconnecting…"
          : "Connecting…";
  const dot =
    connection === "running"
      ? "bg-tone-green"
      : connection === "not_running"
        ? "bg-tone-red"
        : "bg-ink-3 animate-breathe";
  return (
    <div className="shrink-0 border-t border-line px-2.5 py-2.5">
      <button
        type="button"
        disabled={!canChange}
        onClick={() => ui.open({ kind: "addProject" })}
        className="flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] text-ink-2 hover:bg-subtle hover:text-ink disabled:opacity-40"
      >
        <Plus className="size-4" strokeWidth={1.8} />
        Add project
      </button>
      <button
        type="button"
        onClick={() => ui.navigate({ kind: "events" })}
        className="mt-0.5 flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-subtle"
        title={status ? `pid ${status.pid} · ${status.home}` : undefined}
      >
        <span className={cx("ml-1 size-2 shrink-0 rounded-full", dot)} aria-hidden />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[12.5px] font-medium text-ink-2">{title}</span>
          {status ? (
            <span className="block truncate text-[11.5px] text-ink-3">
              v{status.version} · up {duration(status.startedAt, now)}
            </span>
          ) : null}
        </span>
      </button>
    </div>
  );
}
