import { Bot, GitBranch, ListChecks, MoreHorizontal, Pencil, Play, Trash2 } from "lucide-react";
import { type Run, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { runLabel } from "../domain/labels";
import { countsSummary, isRunFinished, progress } from "../domain/tasks";
import {
  currentRun,
  project as projectOf,
  runsIn,
  userAgents,
  workspace as workspaceOf,
} from "../store/selectors";
import { relativeTime } from "../ui/format";
import { Button, EmptyState, PathText, Pill, SectionTitle, StateBadge } from "../ui/kit";
import { Menu } from "../ui/Menu";
import { Crumb, CrumbSep, PageBody, PageHeader, PageTitle } from "../ui/Page";
import { toneClasses } from "../ui/tones";
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
  const runsSupported = useCos((s) => s.runsSupported);
  const active = useCos((s) => currentRun(s, workspaceId));
  const past = useCos((s) => runsIn(s, workspaceId).filter((r) => isRunFinished(r.state)));

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

  const newRun = (
    <Button
      icon={ListChecks}
      disabled={!canChange || !runsSupported || active !== undefined}
      title={active ? "This workspace already has a run" : undefined}
      onClick={() => ui.open({ kind: "newRun", workspaceId })}
    >
      New run
    </Button>
  );

  return (
    <>
      <PageHeader
        actions={
          <>
            {newRun}
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
          </>
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
                Type below to start one. Agents a run starts are on its page.
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

        {active ? (
          <section className="mb-9">
            <SectionTitle>Current run</SectionTitle>
            <RunCard run={active} />
          </section>
        ) : null}

        {runsSupported ? (
          <section>
            <SectionTitle
              actions={
                active ? null : (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={Play}
                    disabled={!canChange}
                    onClick={() => ui.open({ kind: "newRun", workspaceId })}
                  >
                    Start a run
                  </Button>
                )
              }
            >
              Past runs
            </SectionTitle>
            {past.length === 0 ? (
              <p className="px-1 text-[13px] text-ink-3">
                A run hands a goal to a chief, who plans tasks and runs workers on them.
              </p>
            ) : (
              <div className="flex flex-col gap-2">
                {past.map((run) => (
                  <RunCard key={run.id} run={run} compact />
                ))}
              </div>
            )}
          </section>
        ) : null}
      </PageBody>
    </>
  );
}

export function RunCard({ run, compact }: { run: Run; compact?: boolean }) {
  const { ui } = useApp();
  const now = useNow();
  const label = runLabel(run.state);
  const counts = run.counts;
  return (
    <button
      type="button"
      onClick={() => ui.navigate({ kind: "run", id: run.id })}
      className="flex w-full flex-col gap-2 rounded-xl border border-line bg-surface px-4 py-3 text-left shadow-card transition-colors hover:border-line-strong hover:bg-raised"
    >
      <div className="flex w-full items-center gap-2.5">
        <span className="min-w-0 flex-1 truncate text-[14px] font-medium text-ink">{run.goal}</span>
        <StateBadge label={label} />
      </div>
      {!compact && counts && counts.total > 0 ? (
        <div className="h-1 w-full overflow-hidden rounded-full bg-subtle">
          <div
            className={`h-full rounded-full ${toneClasses[label.tone].edge}`}
            style={{ width: `${Math.round(progress(counts) * 100)}%` }}
          />
        </div>
      ) : null}
      <div className="flex w-full items-center gap-2 text-[12px] text-ink-3">
        <span className="truncate">
          {counts && counts.total > 0 ? countsSummary(counts) : (run.reason ?? "No tasks yet")}
        </span>
        <span className="ml-auto shrink-0">{relativeTime(run.updatedAt, now)}</span>
      </div>
    </button>
  );
}
