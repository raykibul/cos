// A workspace's agents as tabs, each with its live terminal. Agents left the
// sidebar for this (docs/chief-of-staff.md, "App"), so the page is where
// you watch and answer them.

import { Maximize2, Play, Square } from "lucide-react";
import { useState } from "react";
import { sortedNames } from "../../api/env";
import type { Agent, Approval } from "../../api/models";
import { useApp, useCanChange, useCos, useNow } from "../../app/context";
import { agentLabel, canResume, isAlive } from "../../domain/labels";
import { agentNeedsYou, pendingApproval } from "../../store/selectors";
import { AgentTerminal } from "../../terminal/AgentTerminal";
import { ApprovalBanner, EnvNames, LoginBanner, TrustBanner } from "../../screens/AgentPage";
import { ChiefMark } from "../../screens/sidebar/ProjectSection";
import { cx } from "../../ui/cx";
import { relativeTime } from "../../ui/format";
import { Button, StateBadge, StateDot } from "../../ui/kit";

/** The tab to open first: one waiting on you, else one that's running, else the newest. */
function firstTab(agents: Agent[], needsYou: (agent: Agent) => boolean): number | null {
  return (
    agents.find(needsYou)?.id ?? agents.find((a) => isAlive(a.state))?.id ?? agents[0]?.id ?? null
  );
}

export function AgentTabs({
  agents,
  path,
  approvals,
}: {
  agents: Agent[];
  /** The workspace's folder, for the trust banner. */
  path: string;
  approvals: Approval[];
}) {
  const waiting = useCos((s) => agents.filter((a) => agentNeedsYou(s, a)).map((a) => a.id));
  const [picked, setPicked] = useState<number | null>(() =>
    firstTab(agents, (a) => waiting.includes(a.id)),
  );
  const selected = agents.find((a) => a.id === picked) ?? agents[0];
  if (!selected) return null;

  return (
    <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-card">
      <div
        role="tablist"
        aria-label="Agents"
        className="flex gap-0.5 overflow-x-auto border-b border-line px-1.5 pt-1.5"
      >
        {agents.map((agent) => {
          const label = agentLabel(agent.state);
          const active = agent.id === selected.id;
          return (
            <button
              key={agent.id}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => setPicked(agent.id)}
              title={`${agent.title}: ${label.title}${agent.envNames.length ? `\nenv: ${sortedNames(agent.envNames).join(", ")}` : ""}`}
              className={cx(
                "relative flex h-9 max-w-[240px] shrink-0 items-center gap-2 rounded-t-lg px-3 text-[13px] transition-colors",
                active
                  ? "bg-page font-medium text-ink after:absolute after:inset-x-0 after:-bottom-px after:h-px after:bg-page"
                  : "text-ink-2 hover:bg-subtle hover:text-ink",
              )}
            >
              <StateDot label={label} className={cx(!isAlive(agent.state) && "opacity-50")} />
              <span className={cx("truncate", !isAlive(agent.state) && "text-ink-3")}>
                {agent.title}
              </span>
              {agent.spawnedBy !== null ? (
                <span
                  className="text-[10px] text-tone-amber"
                  role="img"
                  aria-label="Started by the Chief of Staff"
                >
                  ★
                </span>
              ) : null}
              {waiting.includes(agent.id) ? (
                <span className="size-1.5 shrink-0 rounded-full bg-accent" aria-hidden />
              ) : null}
            </button>
          );
        })}
      </div>
      <AgentPanel
        key={selected.id}
        agent={selected}
        path={path}
        approval={approvals.find((a) => a.agentId === selected.id)}
      />
    </div>
  );
}

function AgentPanel({
  agent,
  path,
  approval,
}: {
  agent: Agent;
  path: string;
  approval: Approval | undefined;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const now = useNow();
  const pending = useCos((s) => pendingApproval(s, agent.id));
  const [busy, setBusy] = useState<"stop" | "resume" | null>(null);
  const alive = isAlive(agent.state);
  const shown = pending ?? approval;

  const act = (kind: "stop" | "resume") => {
    setBusy(kind);
    void ui
      .perform(kind === "stop" ? "Couldn't stop the agent" : "Couldn't resume the agent", () =>
        kind === "stop" ? cos.stopAgent(agent.id) : cos.resumeAgent(agent.id),
      )
      .finally(() => setBusy(null));
  };

  return (
    <div role="tabpanel" aria-label={agent.title} className="flex flex-col gap-3 bg-page p-3">
      <div className="flex flex-wrap items-center gap-2 px-1 text-[12px] text-ink-3">
        <StateBadge label={agentLabel(agent.state)} />
        <span>{agent.model ?? "default model"}</span>
        {agent.spawnedBy !== null ? (
          <span className="flex items-center gap-1.5">
            ·
            <ChiefMark />
            started by the Chief of Staff
          </span>
        ) : null}
        <span>· {relativeTime(agent.lastActivityMs ?? agent.startedMs, now)}</span>
        <EnvNames names={agent.envNames} />
        <div className="ml-auto flex items-center gap-1.5">
          {alive ? (
            <Button
              size="sm"
              icon={Square}
              busy={busy === "stop"}
              disabled={!canChange || agent.state === "starting"}
              onClick={() => act("stop")}
            >
              Stop
            </Button>
          ) : null}
          {canResume(agent.state) ? (
            <Button
              size="sm"
              variant="primary"
              icon={Play}
              busy={busy === "resume"}
              disabled={!canChange || !agent.sessionId}
              onClick={() => act("resume")}
            >
              Resume
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            icon={Maximize2}
            onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
          >
            Open
          </Button>
        </div>
      </div>
      {agent.state === "needs_login" ? <LoginBanner agent={agent} /> : null}
      {agent.state === "needs_trust" ? (
        <TrustBanner agent={agent} path={agent.cwd ?? path} />
      ) : null}
      {shown ? <ApprovalBanner approval={shown} /> : null}
      <AgentTerminal agentId={agent.id} alive={alive} autoFocus={false} className="h-[420px]" />
    </div>
  );
}
