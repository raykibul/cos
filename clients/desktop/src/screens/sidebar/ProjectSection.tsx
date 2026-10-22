// The project the sidebar shows (the rail picks it): its name and menu, a
// `★ Chief of Staff` row that opens the chief, and its workspaces, newest
// first, each with your agents under it. Runs are on the workspace page.

import { GitBranch, LayoutGrid, MoreHorizontal, Plus, Variable } from "lucide-react";
import { envHint } from "../../api/env";
import { type Agent, type Project, type Workspace, workspaceName } from "../../api/models";
import { useApp, useCanChange, useCos, useUi } from "../../app/context";
import { samePage } from "../../app/ui";
import { agentLabel } from "../../domain/labels";
import { staffLabel } from "../../pages/chief/staffState";
import {
  type Activity,
  activityOf,
  currentRun,
  projectEnvNames,
  spawnedAgents,
  staffState,
  userAgents,
  workspaceAgents,
  workspaceOfPage,
  workspacesNewestFirst,
} from "../../store/selectors";
import { cx } from "../../ui/cx";
import { StateDot } from "../../ui/kit";
import { type MenuItem, Menu } from "../../ui/Menu";
import { folderItems } from "../common";

export function ProjectPanel({ project }: { project: Project }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const env = envHint(useCos((s) => projectEnvNames(s, project.id)));
  const workspaces = workspacesNewestFirst(project);

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
    {
      label: "Environment…",
      icon: Variable,
      detail: env ?? undefined,
      onSelect: () => ui.navigate({ kind: "projectEnv", id: project.id }),
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
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-start gap-1 px-4 pt-2 pb-3">
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-serif text-[20px] leading-tight font-medium tracking-[-0.01em] text-ink">
            {project.name}
          </h2>
          <p
            className="mt-0.5 truncate font-mono text-[11px] text-ink-3"
            title={`${project.root}${env ? `\n${env}` : ""}`}
          >
            {project.root}
          </p>
        </div>
        <Menu
          items={menu}
          align="start"
          trigger={(props) => (
            <button
              type="button"
              aria-label={`More for ${project.name}`}
              {...props}
              className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2 hover:text-ink aria-expanded:bg-subtle-2"
            >
              <MoreHorizontal className="size-4" strokeWidth={1.8} />
            </button>
          )}
        />
      </div>

      <div className="px-2.5">
        <ChiefRow project={project} />
      </div>

      <div className="mt-4 flex items-center px-4 pb-1">
        <span className="text-[11.5px] font-semibold tracking-wide text-ink-3 uppercase">
          Workspaces
        </span>
        <button
          type="button"
          aria-label={`New workspace in ${project.name}`}
          title="New workspace"
          disabled={!canChange}
          onClick={() => ui.open({ kind: "newWorkspace", projectId: project.id })}
          className="ml-auto flex size-6 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2/70 hover:text-ink disabled:opacity-40"
        >
          <Plus className="size-3.5" strokeWidth={2} />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-4">
        {workspaces.length === 0 ? (
          <p className="px-2 py-1.5 text-[12.5px] leading-snug text-ink-3">
            No workspaces yet. Ask the chief, or add one with +.
          </p>
        ) : (
          <ul role="tree" aria-label="Workspaces" className="flex flex-col gap-px">
            {workspaces.map((workspace) => (
              <WorkspaceRow key={workspace.id} workspace={workspace} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function ChiefRow({ project }: { project: Project }) {
  const { ui } = useApp();
  const selected = useUi((s) => samePage(s.page, { kind: "chief", id: project.id }));
  const state = useCos((s) => staffState(s, project.id));
  const working = useCos(
    (s) =>
      spawnedAgents(s, s.staffs[project.id]?.agentId).filter(
        (a) => a.state === "working" || a.state === "starting",
      ).length,
  );
  const label = staffLabel(state);
  return (
    <button
      type="button"
      onClick={() => ui.navigate({ kind: "chief", id: project.id })}
      aria-current={selected ? "page" : undefined}
      aria-label={`Chief of Staff, ${project.name}`}
      title={`Chief of Staff for ${project.name}`}
      className={cx(
        "flex w-full items-center gap-2.5 rounded-xl border px-2.5 py-2 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
        selected ? "border-tone-amber/25 bg-tone-amber/10" : "border-transparent hover:bg-subtle",
      )}
    >
      <ChiefMark size="md" />
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-semibold text-ink">Chief of Staff</span>
        <span className="mt-px flex items-center gap-1.5 text-[11.5px] text-ink-3">
          <StateDot label={label} className="size-1.5" />
          <span className="truncate">
            {label.title}
            {working > 0 ? ` · ${working} working` : ""}
          </span>
        </span>
      </span>
    </button>
  );
}

function WorkspaceRow({ workspace }: { workspace: Workspace }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const page = useUi((s) => s.page);
  const selected = useCos((s) => workspaceOfPage(s, page) === workspace.id);
  const exact = samePage(page, { kind: "workspace", id: workspace.id });
  const activity = useCos((s) => activityOf(s, workspaceAgents(s, workspace.id)));
  const agents = useCos((s) => userAgents(s, workspace.id));
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
        aria-expanded={agents.length > 0 ? true : undefined}
        className={cx(
          "group relative flex h-8 items-center rounded-lg pr-1 transition-colors",
          exact ? "bg-subtle-2 text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
        )}
      >
        <button
          type="button"
          onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}
          title={`${workspace.branch}\n${workspace.path}`}
          aria-current={exact ? "page" : undefined}
          className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-lg pr-1 pl-2 text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <GitBranch className="size-3.5 shrink-0 text-ink-3" strokeWidth={1.9} aria-hidden />
          <span className={cx("truncate", selected && "font-medium text-ink")}>{name}</span>
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
      {agents.length > 0 ? (
        <ul role="group" className="mb-1 flex flex-col">
          {agents.map((agent) => (
            <AgentRow key={agent.id} agent={agent} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

function AgentRow({ agent }: { agent: Agent }) {
  const { ui } = useApp();
  const selected = useUi((s) => samePage(s.page, { kind: "agent", id: agent.id }));
  const label = agentLabel(agent.state);
  return (
    <li role="none">
      <button
        type="button"
        role="treeitem"
        aria-selected={selected}
        title={`${agent.title}\n${label.title}`}
        onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
        className={cx(
          "flex h-7 w-full items-center gap-2 rounded-lg pr-2 pl-[30px] text-left text-[12.5px] transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
          selected ? "bg-subtle-2 text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
        )}
      >
        <StateDot label={label} className="size-1.5" />
        <span className="truncate">{agent.title}</span>
      </button>
    </li>
  );
}

/** The chief's amber star, in a soft square. */
export function ChiefMark({ size = "sm" }: { size?: "xs" | "sm" | "md" | "lg" }) {
  return (
    <span
      aria-hidden
      className={cx(
        "grid shrink-0 place-items-center bg-tone-amber/16 leading-none text-tone-amber",
        size === "xs" && "size-4 rounded text-[9px]",
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
