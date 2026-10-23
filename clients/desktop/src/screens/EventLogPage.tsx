import { ScrollText } from "lucide-react";
import { useMemo, useState } from "react";
import type { LoggedEvent } from "../api/models";
import { useCos, useNow } from "../app/context";
import type { Tone } from "../domain/approvals";
import { summarize } from "../domain/events";
import { cx } from "../ui/cx";
import { clockTime, duration } from "../ui/format";
import { EmptyState } from "../ui/kit";
import { PageBody, PageHeader, PageTitle } from "../ui/Page";
import { toneClasses } from "../ui/tones";

const filters = [
  { key: "all", title: "All" },
  { key: "agent", title: "Agents" },
  { key: "approval", title: "Approvals" },
  { key: "staff", title: "Chief of Staff" },
  { key: "project", title: "Projects" },
] as const;

type Filter = (typeof filters)[number]["key"];

function matches(kind: string, filter: Filter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "agent":
      return kind.startsWith("agent.");
    case "approval":
      return kind.startsWith("approval.");
    case "staff":
      return kind.startsWith("staff.");
    case "project":
      return /^(project|workspace)\./.test(kind);
  }
}

function toneOf(kind: string): Tone {
  if (kind.startsWith("approval.")) return "accent";
  if (kind.startsWith("agent.")) return "blue";
  if (kind.startsWith("staff.")) return "purple";
  if (/^(project|workspace)\./.test(kind)) return "teal";
  if (kind === "daemon.stopped") return "red";
  if (kind.startsWith("daemon.")) return "green";
  return "neutral";
}

export function EventLogPage() {
  const events = useCos((s) => s.events);
  const status = useCos((s) => s.status);
  const info = useCos((s) => s.info);
  const [filter, setFilter] = useState<Filter>("all");
  const now = useNow(10_000);
  const shown = useMemo(
    () => events.filter((e) => matches(e.kind, filter)).reverse(),
    [events, filter],
  );

  return (
    <>
      <PageHeader>
        <span className="text-ink-2">Event log</span>
      </PageHeader>
      <PageBody wide>
        <PageTitle
          subtitle={
            status ? (
              <span className="font-mono text-[12px]">
                pid {status.pid} · up {duration(status.startedAt, now)} · {status.events} events · v
                {status.version} · {status.home || info?.home}
              </span>
            ) : (
              "Connecting to cos…"
            )
          }
        >
          Event log
        </PageTitle>

        <div className="mb-3 flex items-center gap-1" role="tablist" aria-label="Filter events">
          {filters.map((f) => (
            <button
              key={f.key}
              type="button"
              role="tab"
              aria-selected={filter === f.key}
              onClick={() => setFilter(f.key)}
              className={cx(
                "h-7 rounded-full px-3 text-[12.5px] transition-colors",
                filter === f.key ? "bg-ink text-page" : "text-ink-2 hover:bg-subtle hover:text-ink",
              )}
            >
              {f.title}
            </button>
          ))}
          <span className="ml-auto text-[12px] text-ink-3">Newest first · live</span>
        </div>

        {shown.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-line-strong">
            <EmptyState icon={ScrollText} title="No events yet">
              Events appear here as the daemon writes them.
            </EmptyState>
          </div>
        ) : (
          <ol className="overflow-hidden rounded-xl border border-line bg-surface shadow-card">
            {shown.map((event) => (
              <EventRow key={event.seq} event={event} />
            ))}
          </ol>
        )}
      </PageBody>
    </>
  );
}

function EventRow({ event }: { event: LoggedEvent }) {
  const tone = toneClasses[toneOf(event.kind)];
  return (
    <li className="flex items-baseline gap-3 border-b border-line px-4 py-2 text-[12.5px] last:border-b-0 animate-fade-in">
      <span className="w-10 shrink-0 text-right font-mono text-[11.5px] text-ink-3 tabular-nums">
        {event.seq}
      </span>
      <span className="w-[62px] shrink-0 font-mono text-[11.5px] text-ink-3 tabular-nums">
        {event.ts ? clockTime(event.ts) : ""}
      </span>
      <span
        className={cx("w-[168px] shrink-0 truncate font-mono text-[12px] font-medium", tone.text)}
      >
        {event.kind}
      </span>
      <span
        className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-2 select-text"
        title={summarize(event)}
      >
        {summarize(event)}
      </span>
    </li>
  );
}
