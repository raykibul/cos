import { FolderGit2, GitBranch, MoreHorizontal, Pencil, Plus, Trash2 } from "lucide-react";
import { type Workspace, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { agentLabel, pluralize } from "../domain/labels";
import { agentNeedsYou, project as projectOf, userAgents } from "../store/selectors";
import { cx } from "../ui/cx";
import { relativeTime } from "../ui/format";
import { Button, EmptyState, PathText, SectionTitle, StateBadge } from "../ui/kit";
import { Menu } from "../ui/Menu";
import { Crumb, PageBody, PageHeader, PageTitle } from "../ui/Page";
import { folderItems } from "./common";

export function ProjectPage({ projectId }: { projectId: number }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const project = useCos((s) => projectOf(s, projectId));
  const loaded = useCos((s) => s.projectsLoaded);

  if (!project) {
    return (
      <>
        <PageHeader />
        <PageBody>
          <EmptyState icon={FolderGit2} title={loaded ? "This project is gone" : "Loading…"}>
            {loaded ? "It was removed. Pick another one in the sidebar." : null}
          </EmptyState>
        </PageBody>
      </>
    );
  }

  return (
    <>
      <PageHeader
        actions={
          <Menu
            align="end"
            items={[
              ...folderItems(cos, project.root),
              "separator",
              {
                label: "Remove project…",
                icon: Trash2,
                danger: true,
                disabled: !canChange,
                onSelect: () => ui.open({ kind: "removeProject", projectId }),
              },
            ]}
            trigger={(props) => (
              <button
                type="button"
                aria-label="Project actions"
                {...props}
                className="flex size-8 items-center justify-center rounded-lg text-ink-2 hover:bg-subtle hover:text-ink"
              >
                <MoreHorizontal className="size-4" strokeWidth={1.8} />
              </button>
            )}
          />
        }
      >
        <Crumb>{project.name}</Crumb>
      </PageHeader>
      <PageBody>
        <PageTitle
          subtitle={
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <PathText path={project.root} />
              <span className="flex items-center gap-1 text-ink-3">
                <GitBranch className="size-3.5" strokeWidth={1.8} />
                {project.baseBranch}
                {project.hasOrigin ? " · has origin" : " · no origin"}
              </span>
            </span>
          }
          actions={
            <Button
              variant="primary"
              icon={Plus}
              disabled={!canChange}
              onClick={() => ui.open({ kind: "newWorkspace", projectId })}
            >
              New workspace
            </Button>
          }
        >
          {project.name}
        </PageTitle>

        <SectionTitle>{pluralize(project.workspaces.length, "workspace")}</SectionTitle>
        {project.workspaces.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-line-strong">
            <EmptyState
              icon={GitBranch}
              title="No workspaces yet"
              actions={
                <Button
                  variant="primary"
                  icon={Plus}
                  disabled={!canChange}
                  onClick={() => ui.open({ kind: "newWorkspace", projectId })}
                >
                  New workspace
                </Button>
              }
            >
              A workspace is a git worktree on its own branch. Agents work inside one.
            </EmptyState>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
            {project.workspaces.map((workspace) => (
              <WorkspaceCard key={workspace.id} workspace={workspace} />
            ))}
          </div>
        )}
      </PageBody>
    </>
  );
}

function WorkspaceCard({ workspace }: { workspace: Workspace }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const now = useNow();
  const agents = useCos((s) => userAgents(s, workspace.id));
  const waiting = useCos(
    (s) => userAgents(s, workspace.id).filter((a) => agentNeedsYou(s, a)).length,
  );
  const working = agents.filter((a) => agentLabel(a.state).busy).length;

  return (
    <div className="group relative flex flex-col gap-2 rounded-xl border border-line bg-surface p-4 shadow-card transition-colors hover:border-line-strong hover:bg-raised">
      <div className="flex w-full items-center gap-2">
        {/* The title's button covers the card, so the whole card opens it. */}
        <button
          type="button"
          onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}
          className="truncate rounded text-left font-serif text-[16px] font-medium text-ink after:absolute after:inset-0 after:rounded-xl"
        >
          {workspaceName(workspace)}
        </button>
        {waiting > 0 ? (
          <span className="ml-auto rounded-full bg-accent/12 px-2 py-0.5 text-[11.5px] font-medium text-accent-ink">
            {waiting} waiting
          </span>
        ) : null}
        <Menu
          align="end"
          width={200}
          items={[
            {
              label: "Rename…",
              icon: Pencil,
              disabled: !canChange,
              onSelect: () => ui.open({ kind: "renameWorkspace", workspaceId: workspace.id }),
            },
            ...folderItems(cos, workspace.path),
            "separator",
            {
              label: "Remove workspace…",
              icon: Trash2,
              danger: true,
              disabled: !canChange,
              onSelect: () => ui.open({ kind: "removeWorkspace", workspaceId: workspace.id }),
            },
          ]}
          trigger={(props) => (
            <button
              type="button"
              aria-label={`Actions for ${workspaceName(workspace)}`}
              {...props}
              className={cx(
                "relative z-10 flex size-6 shrink-0 items-center justify-center rounded-md text-ink-3 hover:bg-subtle hover:text-ink",
                "opacity-0 group-hover:opacity-100 focus-visible:opacity-100 aria-expanded:opacity-100",
                waiting > 0 ? "" : "ml-auto",
              )}
            >
              <MoreHorizontal className="size-4" strokeWidth={1.8} />
            </button>
          )}
        />
      </div>
      <div className="flex items-center gap-1.5 font-mono text-[12px] text-ink-3">
        <GitBranch className="size-3.5" strokeWidth={1.8} />
        <span className="truncate">{workspace.branch}</span>
        <span className="shrink-0">from {workspace.base}</span>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px] text-ink-2">
        <span>
          {pluralize(agents.length, "agent")}
          {working > 0 ? ` · ${working} working` : ""}
        </span>
        <span className="ml-auto text-ink-3">created {relativeTime(workspace.createdAt, now)}</span>
      </div>
      {agents[0] ? (
        <div className="flex min-w-0 items-center gap-2">
          <StateBadge label={agentLabel(agents[0].state)} />
          <span className="truncate text-[12.5px] text-ink-3">{agents[0].title}</span>
        </div>
      ) : null}
    </div>
  );
}
