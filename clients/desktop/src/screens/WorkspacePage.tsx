import { Bot, GitBranch, MoreHorizontal, Pencil, Trash2 } from "lucide-react";
import { workspaceName } from "../api/models";
import { useApp, useCanChange, useCos } from "../app/context";
import { project as projectOf, userAgents, workspace as workspaceOf } from "../store/selectors";
import { EmptyState, PathText, Pill, SectionTitle } from "../ui/kit";
import { Menu } from "../ui/Menu";
import { Crumb, CrumbSep, PageBody, PageHeader, PageTitle } from "../ui/Page";
import { folderItems } from "./common";
import { Composer } from "./Composer";
import { toNewAgent } from "./composerValue";
import { AgentTabs } from "../pages/workspace/AgentTabs";

export function WorkspacePage({ workspaceId }: { workspaceId: number }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const workspace = useCos((s) => workspaceOf(s, workspaceId));
  const project = useCos((s) => (workspace ? projectOf(s, workspace.projectId) : undefined));
  const loaded = useCos((s) => s.projectsLoaded);
  const agents = useCos((s) => userAgents(s, workspaceId));
  const approvals = useCos((s) => s.approvals);
  const agentsSupported = useCos((s) => s.agentsSupported);
  const agentsError = useCos((s) => s.agentsError);

  if (!workspace) {
    return (
      <>
        <PageHeader />
        <PageBody>
          <EmptyState icon={GitBranch} title={loaded ? "This workspace is gone" : "Loading…"}>
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
              {
                label: "Rename…",
                icon: Pencil,
                disabled: !canChange,
                onSelect: () => ui.open({ kind: "renameWorkspace", workspaceId }),
              },
              ...folderItems(cos, workspace.path),
              "separator",
              {
                label: "Remove workspace…",
                icon: Trash2,
                danger: true,
                disabled: !canChange,
                onSelect: () => ui.open({ kind: "removeWorkspace", workspaceId }),
              },
            ]}
            trigger={(props) => (
              <button
                type="button"
                aria-label="Workspace actions"
                {...props}
                className="flex size-8 items-center justify-center rounded-lg text-ink-2 hover:bg-subtle hover:text-ink"
              >
                <MoreHorizontal className="size-4" strokeWidth={1.8} />
              </button>
            )}
          />
        }
      >
        {project ? (
          <>
            <Crumb onClick={() => ui.navigate({ kind: "chief", id: project.id })}>
              <span className="text-tone-amber">★</span> {project.name}
            </Crumb>
            <CrumbSep />
          </>
        ) : null}
        <Crumb>{workspaceName(workspace)}</Crumb>
      </PageHeader>
      <PageBody wide>
        <PageTitle
          badge={workspace.title !== null ? <Pill>{workspace.city}</Pill> : null}
          subtitle={
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="flex items-center gap-1 font-mono text-[12.5px] text-ink-2">
                <GitBranch className="size-3.5" strokeWidth={1.8} />
                {workspace.branch}
                <span className="text-ink-3">from {workspace.base}</span>
              </span>
              <PathText path={workspace.path} className="text-ink-3" />
            </span>
          }
        >
          {workspaceName(workspace)}
        </PageTitle>

        <section className="mb-9">
          <SectionTitle>Agents</SectionTitle>
          {agentsError ? <p className="mb-2 text-[12.5px] text-tone-red">{agentsError}</p> : null}
          {agents.length === 0 ? (
            <div className="rounded-2xl border border-dashed border-line-strong">
              <EmptyState icon={Bot} title="No agents here yet">
                Type below to start one.
              </EmptyState>
            </div>
          ) : (
            <AgentTabs agents={agents} path={workspace.path} approvals={approvals} />
          )}
        </section>

        <section className="mb-9">
          <SectionTitle>New agent</SectionTitle>
          <Composer
            focusOnOpen
            disabled={!canChange || !agentsSupported}
            placeholder={
              agentsSupported
                ? `What should Claude work on in ${workspaceName(workspace)}?`
                : "This daemon can't start agents. Update cos."
            }
            onSend={(value) =>
              ui.perform("Couldn't start the agent", async () => {
                const agent = await cos.createAgent(workspaceId, toNewAgent(value));
                ui.navigate({ kind: "agent", id: agent.id });
              })
            }
          />
          <p className="mt-2 px-1 text-[12px] text-ink-3">
            Starts Claude Code in this worktree. Return sends, Shift-Return adds a line.
          </p>
        </section>
      </PageBody>
    </>
  );
}
