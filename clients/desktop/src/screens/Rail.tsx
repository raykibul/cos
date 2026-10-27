// The project rail on the window's left edge: Waiting on you, then one tile
// per project. A tile goes to that project, back to the tab you left it on
// (its Chief of Staff the first time, or when you're already there; ⌘1 to
// ⌘9 do the same), shows what waits on you there, and picks the project the
// sidebar and the tabs show. Add project and Settings sit at the ends.

import { Inbox, Plus, Settings } from "lucide-react";
import type { ReactNode } from "react";
import { envHint } from "../api/env";
import type { Project } from "../api/models";
import { useApp, useCanChange, useCos, useUi } from "../app/context";
import { samePage } from "../app/ui";
import { projectInitial, projectShortcut } from "../domain/rail";
import {
  projectActivity,
  projectEnvNames,
  projectWaitingCount,
  waitingCount,
} from "../store/selectors";
import { cx } from "../ui/cx";

/** A tile's colours, by the project's place in the list. */
const tints = [
  { tile: "bg-accent/16 text-accent-ink", ring: "ring-accent" },
  { tile: "bg-tone-blue/16 text-tone-blue", ring: "ring-tone-blue" },
  { tile: "bg-tone-green/16 text-tone-green", ring: "ring-tone-green" },
  { tile: "bg-tone-purple/16 text-tone-purple", ring: "ring-tone-purple" },
  { tile: "bg-tone-teal/16 text-tone-teal", ring: "ring-tone-teal" },
  { tile: "bg-tone-amber/18 text-tone-amber", ring: "ring-tone-amber" },
] as const;

export function Rail({ shown }: { shown: number | null }) {
  const { ui } = useApp();
  const projects = useCos((s) => s.projects);
  const waiting = useCos(waitingCount);
  const onWaiting = useUi((s) => s.page.kind === "waiting");
  const onSettings = useUi((s) => s.page.kind === "settings");
  const canChange = useCanChange();

  return (
    <nav
      aria-label="Projects"
      className="flex w-[72px] shrink-0 flex-col items-center border-r border-line bg-rail pb-3 select-none"
    >
      {/* Under the traffic lights; drags the window. */}
      <div data-tauri-drag-region className="h-[46px] w-full shrink-0" />
      <RailButton
        label={waiting > 0 ? `Waiting on you, ${waiting}` : "Waiting on you"}
        title="Waiting on you"
        selected={onWaiting}
        onClick={() => ui.navigate({ kind: "waiting" })}
      >
        <Inbox className="size-[18px]" strokeWidth={1.8} aria-hidden />
        {waiting > 0 ? <Badge count={waiting} /> : null}
      </RailButton>
      <div aria-hidden className="my-2.5 h-px w-6 bg-line-strong" />

      <div className="flex min-h-0 w-full flex-1 flex-col items-center gap-0.5 overflow-y-auto [scrollbar-width:none]">
        {projects.map((project, index) => (
          <ProjectTile
            key={project.id}
            project={project}
            index={index}
            shown={project.id === shown}
          />
        ))}
        <div className="mt-1.5">
          <button
            type="button"
            aria-label="Add project"
            title="Add project"
            disabled={!canChange}
            onClick={() => ui.open({ kind: "addProject" })}
            className="flex size-10 items-center justify-center rounded-xl border border-dashed border-line-strong text-ink-3 transition-colors hover:border-ink-3/50 hover:text-ink disabled:opacity-40"
          >
            <Plus className="size-4" strokeWidth={1.9} aria-hidden />
          </button>
        </div>
      </div>

      <RailButton
        label="Settings"
        title="Settings"
        selected={onSettings}
        onClick={() => ui.navigate({ kind: "settings" })}
      >
        <Settings className="size-[18px]" strokeWidth={1.8} aria-hidden />
      </RailButton>
    </nav>
  );
}

function ProjectTile({
  project,
  index,
  shown,
}: {
  project: Project;
  index: number;
  shown: boolean;
}) {
  const { ui } = useApp();
  const onChief = useUi((s) => samePage(s.page, { kind: "chief", id: project.id }));
  const waiting = useCos((s) => projectWaitingCount(s, project));
  const working = useCos((s) => projectActivity(s, project) === "working");
  const env = envHint(useCos((s) => projectEnvNames(s, project.id)));
  const tint = tints[index % tints.length]!;
  const shortcut = projectShortcut(index);
  const label = `Chief of Staff, ${project.name}${waiting > 0 ? `, ${waiting} waiting` : ""}`;
  return (
    <div className="relative flex w-full justify-center">
      {onChief ? (
        <span aria-hidden className="absolute top-[9px] left-0 h-[26px] w-[3px] rounded-r bg-ink" />
      ) : null}
      <button
        type="button"
        aria-label={label}
        aria-current={onChief ? "page" : undefined}
        title={`Chief of Staff for ${project.name}${shortcut ? ` (${shortcut})` : ""}\n${project.root}${env ? `\n${env}` : ""}`}
        onClick={() => ui.openProject(project.id)}
        className="flex w-full flex-col items-center gap-1 py-1 outline-none [&:focus-visible>span:first-child]:ring-2"
      >
        <span
          className={cx(
            "relative grid size-10 place-items-center rounded-xl font-serif text-[19px] leading-none font-semibold transition-shadow",
            tint.tile,
            shown && cx("ring-2 ring-offset-2 ring-offset-rail", tint.ring),
          )}
        >
          {projectInitial(project.name)}
          {waiting > 0 ? (
            <Badge count={waiting} />
          ) : working ? (
            <span className="absolute -right-1 -bottom-1 size-3 animate-breathe rounded-full border-2 border-rail bg-tone-blue" />
          ) : null}
        </span>
        <span className="h-3 font-mono text-[10px] leading-3 text-ink-3">{shortcut}</span>
      </button>
    </div>
  );
}

function RailButton({
  label,
  title,
  selected,
  onClick,
  children,
}: {
  label: string;
  title: string;
  selected: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title}
      aria-current={selected ? "page" : undefined}
      onClick={onClick}
      className={cx(
        "relative flex size-10 shrink-0 items-center justify-center rounded-xl transition-colors",
        selected ? "bg-subtle-2 text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
      )}
    >
      {children}
    </button>
  );
}

function Badge({ count }: { count: number }) {
  return (
    <span
      aria-hidden
      className="absolute -top-1.5 -right-2 min-w-[18px] rounded-full border-2 border-rail bg-accent px-1 font-sans text-[10.5px] leading-[14px] font-semibold text-white tabular-nums"
    >
      {count}
    </span>
  );
}
