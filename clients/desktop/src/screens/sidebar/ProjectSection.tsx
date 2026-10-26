// The project the sidebar shows (the rail picks it): its name and menu, a
// `★ Chief of Staff` row that opens the chief, and its sessions, one row
// each, newest first. Workspaces stay out of sight; sessions that share one
// folder sit together, joined by a bracket in that folder's colour.

import {
  FolderX,
  GitBranch,
  LayoutGrid,
  MoreHorizontal,
  NotebookPen,
  Plus,
  Trash2,
  Variable,
} from "lucide-react";
import { useMemo } from "react";
import { envHint } from "../../api/env";
import { type Agent, type Project, workspaceName } from "../../api/models";
import { useApp, useCanChange, useCos, useUi } from "../../app/context";
import { samePage } from "../../app/ui";
import { agentLabel, attentionWord } from "../../domain/labels";
import { staffLabel } from "../../pages/chief/staffState";
import {
  type Activity,
  type SessionGroup,
  projectEnvNames,
  sessionGroups,
  spawnedAgents,
  staffState,
} from "../../store/selectors";
import { cx } from "../../ui/cx";
import { StateDot } from "../../ui/kit";
import { StateIcon } from "../../ui/StateIcon";
import { type MenuItem, Menu } from "../../ui/Menu";
import { folderItems } from "../common";

export function ProjectPanel({ project }: { project: Project }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const env = envHint(useCos((s) => projectEnvNames(s, project.id)));
  const agents = useCos((s) => s.agents);
  const agentsSupported = useCos((s) => s.agentsSupported);
  const groups = useMemo(() => sessionGroups(agents, project), [agents, project]);

  const menu: MenuItem[] = [
    {
      label: "New session…",
      icon: Plus,
      disabled: !canChange || !agentsSupported,
      onSelect: () => ui.open({ kind: "newSession", projectId: project.id }),
    },
    {
      label: "New workspace…",
      icon: GitBranch,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "newWorkspace", projectId: project.id }),
    },
    {
      label: "All workspaces",
      icon: LayoutGrid,
      onSelect: () => ui.navigate({ kind: "project", id: project.id }),
    },
    {
      label: "Chief's brief & memory…",
      icon: NotebookPen,
      onSelect: () => ui.navigate({ kind: "chiefMemory", id: project.id }),
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
          Sessions
        </span>
        <button
          type="button"
          aria-label={`New session in ${project.name}`}
          title="New session"
          disabled={!canChange || !agentsSupported}
          onClick={() => ui.open({ kind: "newSession", projectId: project.id })}
          className="ml-auto flex size-6 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2/70 hover:text-ink disabled:opacity-40"
        >
          <Plus className="size-3.5" strokeWidth={2} />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-4">
        {groups.length === 0 ? (
          <p className="px-2 py-1.5 text-[12.5px] leading-snug text-ink-3">
            No sessions yet. Ask the chief, or start one with +.
          </p>
        ) : (
          <ul role="tree" aria-label="Sessions" className="flex flex-col gap-px">
            {groups.map((group, index) =>
              group.agents.length === 1 ? (
                <SessionRow key={group.agents[0]!.id} agent={group.agents[0]!} group={group} />
              ) : (
                <SharedFolder
                  key={`w${group.workspace.id}`}
                  group={group}
                  tint={folderTints[sharedIndex(groups, index) % folderTints.length]!}
                />
              ),
            )}
          </ul>
        )}
      </div>
    </div>
  );
}

/** Colours for folders that more than one session shares, in turn. */
const folderTints = [
  "border-tone-purple/80",
  "border-tone-teal/80",
  "border-tone-blue/80",
  "border-tone-green/80",
  "border-tone-amber/80",
];

/** Which shared folder this is, counting only the groups with more than one session. */
function sharedIndex(groups: SessionGroup[], index: number): number {
  return groups.slice(0, index).filter((g) => g.agents.length > 1).length;
}

