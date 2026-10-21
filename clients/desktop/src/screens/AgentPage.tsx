import {
  Bot,
  MoreHorizontal,
  Play,
  ShieldQuestion,
  Square,
  Trash2,
  UserRoundCheck,
} from "lucide-react";
import { useState } from "react";
import type { Agent, Approval } from "../api/models";
import { useApp, useCanChange, useCos } from "../app/context";
import {
  approvalDetail,
  approvalHeadline,
  isWithChief,
  routeExplanation,
} from "../domain/approvals";
import { agentLabel, canResume, isAlive, permissionModeTitle } from "../domain/labels";
import {
  agent as agentOf,
  pendingApproval,
  project as projectOf,
  run as runOf,
  workspace as workspaceOf,
} from "../store/selectors";
import { AgentTerminal } from "../terminal/AgentTerminal";
import { cx } from "../ui/cx";
import { Button, EmptyState, PathText, StateBadge } from "../ui/kit";
import { type MenuItem, Menu } from "../ui/Menu";
import { Crumb, CrumbSep, PageHeader } from "../ui/Page";
import { folderItems } from "./common";
import { ApprovalButtons } from "./shared";

export function AgentPage({ agentId }: { agentId: number }) {
  const agent = useCos((s) => agentOf(s, agentId));
  const loaded = useCos((s) => s.projectsLoaded && s.agents.length > 0);
  if (!agent) {
    return (
      <>
        <PageHeader />
        <EmptyState
          icon={Bot}
          title={loaded ? "This agent is gone" : "Loading…"}
          className="flex-1"
        >
          {loaded ? "It was removed. Pick another one in the sidebar." : null}
        </EmptyState>
      </>
    );
  }
  return <AgentView agent={agent} />;
}

function AgentView({ agent }: { agent: Agent }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const workspace = useCos((s) => workspaceOf(s, agent.workspaceId));
  const project = useCos((s) => (workspace ? projectOf(s, workspace.projectId) : undefined));
  const run = useCos((s) => (agent.runId !== null ? runOf(s, agent.runId) : undefined));
  const approval = useCos((s) => pendingApproval(s, agent.id));
  const [busy, setBusy] = useState<"stop" | "resume" | null>(null);
  const alive = isAlive(agent.state);
  const label = agentLabel(agent.state);
  const cwd = agent.cwd ?? workspace?.path ?? null;

  const act = (kind: "stop" | "resume") => {
    setBusy(kind);
    void ui
      .perform(kind === "stop" ? "Couldn't stop the agent" : "Couldn't resume the agent", () =>
        kind === "stop" ? cos.stopAgent(agent.id) : cos.resumeAgent(agent.id),
      )
      .finally(() => setBusy(null));
  };

  const menu: MenuItem[] = [
    ...(cwd ? folderItems(cos, cwd) : []),
    "separator",
    {
      label: "Remove agent…",
      icon: Trash2,
      danger: true,
      disabled: !canChange || alive,
      detail: alive ? "Stop it first" : undefined,
      onSelect: () => ui.open({ kind: "removeAgent", agentId: agent.id }),
    },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        actions={
          <>
            {alive ? (
              <Button
                icon={Square}
                busy={busy === "stop"}
                disabled={!canChange || agent.state === "starting"}
                onClick={() => act("stop")}
                title="Sends /exit to Claude"
              >
                Stop
              </Button>
            ) : null}
            {canResume(agent.state) ? (
              <Button
                variant="primary"
                icon={Play}
                busy={busy === "resume"}
                disabled={!canChange || !agent.sessionId}
                title={
                  agent.sessionId
                    ? "Starts Claude again on this conversation"
                    : "No session to resume"
                }
                onClick={() => act("resume")}
              >
                Resume
              </Button>
            ) : null}
            <Menu
              align="end"
              items={menu}
              trigger={(props) => (
                <button
                  type="button"
                  aria-label="Agent actions"
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
            <Crumb onClick={() => ui.navigate({ kind: "project", id: project.id })}>
              {project.name}
            </Crumb>
            <CrumbSep />
          </>
        ) : null}
        {workspace ? (
          <>
            <Crumb onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}>
              {workspace.title}
            </Crumb>
            <CrumbSep />
          </>
        ) : null}
        {run ? (
          <>
            <Crumb onClick={() => ui.navigate({ kind: "run", id: run.id })}>{run.goal}</Crumb>
            <CrumbSep />
          </>
        ) : null}
        <span className="truncate font-medium text-ink">{agent.title}</span>
        <StateBadge label={label} className="ml-1.5" />
      </PageHeader>

      <div className="flex min-h-0 flex-1 flex-col gap-3 px-5 pb-4">
        {agent.state === "needs_trust" ? <TrustBanner agent={agent} path={cwd} /> : null}
        {approval ? <ApprovalBanner approval={approval} /> : null}
        <AgentTerminal key={agent.id} agentId={agent.id} alive={alive} className="flex-1" />
        <AgentFooter agent={agent} cwd={cwd} />
      </div>
    </div>
  );
}

