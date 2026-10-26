// A session the chief started (cos's `agent_spawn`), as a card in its chat,
// as on the iPhone: its title, folder and model, and its live state with its
// last message. Clicking it shows the session beside the chat; "Open in tab"
// gives it a page of its own.

import { ArrowUpRight, SquareTerminal } from "lucide-react";
import { workspaceName } from "../../api/models";
import type { TranscriptMessage } from "../../api/staff";
import { useApp, useCos, useUi } from "../../app/context";
import { agentLabel } from "../../domain/labels";
import { agent as agentOf, workspace as workspaceOf } from "../../store/selectors";
import { cx } from "../../ui/cx";
import { clockTime } from "../../ui/format";
import { StateIcon } from "../../ui/StateIcon";

export function SpawnedCard({
  message,
  chiefId,
  className,
}: {
  message: TranscriptMessage;
  /** The chief's agent, to find the session by title on a daemon that doesn't say its id. */
  chiefId: number | null;
  className?: string;
}) {
  const { ui } = useApp();
  const tool = message.tool;
  const agent = useCos((s) => {
    if (!tool) return undefined;
    if (tool.agentId !== null) return agentOf(s, tool.agentId);
    if (chiefId === null) return undefined;
    return s.agents.find((a) => a.spawnedBy === chiefId && a.title === tool.summary);
  });
  const workspace = useCos((s) => workspaceOf(s, agent?.workspaceId));
  const beside = useUi((s) => agent !== undefined && s.chiefSession === agent.id);
  if (!tool) return null;

  const title = agent?.title ?? (tool.summary || "A new session");
  const label = agent ? agentLabel(agent.state) : null;
  const place = [workspace ? workspaceName(workspace) : null, agent?.model]
    .filter(Boolean)
    .join(" · ");
  const last = agent?.state !== "working" ? agent?.lastMessage?.split("\n")[0]?.trim() : null;
  const time = message.tsMs ? clockTime(message.tsMs).slice(0, 5) : null;

  return (
    <div
      className={cx(
        "group/card my-1 flex max-w-[560px] items-stretch overflow-hidden rounded-xl border bg-surface shadow-card transition-colors",
        beside ? "border-accent/50 ring-2 ring-accent/15" : "border-line hover:border-ink-3/35",
        className,
      )}
    >
      <button
        type="button"
        disabled={!agent}
        aria-pressed={beside}
        aria-label={`Session started: ${title}${label ? `, ${label.title}` : ""}`}
        title={agent ? "Show this session beside the chat" : "cos doesn't know this session"}
        onClick={() => agent && ui.showChiefSession(beside ? null : agent.id)}
        className="flex min-w-0 flex-1 items-start gap-3 px-3.5 py-3 text-left disabled:cursor-default"
      >
        <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-tone-blue/14 text-tone-blue">
          <SquareTerminal className="size-4" strokeWidth={1.8} aria-hidden />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[10.5px] font-semibold tracking-[0.06em] text-ink-3 uppercase">
            Session started{time ? ` · ${time}` : ""}
          </span>
          <span className="truncate text-[14px] font-semibold text-ink">{title}</span>
          {place ? <span className="truncate text-[12.5px] text-ink-2">{place}</span> : null}
          {agent && label ? (
            <span
              className={cx(
                "mt-1 flex min-w-0 items-center gap-1.5 text-[12.5px]",
                label.attention ? "text-accent-ink" : "text-ink-2",
              )}
            >
              <StateIcon state={agent.state} />
              <span className="shrink-0">{label.title}</span>
              {last ? <span className="min-w-0 truncate text-ink-3">· {last}</span> : null}
            </span>
          ) : null}
        </span>
      </button>
      {agent ? (
        <button
          type="button"
          aria-label={`Open ${title} in a tab`}
          title="Open in a tab"
          onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
          className="flex w-10 shrink-0 items-center justify-center border-l border-line text-ink-3 hover:bg-subtle hover:text-ink"
        >
          <ArrowUpRight className="size-4" strokeWidth={1.9} aria-hidden />
        </button>
      ) : null}
    </div>
  );
}
