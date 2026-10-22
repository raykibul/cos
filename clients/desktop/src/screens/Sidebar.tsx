import {
  ChevronRight,
  Folder,
  FolderOpen,
  GitBranch,
  Inbox,
  ListChecks,
  MoreHorizontal,
  Plus,
  ScrollText,
  type LucideIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import type { Agent, Project, Run, Workspace } from "../api/models";
import { useApp, useCanChange, useCos, useUi } from "../app/context";
import { type Page, samePage } from "../app/ui";
import { agentLabel, isAlive, runLabel } from "../domain/labels";
import { isRunFinished } from "../domain/tasks";
import {
  agentNeedsYou,
  currentRun,
  sidebarRuns,
  userAgents,
  waitingCount,
  waitingCountForRun,
} from "../store/selectors";
import { cx } from "../ui/cx";
import { type MenuItem, Menu } from "../ui/Menu";
import { StateDot } from "../ui/kit";
import { toneClasses } from "../ui/tones";
import { folderItems } from "./common";
import { HostPicker, HostStatus } from "./Hosts";

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
      {/* Under the traffic lights; drags the window. */}
      <div data-tauri-drag-region className="h-[46px] shrink-0" />
      <HostPicker />
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
          <ul className="flex flex-col gap-px" role="tree" aria-label="Projects">
            {projects.map((project) => (
              <ProjectRows key={project.id} project={project} page={page} />
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

/** One row of the tree: indent, an optional chevron, content and a hover menu. */
function TreeRow({
  depth,
  selected,
  onClick,
  expanded,
  onToggle,
  children,
  menu,
  title,
}: {
  depth: number;
  selected: boolean;
  onClick: () => void;
  expanded?: boolean;
  onToggle?: () => void;
  children: ReactNode;
  menu?: MenuItem[];
  title?: string;
}) {
  return (
    <div
      role="treeitem"
      aria-selected={selected}
      aria-expanded={onToggle ? expanded : undefined}
      className={cx(
        "group relative flex h-[30px] items-center rounded-lg pr-1 transition-colors",
        selected ? "bg-subtle-2 text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
      )}
      style={{ paddingLeft: 4 + depth * 14 }}
    >
      {onToggle ? (
        <button
          type="button"
          tabIndex={-1}
          aria-label={expanded ? "Collapse" : "Expand"}
          onClick={onToggle}
          className="flex size-5 shrink-0 items-center justify-center rounded text-ink-3 hover:text-ink"
        >
          <ChevronRight
            className={cx("size-3.5 transition-transform duration-150", expanded && "rotate-90")}
            strokeWidth={2}
          />
        </button>
      ) : (
        <span className="w-5 shrink-0" />
      )}
      <button
        type="button"
        onClick={onClick}
        title={title}
        className={cx(
          "flex h-full min-w-0 flex-1 items-center gap-2 rounded-md pr-1 text-left text-[13px] outline-none",
          "focus-visible:ring-2 focus-visible:ring-accent/50",
          selected && "font-medium",
        )}
      >
        {children}
      </button>
      {menu ? (
        <Menu
          items={menu}
          align="start"
          trigger={(props) => (
            <button
              type="button"
              aria-label="More"
              {...props}
              className={cx(
                "flex size-6 shrink-0 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2 hover:text-ink",
                "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 aria-expanded:opacity-100",
              )}
            >
              <MoreHorizontal className="size-4" strokeWidth={1.8} />
            </button>
          )}
        />
      ) : null}
    </div>
  );
}

function AttentionDot() {
  return (
    <span
      className="ml-auto size-1.5 shrink-0 rounded-full bg-accent"
      aria-label="Waiting on you"
      role="img"
    />
  );
}

function ProjectRows({ project, page }: { project: Project; page: Page }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const key = `p${project.id}`;
  const collapsed = useUi((s) => s.collapsed[key] ?? false);
  const selected = samePage(page, { kind: "project", id: project.id });

  const menu: MenuItem[] = [
    {
      label: "New workspace…",
      icon: Plus,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "newWorkspace", projectId: project.id }),
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
    <li role="none">
      <TreeRow
        depth={0}
        selected={selected}
        expanded={!collapsed}
        onToggle={project.workspaces.length ? () => ui.toggle(key) : undefined}
        onClick={() => ui.navigate({ kind: "project", id: project.id })}
        menu={menu}
        title={project.root}
      >
        {collapsed ? (
          <Folder className="size-4 shrink-0 text-ink-3" strokeWidth={1.8} />
        ) : (
          <FolderOpen className="size-4 shrink-0 text-ink-3" strokeWidth={1.8} />
        )}
        <span className="truncate">{project.name}</span>
      </TreeRow>
      {!collapsed && project.workspaces.length > 0 ? (
        <ul role="group" className="flex flex-col gap-px">
          {project.workspaces.map((ws) => (
            <WorkspaceRows key={ws.id} workspace={ws} page={page} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

function WorkspaceRows({ workspace, page }: { workspace: Workspace; page: Page }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const key = `w${workspace.id}`;
  const collapsed = useUi((s) => s.collapsed[key] ?? false);
  const selectedRun = page.kind === "run" ? page.id : null;
  const runs = useCos((s) => sidebarRuns(s, workspace.id, selectedRun));
  const agents = useCos((s) => userAgents(s, workspace.id));
  const hasRun = useCos((s) => currentRun(s, workspace.id) !== undefined);
  const runsSupported = useCos((s) => s.runsSupported);
  const agentsSupported = useCos((s) => s.agentsSupported);
  const hasChildren = runs.length + agents.length > 0;

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
      <TreeRow
        depth={1}
        selected={samePage(page, { kind: "workspace", id: workspace.id })}
        expanded={!collapsed}
        onToggle={hasChildren ? () => ui.toggle(key) : undefined}
        onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}
        menu={menu}
        title={workspace.branch}
      >
        <GitBranch className="size-3.5 shrink-0 text-ink-3" strokeWidth={1.9} />
        <span className="truncate">{workspace.title}</span>
      </TreeRow>
      {!collapsed && hasChildren ? (
        <ul role="group" className="flex flex-col gap-px">
          {runs.map((run) => (
            <RunRow key={`run-${run.id}`} run={run} page={page} />
          ))}
          {agents.map((agent) => (
            <AgentRow key={`agent-${agent.id}`} agent={agent} page={page} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

function RunRow({ run, page }: { run: Run; page: Page }) {
  const { ui } = useApp();
  const canChange = useCanChange();
  const waiting = useCos((s) => waitingCountForRun(s, run.id));
  const label = runLabel(run.state);
  const menu: MenuItem[] = [];
  if (run.chiefAgentId !== null) {
    const chief = run.chiefAgentId;
    menu.push({
      label: "Open the chief's terminal",
      onSelect: () => ui.navigate({ kind: "agent", id: chief }),
    });
  }
  if (!isRunFinished(run.state)) {
    menu.push({
      label: "Cancel run…",
      danger: true,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "cancelRun", runId: run.id }),
    });
  }
  return (
    <li role="none">
      <TreeRow
        depth={2}
        selected={samePage(page, { kind: "run", id: run.id })}
        onClick={() => ui.navigate({ kind: "run", id: run.id })}
        menu={menu.length ? menu : undefined}
        title={`${run.goal}: ${label.title}`}
      >
        <ListChecks
          className={cx(
            "size-3.5 shrink-0",
            toneClasses[label.tone].text,
            label.busy && "animate-breathe",
          )}
          strokeWidth={2}
        />
        <span className="truncate">{run.goal}</span>
        {waiting > 0 ? <AttentionDot /> : null}
      </TreeRow>
    </li>
  );
}

function AgentRow({ agent, page }: { agent: Agent; page: Page }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const needsYou = useCos((s) => agentNeedsYou(s, agent));
  const label = agentLabel(agent.state);
  const menu: MenuItem[] = [
    isAlive(agent.state)
      ? {
          label: "Stop",
          disabled: !canChange,
          onSelect: () => void ui.perform("Couldn't stop the agent", () => cos.stopAgent(agent.id)),
        }
      : {
          label: "Resume",
          disabled: !canChange,
          onSelect: () =>
            void ui.perform("Couldn't resume the agent", () => cos.resumeAgent(agent.id)),
        },
    "separator",
    {
      label: "Remove agent…",
      danger: true,
      disabled: !canChange || isAlive(agent.state),
      onSelect: () => ui.open({ kind: "removeAgent", agentId: agent.id }),
    },
  ];
  return (
    <li role="none">
      <TreeRow
        depth={2}
        selected={samePage(page, { kind: "agent", id: agent.id })}
        onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
        menu={menu}
        title={`${agent.title}: ${label.title}`}
      >
        <span className="flex size-3.5 shrink-0 items-center justify-center">
          <StateDot label={label} className={cx(!isAlive(agent.state) && "opacity-50")} />
        </span>
        <span className={cx("truncate", !isAlive(agent.state) && "text-ink-3")}>{agent.title}</span>
        {needsYou ? <AttentionDot /> : null}
      </TreeRow>
    </li>
  );
}

function SidebarFooter() {
  const { ui } = useApp();
  const canChange = useCanChange();
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
      <HostStatus />
    </div>
  );
}