function TrustBanner({ agent, path }: { agent: Agent; path: string | null }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState(false);
  return (
    <Banner icon={ShieldQuestion} tone="accent">
      <div className="min-w-0 flex-1">
        <p className="text-[13.5px] font-medium text-ink">
          Claude asks whether to trust this folder
        </p>
        {path ? <PathText path={path} /> : null}
      </div>
      <Button
        variant="accent"
        busy={busy}
        disabled={!canChange}
        onClick={() => {
          setBusy(true);
          void ui
            .perform("Couldn't answer the trust dialog", () => cos.trustAgent(agent.id))
            .finally(() => setBusy(false));
        }}
      >
        Trust folder
      </Button>
    </Banner>
  );
}

function ApprovalBanner({ approval }: { approval: Approval }) {
  const withChief = isWithChief(approval);
  const why = routeExplanation(approval);
  return (
    <Banner
      icon={withChief ? UserRoundCheck : ShieldQuestion}
      tone={withChief ? "neutral" : "accent"}
    >
      <div className="min-w-0 flex-1">
        <p className="text-[13.5px] font-medium text-ink">
          {withChief ? "The chief is deciding" : approvalHeadline(approval)}
        </p>
        <p
          className="mt-0.5 truncate font-mono text-[12.5px] text-ink-2 select-text"
          title={approvalDetail(approval)}
        >
          {withChief ? `${approval.toolName}: ` : ""}
          {approvalDetail(approval)}
        </p>
        {why ? <p className="mt-0.5 text-[12px] text-ink-3">{why}</p> : null}
      </div>
      {withChief ? null : <ApprovalButtons approval={approval} />}
    </Banner>
  );
}

function Banner({
  icon: Icon,
  tone,
  children,
}: {
  icon: typeof Bot;
  tone: "accent" | "neutral";
  children: React.ReactNode;
}) {
  return (
    <div
      role="region"
      aria-label="Waiting"
      className={cx(
        "flex items-center gap-3.5 rounded-xl border px-4 py-3 animate-fade-in",
        tone === "accent" ? "border-accent/30 bg-accent/8" : "border-line bg-subtle/70",
      )}
    >
      <Icon
        className={cx("size-5 shrink-0", tone === "accent" ? "text-accent" : "text-ink-3")}
        strokeWidth={1.7}
      />
      {children}
    </div>
  );
}

function AgentFooter({ agent, cwd }: { agent: Agent; cwd: string | null }) {
  const parts = [
    agent.role !== "user" ? agent.role : null,
    agent.model ?? "default model",
    permissionModeTitle(agent.permissionMode),
    agent.pid !== null ? `pid ${agent.pid}` : null,
    agent.exitCode !== null ? `exit ${agent.exitCode}` : null,
  ].filter(Boolean);
  return (
    <div className="flex items-center gap-3 px-1 text-[12px] text-ink-3">
      <span className="shrink-0">{parts.join(" · ")}</span>
      {cwd ? <PathText path={cwd} className="ml-auto truncate text-[11.5px] text-ink-3" /> : null}
    </div>
  );
}
