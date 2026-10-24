// A workspace's sessions as cards: each says its state, where it is and
// what it said last, and answers what it waits on in place. A click opens
// the session's own page, with its conversation and terminal.

import { ArrowUpRight } from "lucide-react";
import { useMemo } from "react";
import type { Agent } from "../../api/models";
import { useApp, useCos, useNow } from "../../app/context";
import { approvalDetail, approvalHeadline } from "../../domain/approvals";
import { agentLabel, isAlive } from "../../domain/labels";
import { agentNeedsYou, pendingApproval } from "../../store/selectors";
import { EnvNames, LoginHint, TrustBanner } from "../../screens/AgentPage";
import { ApprovalButtons } from "../../screens/shared";
import { ChiefMark } from "../../screens/sidebar/ProjectSection";
import { cx } from "../../ui/cx";
import { relativeTime } from "../../ui/format";
import { StateBadge } from "../../ui/kit";
import { StateIcon } from "../../ui/StateIcon";

/** The ones waiting on you first, then the running ones, then the newest. */
function ordered(agents: Agent[], needsYou: Set<number>): Agent[] {
  const rank = (a: Agent) => (needsYou.has(a.id) ? 0 : isAlive(a.state) ? 1 : 2);
  return [...agents].sort((a, b) => rank(a) - rank(b) || b.startedMs - a.startedMs);
}

export function AgentCards({ agents, path }: { agents: Agent[]; path: string }) {
  const waiting = useCos((s) => agents.filter((a) => agentNeedsYou(s, a)).map((a) => a.id));
  const list = useMemo(() => ordered(agents, new Set(waiting)), [agents, waiting]);
  return (
    <ul aria-label="Sessions" className="flex flex-col gap-3">
      {list.map((agent) => (
        <AgentCard key={agent.id} agent={agent} path={path} />
      ))}
    </ul>
  );
}

function AgentCard({ agent, path }: { agent: Agent; path: string }) {
  const { ui } = useApp();
  const now = useNow();
  const approval = useCos((s) => pendingApproval(s, agent.id));
  const label = agentLabel(agent.state);
  const open = () => ui.navigate({ kind: "agent", id: agent.id });
  return (
    <li>
      <article
        aria-label={agent.title}
        className={cx(
          "group relative flex flex-col gap-2 rounded-xl border bg-surface px-4 py-3.5 shadow-card transition-colors hover:bg-raised",
          label.attention ? "border-accent/35" : "border-line hover:border-line-strong",
        )}
      >
        <div className="flex min-w-0 items-center gap-2.5">
          <StateIcon state={agent.state} />
          {/* The title's button covers the card, so the whole card opens the session. */}
          <button
            type="button"
            onClick={open}
            className="min-w-0 truncate text-left text-[14px] font-medium text-ink outline-none after:absolute after:inset-0 after:rounded-xl focus-visible:underline"
          >
            {agent.title}
          </button>
          <StateBadge label={label} />
          <span className="ml-auto flex shrink-0 items-center gap-1.5 text-[12px] text-ink-3">
            {agent.spawnedBy !== null ? (
              <span
                role="img"
                aria-label="Started by the Chief of Staff"
                title="Started by the Chief of Staff"
              >
                <ChiefMark size="xs" />
              </span>
            ) : null}
            <span>{agent.model ?? "default model"}</span>
            <span>· {relativeTime(agent.lastActivityMs ?? agent.startedMs, now)}</span>
            <ArrowUpRight
              className="size-3.5 opacity-0 transition-opacity group-hover:opacity-100"
              strokeWidth={2}
              aria-hidden
            />
          </span>
        </div>
        {agent.lastMessage ? (
          <p className="line-clamp-2 text-[13px] leading-snug text-ink-2">{agent.lastMessage}</p>
        ) : null}
        {agent.envNames.length > 0 ? (
          <div className="relative z-10 flex w-fit items-center text-[12px] text-ink-3">
            <EnvNames names={agent.envNames} bare />
          </div>
        ) : null}
        {approval ? (
          <div className="relative z-10 flex items-center gap-3 border-t border-line pt-2.5">
            <div className="min-w-0 flex-1">
              <p className="text-[12.5px] text-ink">{approvalHeadline(approval)}</p>
              <p
                className="truncate font-mono text-[12px] text-ink-2"
                title={approvalDetail(approval)}
              >
                {approvalDetail(approval)}
              </p>
            </div>
            <ApprovalButtons approval={approval} size="sm" />
          </div>
        ) : null}
        {agent.state === "needs_trust" ? (
          <div className="relative z-10">
            <TrustBanner agent={agent} path={agent.cwd ?? path} />
          </div>
        ) : null}
        {agent.state === "needs_login" ? (
          <div className="relative z-10 border-t border-line pt-2.5">
            <LoginHint agent={agent} />
          </div>
        ) : null}
      </article>
    </li>
  );
}