/** Sessions working in the same folder, joined by a bracket; its tooltip names the folder. */
function SharedFolder({ group, tint }: { group: SessionGroup; tint: string }) {
  const name = workspaceName(group.workspace);
  return (
    <li role="none" className="relative my-0.5">
      <span
        aria-hidden
        title={`${group.agents.length} sessions share ${name}\n${group.workspace.path}\nTheir edits land in the same files.`}
        className="absolute top-2 bottom-2 left-0.5 w-3 cursor-default"
      >
        <span
          className={cx("absolute inset-y-0 left-1 w-[7px] rounded-l-md border-2 border-r-0", tint)}
        />
      </span>
      <ul
        role="group"
        aria-label={`${group.agents.length} sessions in ${name}`}
        className="flex flex-col gap-px"
      >
        {group.agents.map((agent) => (
          <SessionRow key={agent.id} agent={agent} group={group} indent />
        ))}
      </ul>
    </li>
  );
}

function SessionRow({
  agent,
  group,
  indent = false,
}: {
  agent: Agent;
  group: SessionGroup;
  indent?: boolean;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const agentsSupported = useCos((s) => s.agentsSupported);
  const selected = useUi((s) => samePage(s.page, { kind: "agent", id: agent.id }));
  const label = agentLabel(agent.state);
  const word = attentionWord(agent.state);
  const name = workspaceName(group.workspace);
  const others = group.agents.filter((a) => a.id !== agent.id).map((a) => a.title);
  const tooltip = [
    agent.title,
    `${label.title} · ${agent.model ?? "default model"}`,
    agent.spawnedBy !== null ? "Started by the Chief of Staff" : null,
    others.length > 0 ? `Shares ${name} with ${others.join(", ")}` : `In ${name}`,
  ]
    .filter(Boolean)
    .join("\n");
  // Right-click: another session in the same folder, the folder, or remove it.
  const menu: MenuItem[] = [
    {
      label: `New session in ${name}`,
      icon: Plus,
      disabled: !canChange || !agentsSupported,
      onSelect: () => ui.open({ kind: "newAgent", workspaceId: group.workspace.id }),
    },
    "separator",
    ...folderItems(cos, agent.cwd ?? group.workspace.path),
    "separator",
    {
      label: "Remove session…",
      icon: Trash2,
      danger: true,
      detail: others.length > 0 ? undefined : `Deletes ${name} with it`,
      disabled: !canChange,
      onSelect: () => ui.open({ kind: "removeAgent", agentId: agent.id }),
    },
    // Alone in its folder, Remove session already deletes the folder.
    ...(others.length > 0
      ? [
          {
            label: `Remove ${name}…`,
            icon: FolderX,
            danger: true,
            detail: `The folder and its ${others.length + 1} sessions`,
            disabled: !canChange,
            onSelect: () => ui.open({ kind: "removeWorkspace", workspaceId: group.workspace.id }),
          } satisfies MenuItem,
        ]
      : []),
  ];
  return (
    <li role="none">
      <Menu
        items={menu}
        width={240}
        trigger={({ ref, onContextMenu, ...popup }) => (
          <button
            ref={ref}
            type="button"
            role="treeitem"
            aria-selected={selected}
            aria-label={`${agent.title}, ${label.title}`}
            aria-haspopup={popup["aria-haspopup"]}
            aria-expanded={popup["aria-expanded"]}
            aria-controls={popup["aria-controls"]}
            title={tooltip}
            onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
            onContextMenu={onContextMenu}
            className={cx(
              "flex h-8 w-full items-center gap-2 rounded-lg pr-2 text-left text-[13px] transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
              indent ? "pl-[22px]" : "pl-2.5",
              selected
                ? "bg-subtle-2 text-ink"
                : "text-ink-2 hover:bg-subtle hover:text-ink aria-expanded:bg-subtle",
              agent.state === "exited" && !selected && "text-ink-3",
            )}
          >
            <StateIcon state={agent.state} />
            <span className={cx("min-w-0 flex-1 truncate", word && "text-ink")}>{agent.title}</span>
            {word ? (
              <span className="shrink-0 rounded-full bg-accent/14 px-1.5 py-px text-[10.5px] font-semibold text-accent-ink">
                {word}
              </span>
            ) : agent.state === "interrupted" ? (
              <span className="shrink-0 text-[11px] text-tone-amber">stopped</span>
            ) : agent.state === "exited" ? (
              <span className="shrink-0 text-[11px] text-ink-3">exited</span>
            ) : null}
          </button>
        )}
      />
    </li>
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
