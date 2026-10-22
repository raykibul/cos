// A project in the sidebar, as in Selene: one `★ Chief of Staff · project`
// row that opens the chief and unfolds the project's workspaces under it,
// newest first. Agents and runs are on the workspace page, which keeps the
// sidebar short (docs/chief-of-staff.md, "App").

import { ChevronRight, GitBranch, LayoutGrid, MoreHorizontal, Plus } from "lucide-react";
import { type Project, type Workspace, workspaceName } from "../../api/models";
import { useApp, useCanChange, useCos, useUi } from "../../app/context";
import { samePage } from "../../app/ui";
import {
  type Activity,
  activityOf,
  currentRun,
  projectActivity,
  projectOfPage,
  workspaceAgents,
  workspaceOfPage,
  workspacesNewestFirst,
} from "../../store/selectors";
import { cx } from "../../ui/cx";
import { type MenuItem, Menu } from "../../ui/Menu";
import { folderItems } from "../common";

export function ProjectSection({ project }: { project: Project }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const page = useUi((s) => s.page);
  const explicit = useUi((s) => s.expanded[project.id]);
  const inside = useCos((s) => projectOfPage(s, page) === project.id);
  const activity = useCos((s) => projectActivity(s, project));
  const open = explicit ?? inside;
  const selected = samePage(page, { kind: "chief", id: project.id });
  const workspaces = workspacesNewestFirst(project);

  const select = () => {
    if (selected) ui.setExpanded(project.id, !open);
    else {
      ui.navigate({ kind: "chief", id: project.id });
      ui.setExpanded(project.id, true);
    }
  };

  const menu: MenuItem[] = [
    {
      label: "New workspace…",
      icon: Plus,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "newWorkspace", projectId: project.id }),
    },
    {
      label: "All workspaces",
      icon: LayoutGrid,
      onSelect: () => ui.navigate({ kind: "project", id: project.id }),
    },
    "separator",
    ...folderItems(cos, project.root),
    "separator",
    {
      label: "Remove project…",
      danger: true,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "removeProject", projectId: project.id }),
    },
  ];

  return (
    <li role="none" className="mb-0.5">
      <div
        role="treeitem"
        aria-selected={selected}
        aria-expanded={open}
        aria-label={`Chief of Staff, ${project.name}`}
        className={cx(
          "group relative flex h-9 items-center rounded-lg pr-1 transition-colors",
          selected ? "bg-tone-amber/12" : "hover:bg-subtle",
        )}
      >
        <button
          type="button"
          onClick={select}
          title={`Chief of Staff for ${project.name}\n${project.root}`}
          className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-lg pl-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <ChiefMark />
          <span className="flex min-w-0 flex-1 items-baseline gap-1.5 text-[13px] whitespace-nowrap">
            <span className={cx("shrink-0", selected ? "text-tone-amber" : "text-ink-3")}>
              Chief of Staff
            </span>
            <span className="min-w-0 truncate font-medium text-ink">{project.name}</span>
          </span>
          <ActivityDot activity={activity} />
        </button>
        <Menu
          items={menu}
          align="start"
          trigger={(props) => (
            <button
              type="button"
              aria-label={`More for ${project.name}`}
              {...props}
              className={cx(
                // No width until hovered or focused, so the name keeps the room.
                "flex h-6 w-0 shrink-0 items-center justify-center overflow-hidden rounded-md text-ink-3 hover:bg-subtle-2 hover:text-ink",
                "opacity-0 group-hover:w-6 group-hover:opacity-100 focus-visible:w-6 focus-visible:opacity-100 aria-expanded:w-6 aria-expanded:opacity-100",
              )}
            >
              <MoreHorizontal className="size-4 shrink-0" strokeWidth={1.8} />
            </button>
          )}
        />
        <button
          type="button"
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} workspaces in ${project.name}`}
          title={open ? "Hide workspaces" : "Show workspaces"}
          onClick={() => ui.setExpanded(project.id, !open)}
          className={cx(
            "flex h-6 shrink-0 items-center gap-0.5 rounded-md pr-1 pl-1.5 text-[11px] text-ink-3 tabular-nums hover:bg-subtle-2 hover:text-ink",
            open && "text-ink-2",
          )}
        >
          {workspaces.length}
          <ChevronRight
            className={cx("size-3 transition-transform duration-150", open && "rotate-90")}
            strokeWidth={2.2}
          />
        </button>
      </div>
      {open ? (
        <ul role="group" className="mt-0.5 mb-2 flex flex-col gap-px">
          {workspaces.map((workspace) => (
            <WorkspaceRow key={workspace.id} workspace={workspace} />
          ))}
          {workspaces.length === 0 ? (
            <li role="none">
              <button
                type="button"
                disabled={!canChange}
                onClick={() => ui.open({ kind: "newWorkspace", projectId: project.id })}
                className="flex h-8 w-full items-center gap-2 rounded-lg pl-9 text-left text-[12.5px] text-ink-3 hover:bg-subtle hover:text-ink disabled:opacity-40"
              >
                <Plus className="size-3.5" strokeWidth={1.9} />
                New workspace
              </button>
            </li>
          ) : null}
        </ul>
      ) : null}
    </li>
  );
}

/** The chief's amber star, in a soft square. */
export function ChiefMark({ size = "sm" }: { size?: "sm" | "md" | "lg" }) {
  return (
    <span
      aria-hidden
      className={cx(
        "grid shrink-0 place-items-center bg-tone-amber/16 leading-none text-tone-amber",
        size === "sm" && "size-5 rounded-md text-[11px]",
        size === "md" && "size-8 rounded-lg text-[15px]",
        size === "lg" && "size-10 rounded-xl text-[19px]",
      )}
    >
      ★
    </span>
  );
}

const activityTitles: Record<Activity, string> = {
  attention: "Waiting on you",
  working: "An agent is working",
  idle: "Agents are idle",
  none: "",
};

/** A small dot: accent when something waits on you, breathing while agents work. */
export function ActivityDot({ activity }: { activity: Activity }) {
  if (activity === "none") return null;
  return (
    <span
      role="img"
      aria-label={activityTitles[activity]}
      title={activityTitles[activity]}
      className={cx(
        "size-1.5 shrink-0 rounded-full",
        activity === "attention" && "bg-accent",
        activity === "working" && "animate-breathe bg-tone-blue",
        activity === "idle" && "bg-tone-green/80",
      )}
    />
  );
}

function WorkspaceRow({ workspace }: { workspace: Workspace }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const page = useUi((s) => s.page);
  const selected = useCos((s) => workspaceOfPage(s, page) === workspace.id);
  const exact = samePage(page, { kind: "workspace", id: workspace.id });
  const activity = useCos((s) => activityOf(s, workspaceAgents(s, workspace.id)));
  const hasRun = useCos((s) => currentRun(s, workspace.id) !== undefined);
  const runsSupported = useCos((s) => s.runsSupported);
  const agentsSupported = useCos((s) => s.agentsSupported);
  const name = workspaceName(workspace);

  const menu: MenuItem[] = [
    {
      label: "New agent…",
      disabled: !canChange || !agentsSupported,
      onSelect: () => ui.open({ kind: "newAgent", workspaceId: workspace.id }),
    },
    {
      label: "New run…",
      disabled: !canChange || !runsSupported || hasRun,
      detail: hasRun ? "A run is already going here." : undefined,
      onSelect: () => ui.open({ kind: "newRun", workspaceId: workspace.id }),
    },
    {
      label: "Rename…",
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "renameWorkspace", workspaceId: workspace.id }),
    },
    "separator",
    ...folderItems(cos, workspace.path),
    "separator",
    {
      label: "Remove workspace…",
      danger: true,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "removeWorkspace", workspaceId: workspace.id }),
    },
  ];

  return (
    <li role="none">
      <div
        role="treeitem"
        aria-selected={selected}
        className={cx(
          "group relative flex h-8 items-center rounded-lg pr-1 transition-colors",
          selected ? "bg-subtle-2 text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
        )}
      >
        <button
          type="button"
          onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}
          title={`${workspace.branch}\n${workspace.path}`}
          aria-current={exact ? "page" : undefined}
          className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-lg pr-1 pl-9 text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <GitBranch className="size-3.5 shrink-0 text-ink-3" strokeWidth={1.9} aria-hidden />
          <span className={cx("truncate", selected && "font-medium")}>{name}</span>
          <span className="ml-auto flex shrink-0 items-center pl-1">
            <ActivityDot activity={activity} />
          </span>
        </button>
        <Menu
          items={menu}
          align="start"
          trigger={(props) => (
            <button
              type="button"
              aria-label={`More for ${name}`}
              {...props}
              className={cx(
                "flex h-6 w-0 shrink-0 items-center justify-center overflow-hidden rounded-md text-ink-3 hover:bg-subtle-2 hover:text-ink",
                "opacity-0 group-hover:w-6 group-hover:opacity-100 focus-visible:w-6 focus-visible:opacity-100 aria-expanded:w-6 aria-expanded:opacity-100",
              )}
            >
              <MoreHorizontal className="size-4 shrink-0" strokeWidth={1.8} />
            </button>
          )}
        />
      </div>
    </li>
  );
}
