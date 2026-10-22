// Settings' "Judgment (jev)" section. jev is a fast judgment model cos runs
// in shadow mode: for permission requests that come to you, and for items in
// a chief's inbox, it notes what it would have done and changes nothing.
// Here: where its API key came from, which projects it judges (off by
// default, since what it weighs goes to TypeSafe), and how its verdicts
// compare with what you did. A daemon without `/v1/judge` hides it.

import { KeyRound, Lock, RotateCw, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import {
  type JudgeExample,
  type JudgeOverview,
  type JudgePointStats,
  type JudgeProject,
  keyFound,
  keySourceLabel,
  pointStats,
} from "../api/judge";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { pluralize } from "../domain/labels";
import { cx } from "../ui/cx";
import { relativeTime } from "../ui/format";
import { Button, Card, SectionTitle, Spinner, Switch } from "../ui/kit";

/** How often to ask again while the daemon probes the login shell for the key. */
const PROBE_POLL_MS = 2000;

export function JudgeSettings() {
  const { cos } = useApp();
  const running = useCos((s) => s.connection === "running");
  const supported = useCos((s) => s.judgeSupported);
  const judge = useCos((s) => s.judge);
  const error = useCos((s) => s.judgeError);
  const probing = judge?.key.probing ?? false;

  useEffect(() => {
    if (running) cos.refreshJudge();
  }, [cos, running]);

  // The probe ends with `judge.changed`; asking again covers a daemon that doesn't say.
  useEffect(() => {
    if (!running || !probing) return;
    const timer = setInterval(() => cos.refreshJudge(), PROBE_POLL_MS);
    return () => clearInterval(timer);
  }, [cos, running, probing]);

  if (!supported) return null;

  let body;
  if (judge) {
    body = (
      <>
        <KeyCard judge={judge} />
        <StatsCard judge={judge} />
      </>
    );
  } else if (!running) {
    body = (
      <Card className="px-5 py-4 text-[13px] text-ink-2">
        jev is set up in the cos daemon, which isn't answering right now.
      </Card>
    );
  } else {
    body = (
      <Card className="flex items-center gap-2 px-5 py-4 text-[13px] text-ink-3">
        {error ? <span className="text-tone-red">{error}</span> : <Spinner />}
      </Card>
    );
  }

  return (
    <section className="mb-10" aria-label="Judgment (jev)">
      <SectionTitle>Judgment (jev)</SectionTitle>
      {body}
    </section>
  );
}

/** `api.typesafe.ai`, or the endpoint as given. */
function endpointHost(endpoint: string): string {
  try {
    return new URL(endpoint).host || endpoint;
  } catch {
    return endpoint;
  }
}

function KeyCard({ judge }: { judge: JudgeOverview }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [reloading, setReloading] = useState(false);
  const found = keyFound(judge);
  const reload = () => {
    setReloading(true);
    void ui
      .perform("Couldn't look for the jev key", () => cos.reloadJudge())
      .finally(() => setReloading(false));
  };
  const service = [judge.model, judge.endpoint ? endpointHost(judge.endpoint) : ""]
    .filter(Boolean)
    .join(" · ");

  return (
    <Card className="mb-4">
      <div className="flex items-start gap-3 px-5 py-4">
        <KeyRound
          className={cx("mt-0.5 size-4 shrink-0", found ? "text-tone-green" : "text-ink-3")}
          strokeWidth={1.8}
          aria-hidden
        />
        <div className="min-w-0 flex-1" aria-label="jev key" role="group">
          <p className="text-[14px] font-medium text-ink">
            {found ? `Key found in ${keySourceLabel(judge.key.source)}` : "No key"}
          </p>
          {!found || !judge.active ? (
            <p className="mt-0.5 text-[12.5px] leading-snug text-ink-3 select-text">
              {judge.offReason ?? "jev is off until TYPESAFEAI_API_KEY is set."}
            </p>
          ) : service ? (
            <p className="mt-0.5 font-mono text-[12px] text-ink-3">{service}</p>
          ) : null}
          {judge.key.detail ? (
            <p className="mt-2 flex gap-2 text-[12.5px] leading-snug text-ink-2">
              <TriangleAlert
                className="mt-0.5 size-3.5 shrink-0 text-tone-amber"
                strokeWidth={1.8}
                aria-hidden
              />
              <span className="select-text">{judge.key.detail}</span>
            </p>
          ) : null}
          {judge.key.probing ? (
            <p className="mt-2 flex items-center gap-2 text-[12.5px] text-ink-3">
              <Spinner className="size-3.5" />
              Checking your login shell…
            </p>
          ) : null}
        </div>
        <Button size="sm" icon={RotateCw} busy={reloading} disabled={!canChange} onClick={reload}>
          Check again
        </Button>
      </div>
      <p className="flex gap-2 border-t border-line px-5 py-3 text-[12.5px] leading-snug text-ink-3">
        <Lock className="mt-0.5 size-3.5 shrink-0" strokeWidth={1.8} aria-hidden />
        <span>
          Shadow mode: jev only notes what it would have done and never changes what happens. For
          projects you switch on, what it weighs (commands, paths, task titles) goes to TypeSafe.
        </span>
      </p>
      <ProjectSwitches judge={judge} />
    </Card>
  );
}

function ProjectSwitches({ judge }: { judge: JudgeOverview }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState<Record<number, boolean>>({});
  const toggle = (project: JudgeProject, enabled: boolean) => {
    setBusy((b) => ({ ...b, [project.projectId]: true }));
    void ui
      .perform(`Couldn't switch jev for ${project.name}`, () =>
        cos.setProjectJudge(project.projectId, enabled),
      )
      .finally(() => setBusy((b) => ({ ...b, [project.projectId]: false })));
  };

  if (judge.projects.length === 0) {
    return (
      <p className="border-t border-line px-5 py-3 text-[13px] text-ink-3">
        No projects yet. Each one you add shows up here, switched off.
      </p>
    );
  }
  return (
    <div className="border-t border-line">
      <ul className="divide-y divide-line" aria-label="Judged projects">
        {judge.projects.map((project) => (
          <li
            key={project.projectId}
            className="flex items-center gap-3 px-5 py-2.5"
            aria-label={project.name}
          >
            <span className="min-w-0 flex-1 truncate text-[13.5px] text-ink">{project.name}</span>
            <span className="text-[12px] text-ink-3">
              {project.enabled ? (judge.active ? "Judged" : "On, waiting for a key") : "Off"}
            </span>
            <Switch
              on={project.enabled}
              busy={busy[project.projectId] ?? false}
              disabled={!canChange}
              label={`Judge ${project.name}`}
              onChange={(on) => toggle(project, on)}
            />
          </li>
        ))}
      </ul>
    </div>
  );
}

function StatsCard({ judge }: { judge: JudgeOverview }) {
  const permission = pointStats(judge, "permission");
  const wake = pointStats(judge, "wake");
  const others = judge.points.filter((p) => p.point !== "permission" && p.point !== "wake");
  return (
    <Card>
      <PermissionStats stats={permission} />
      <div className="border-t border-line">
        <VerdictStats
          title="Chief inboxes"
          subtitle="What it would have done with each item in a chief's inbox."
          empty="Nothing judged yet. jev weighs in when something lands in the inbox of a switched-on project's chief."
          stats={wake}
          labels={WAKE_LABELS}
        />
      </div>
      {others.map((stats) => (
        <div key={stats.point} className="border-t border-line">
          <VerdictStats title={stats.point} empty="Nothing judged yet." stats={stats} labels={{}} />
        </div>
      ))}
    </Card>
  );
}

/** `52 decisions · 3 no opinion · 1 error`. */
function countsLine(stats: JudgePointStats): string {
  const parts = [pluralize(stats.decisions, "decision"), `${stats.noOpinion} no opinion`];
  if (stats.errors > 0) parts.push(pluralize(stats.errors, "error"));
  return parts.join(" · ");
}

function PermissionStats({ stats }: { stats: JudgePointStats | null }) {
  const empty = !stats || (stats.decisions === 0 && stats.answered === 0);
  return (
    <div className="px-5 py-4" role="group" aria-label="Permission requests">
      <p className="text-[13.5px] font-medium text-ink">Permission requests</p>
      {empty ? (
        <p className="mt-0.5 text-[12.5px] leading-snug text-ink-3">
          Nothing judged yet. jev weighs in when an agent in a switched-on project asks you for a
          permission.
        </p>
      ) : (
        <>
          <p className="mt-1 text-[13px] text-ink-2">
            {stats.answered > 0
              ? `Would have auto-approved ${stats.wouldApprove} of ${stats.answered}, disagreed on ${stats.disagreed}`
              : "None you've answered yet, so nothing to compare."}
          </p>
          {stats.missed > 0 ? (
            <p className="mt-0.5 text-[12.5px] text-ink-3">
              Asked you about {stats.missed} you then allowed
            </p>
          ) : null}
          <p className="mt-0.5 text-[12px] text-ink-3">{countsLine(stats)}</p>
          {stats.disagreements.length > 0 ? <Disagreements examples={stats.disagreements} /> : null}
        </>
      )}
    </div>
  );
}

const WOULD_LABELS: Record<string, string> = { approve: "Would approve", ask_you: "Would ask you" };

const ACTUAL_LABELS: Record<string, string> = {
  denied: "you denied",
  allowed: "you allowed",
  terminal: "answered in the terminal",
  expired: "expired",
};

function Disagreements({ examples }: { examples: JudgeExample[] }) {
  const now = useNow();
  return (
    <div className="mt-3">
      <p className="mb-1 text-[12px] font-medium text-ink-3">Disagreements</p>
      <ul className="divide-y divide-line rounded-lg border border-line" aria-label="Disagreements">
        {examples.map((example) => (
          <li
            key={example.id}
            className="flex flex-col gap-0.5 px-3 py-2"
            aria-label={example.summary}
          >
            <div className="flex items-baseline gap-3">
              <code
                className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-ink select-text"
                title={example.summary}
              >
                {example.summary || example.subject}
              </code>
              <span className="shrink-0 text-[12px] text-ink-3">
                {relativeTime(example.createdMs, now)}
              </span>
            </div>
            <p className="text-[12px] text-ink-2">
              {WOULD_LABELS[example.would] ?? `Would ${example.would}`},{" "}
              {example.actual
                ? (ACTUAL_LABELS[example.actual] ?? example.actual)
                : "not answered yet"}
              {example.why ? (
                <span className="text-ink-3">
                  {" · "}
                  <span className="font-mono text-[11.5px]">{example.why}</span>
                </span>
              ) : null}
            </p>
          </li>
        ))}
      </ul>
    </div>
  );
}

const WAKE_LABELS: Record<string, string> = { now: "Wake now", batch: "Batch", none: "Let it be" };

function VerdictStats({
  title,
  subtitle,
  empty,
  stats,
  labels,
}: {
  title: string;
  subtitle?: string;
  empty: string;
  stats: JudgePointStats | null;
  labels: Record<string, string>;
}) {
  // The known verdicts in their order, even at 0, then any a newer daemon adds.
  const keys = [
    ...Object.keys(labels),
    ...Object.keys(stats?.would ?? {}).filter((k) => !(k in labels)),
  ];
  return (
    <div className="px-5 py-4" role="group" aria-label={title}>
      <p className="text-[13.5px] font-medium text-ink">{title}</p>
      {!stats || stats.decisions === 0 ? (
        <p className="mt-0.5 text-[12.5px] leading-snug text-ink-3">{empty}</p>
      ) : (
        <>
          {subtitle ? <p className="mt-0.5 text-[12.5px] text-ink-3">{subtitle}</p> : null}
          <dl className="mt-3 flex flex-wrap gap-x-8 gap-y-2">
            {keys.map((key) => (
              <div key={key} className="flex flex-col">
                <dt className="text-[12px] text-ink-3">{labels[key] ?? key}</dt>
                <dd className="font-serif text-[20px] leading-tight text-ink tabular-nums">
                  {stats.would[key] ?? 0}
                </dd>
              </div>
            ))}
          </dl>
          <p className="mt-2 text-[12px] text-ink-3">{countsLine(stats)}</p>
        </>
      )}
    </div>
  );
}
