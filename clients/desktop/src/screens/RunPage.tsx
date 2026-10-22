import {
  ArrowUpRight,
  Ban,
  CircleAlert,
  ClipboardCheck,
  Crown,
  FileText,
  GitBranch,
  ListChecks,
  MoreHorizontal,
  RotateCcw,
  SendHorizontal,
  SlidersHorizontal,
  SquareTerminal,
  X,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { isNotFound, messageOf } from "../api/errors";
import type { Agent, Approval, Run, RunTask } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { approvalDetail, approvalOutcome, approvalTone } from "../domain/approvals";
import { agentLabel, isAlive, pluralize, presetTitle, runLabel, taskLabel } from "../domain/labels";
import {
  countsSummary,
  isRunFinished,
  progress,
  scopeProblems,
  type TaskActionKind,
  taskActionAllowed,
  testPassed,
  testTitle,
} from "../domain/tasks";
import {
  agent as agentOf,
  agentsForRun,
  approvalsForRun,
  project as projectOf,
  questionsFor,
  run as runOf,
  workspace as workspaceOf,
} from "../store/selectors";
import { AgentTerminal } from "../terminal/AgentTerminal";
import { cx } from "../ui/cx";
import { relativeTime } from "../ui/format";
import {
  AutoTextarea,
  Button,
  EmptyState,
  IconButton,
  Pill,
  SectionTitle,
  Spinner,
  StateBadge,
  StateDot,
} from "../ui/kit";
import { type MenuItem, Menu } from "../ui/Menu";
import { Crumb, CrumbSep, PageBody, PageHeader, PageTitle } from "../ui/Page";
import { toneClasses } from "../ui/tones";
import { folderItems } from "./common";
import { QuestionCard } from "./shared";

export function RunPage({ runId }: { runId: number }) {
  const { cos } = useApp();
  // Boards, limits and the report are fetched only while a page shows the run.
  useEffect(() => cos.watchRun(runId), [cos, runId]);
  const run = useCos((s) => runOf(s, runId));
  const error = useCos((s) => s.boardErrors[runId] ?? null);

  if (!run) {
    return (
      <>
        <PageHeader />
        <EmptyState
          icon={error ? CircleAlert : ListChecks}
          title={error ? "This run couldn't be loaded" : "Loading…"}
          className="flex-1"
        >
          {error}
        </EmptyState>
      </>
    );
  }
  return <RunView run={run} />;
}

function RunView({ run }: { run: Run }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const now = useNow();
  const workspace = useCos((s) => workspaceOf(s, run.workspaceId));
  const project = useCos((s) => (workspace ? projectOf(s, workspace.projectId) : undefined));
  const board = useCos((s) => s.boards[run.id]);
  const boardError = useCos((s) => s.boardErrors[run.id] ?? null);
  const questions = useCos((s) => questionsFor(s, run.id));
  const chief = useCos((s) => agentOf(s, run.chiefAgentId));
  const [terminal, setTerminal] = useState<number | null>(null);
  const finished = isRunFinished(run.state);
  const label = runLabel(run.state);
  const tasks = board?.tasks ?? [];
  // Tasks are numbered from 1 in board order, and dependencies by those numbers.
  const numbers = new Map(tasks.map((t, i) => [t.id, i + 1]));

  const menu: MenuItem[] = [
    {
      label: "Change limits…",
      icon: SlidersHorizontal,
      disabled: !canChange || finished,
      onSelect: () => ui.open({ kind: "limits", runId: run.id }),
    },
    ...(workspace ? folderItems(cos, workspace.path) : []),
  ];

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <PageHeader
        actions={
          <>
            {chief ? (
              <Button icon={Crown} onClick={() => setTerminal(chief.id)}>
                Chief
              </Button>
            ) : null}
            {!finished ? (
              <Button
                variant="danger"
                icon={Ban}
                disabled={!canChange}
                onClick={() => ui.open({ kind: "cancelRun", runId: run.id })}
              >
                Cancel run
              </Button>
            ) : null}
            <Menu
              align="end"
              items={menu}
              trigger={(props) => (
                <button
                  type="button"
                  aria-label="Run actions"
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
        <Crumb>Run {run.id}</Crumb>
      </PageHeader>

      <PageBody wide>
        <PageTitle
          badge={<StateBadge label={label} />}
          subtitle={
            <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
              {run.counts && run.counts.total > 0 ? <span>{countsSummary(run.counts)}</span> : null}
              <span className="text-ink-3">started {relativeTime(run.createdAt, now)}</span>
              {workspace ? (
                <span className="flex items-center gap-1 font-mono text-[12px] text-ink-3">
                  <GitBranch className="size-3.5" strokeWidth={1.8} />
                  {workspace.branch}
                </span>
              ) : null}
            </span>
          }
        >
          {run.goal}
        </PageTitle>

        {run.counts &&
        run.counts.total > 0 &&
        (run.state === "running" || run.state === "reporting") ? (
          <div className="-mt-4 mb-7 h-1 w-full overflow-hidden rounded-full bg-subtle">
            <div
              className={cx(
                "h-full rounded-full transition-[width] duration-500",
                toneClasses[label.tone].edge,
              )}
              style={{ width: `${Math.round(progress(run.counts) * 100)}%` }}
            />
          </div>
        ) : null}

        <StateNotice
          run={run}
          taskCount={tasks.length}
          onChief={() => chief && setTerminal(chief.id)}
        />

        {boardError ? (
          <p
            role="alert"
            className="mb-6 rounded-lg bg-tone-red/10 px-3 py-2 text-[12.5px] text-tone-red"
          >
            {boardError}
          </p>
        ) : null}

        {questions.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>
              {questions.length === 1 ? "The chief asks" : "The chief asks you"}
            </SectionTitle>
            <div className="flex flex-col gap-3">
              {questions.map((q) => (
                <QuestionCard key={q.id} question={q} showRun={false} />
              ))}
            </div>
          </section>
        ) : null}

        <Report run={run} />

        <section className="mb-8">
          <SectionTitle>{board ? pluralize(tasks.length, "task") : "Tasks"}</SectionTitle>
          {!board && !boardError ? (
            <div className="flex justify-center py-8">
              <Spinner />
            </div>
          ) : tasks.length === 0 ? (
            <div className="rounded-2xl border border-dashed border-line-strong">
              <EmptyState icon={ListChecks} title="No tasks yet">
                {run.state === "planning"
                  ? "The chief is reading the code and writing a plan."
                  : "This run has no tasks."}
              </EmptyState>
            </div>
          ) : (
            <ol className="flex flex-col gap-2.5">
              {tasks.map((task) => (
                <TaskCard
                  key={task.id}
                  task={task}
                  run={run}
                  numbers={numbers}
                  onTerminal={setTerminal}
                />
              ))}
            </ol>
          )}
        </section>

        {!finished && run.state !== "planning" ? <MessageChief run={run} /> : null}

        <div className="grid grid-cols-1 gap-8 lg:grid-cols-[1fr_280px]">
          <Activity run={run} onTerminal={setTerminal} />
          <aside className="flex flex-col gap-6">
            <Limits run={run} />
            <People run={run} onTerminal={setTerminal} />
          </aside>
        </div>
      </PageBody>

      {terminal !== null ? (
        <TerminalPanel agentId={terminal} onClose={() => setTerminal(null)} />
      ) : null}
    </div>
  );
}

/** What the run waits on, in one line, with the action it needs. */
function StateNotice({
  run,
  taskCount,
  onChief,
}: {
  run: Run;
  taskCount: number;
  onChief: () => void;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState(false);

  if (run.state === "awaiting_plan_approval") {
    return (
      <div className="mb-8 flex items-center gap-4 rounded-xl border border-accent/30 bg-accent/8 px-4 py-3.5 animate-fade-in">
        <ClipboardCheck className="size-5 shrink-0 text-accent" strokeWidth={1.7} />
        <div className="min-w-0 flex-1">
          <p className="text-[14px] font-medium text-ink">The chief's plan is ready</p>
          <p className="text-[12.5px] text-ink-2">
            {pluralize(taskCount, "task")} below. Approving starts workers on the ones with nothing
            to wait for.
          </p>
        </div>
        <Button
          variant="accent"
          busy={busy}
          disabled={!canChange}
          onClick={() => {
            setBusy(true);
            void ui
              .perform("Couldn't approve the plan", () => cos.approvePlan(run.id))
              .finally(() => setBusy(false));
          }}
        >
          Approve plan
        </Button>
      </div>
    );
  }
  if (run.state === "planning") {
    return (
      <Notice icon={Crown}>
        The chief is planning.{" "}
        <button
          type="button"
          className="font-medium text-ink underline-offset-2 hover:underline"
          onClick={onChief}
        >
          Watch its terminal
        </button>
      </Notice>
    );
  }
  if ((run.state === "failed" || run.state === "cancelled") && run.reason) {
    return (
      <Notice
        icon={run.state === "failed" ? XCircle : Ban}
        tone={run.state === "failed" ? "red" : undefined}
      >
        {run.state === "failed" ? "Failed: " : "Cancelled: "}
        {run.reason}
      </Notice>
    );
  }
  return null;
}

function Notice({
  icon: Icon,
  tone,
  children,
}: {
  icon: typeof Crown;
  tone?: "red";
  children: React.ReactNode;
}) {
  return (
    <div
      className={cx(
        "mb-8 flex items-center gap-3 rounded-xl border px-4 py-3 text-[13px] animate-fade-in",
        tone === "red"
          ? "border-tone-red/25 bg-tone-red/8 text-tone-red"
          : "border-line bg-subtle/60 text-ink-2",
      )}
    >
      <Icon className="size-4 shrink-0" strokeWidth={1.8} />
      <p className="min-w-0 flex-1 select-text">{children}</p>
    </div>
  );
}

const actionTitles: Record<TaskActionKind, string> = {
  retry: "Retry…",
  fail: "Mark failed…",
  cancel: "Cancel task…",
};

function TaskCard({
  task,
  run,
  numbers,
  onTerminal,
}: {
  task: RunTask;
  run: Run;
  /** Each task's number on the board, by id. */
  numbers: Map<number, number>;
  onTerminal: (agentId: number) => void;
}) {
  const { ui } = useApp();
  const canChange = useCanChange();
  const worker = useCos((s) => agentOf(s, task.agentId));
  const reviewer = useCos((s) => agentOf(s, task.review?.agentId));
  const label = taskLabel(task.state);
  const actions = (["retry", "fail", "cancel"] as const).filter((kind) =>
    taskActionAllowed(kind, task, run.state),
  );
  const problems = task.checks?.scope ? scopeProblems(task.checks.scope) : [];
  const tests = task.checks?.tests ?? null;

  return (
    <li className="group relative overflow-hidden rounded-xl border border-line bg-surface shadow-card">
      <span
        className={cx("absolute inset-y-0 left-0 w-[3px]", toneClasses[label.tone].edge)}
        aria-hidden
      />
      <div className="flex items-start gap-3 py-3 pr-3 pl-4">
        <span className="mt-0.5 w-5 shrink-0 text-right font-mono text-[12px] text-ink-3 tabular-nums">
          {numbers.get(task.id) ?? task.position + 1}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="min-w-0 truncate text-[14px] font-medium text-ink">{task.title}</span>
            <StateBadge label={label} />
            {task.retries > 0 ? <span className="text-[12px] text-ink-3">retried</span> : null}
          </div>
          {task.instructions ? (
            <p className="mt-1 line-clamp-2 text-[12.5px] leading-snug text-ink-2 select-text">
              {task.instructions}
            </p>
          ) : null}
          {task.files.length > 0 || task.dependsOn.length > 0 ? (
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {task.files.slice(0, 5).map((file) => (
                <Pill key={file}>{file}</Pill>
              ))}
              {task.files.length > 5 ? (
                <span className="text-[11.5px] text-ink-3">+{task.files.length - 5} more</span>
              ) : null}
              {task.dependsOn.length > 0 ? (
                <span className="text-[11.5px] text-ink-3">
                  after {task.dependsOn.map((d) => numbers.get(d) ?? `#${d}`).join(", ")}
                </span>
              ) : null}
            </div>
          ) : null}

          {tests || problems.length > 0 || task.checks?.commitsAhead != null ? (
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px]">
              {tests ? (
                <span className={testPassed(tests) ? "text-tone-green" : "text-tone-red"}>
                  {testTitle(tests)}
                </span>
              ) : null}
              {problems.length === 0 && task.checks?.scope ? (
                <span className="text-tone-green">Stayed in scope</span>
              ) : null}
              {problems.map((p) => (
                <span key={p} className="text-tone-red">
                  {p}
                </span>
              ))}
              {task.checks?.commitsAhead != null ? (
                <span className="text-ink-3">{pluralize(task.checks.commitsAhead, "commit")}</span>
              ) : null}
            </div>
          ) : null}

          {task.review ? (
            <p className="mt-1.5 text-[12px] text-ink-2">
              <span
                className={
                  task.review.verdict === "approve" ? "text-tone-green" : "text-tone-amber"
                }
              >
                Review: {task.review.verdict.replaceAll("_", " ")}
              </span>
              {task.review.notes ? (
                <span className="text-ink-3"> · {task.review.notes}</span>
              ) : null}
            </p>
          ) : null}

          {task.lastFailure && task.state !== "merged" ? (
            <p className="mt-2 rounded-lg bg-tone-red/8 px-2.5 py-1.5 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-tone-red select-text">
              {task.lastFailure}
            </p>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-1">
          {worker ? <WorkerButton agent={worker} onClick={() => onTerminal(worker.id)} /> : null}
          {reviewer && reviewer.id !== worker?.id ? (
            <WorkerButton agent={reviewer} onClick={() => onTerminal(reviewer.id)} />
          ) : null}
          {actions.length > 0 ? (
            <Menu
              align="end"
              width={200}
              items={actions.map((kind) => ({
                label: actionTitles[kind],
                icon: kind === "retry" ? RotateCcw : kind === "fail" ? XCircle : Ban,
                danger: kind !== "retry",
                disabled: !canChange,
                onSelect: () =>
                  ui.open({ kind: "taskAction", runId: run.id, taskId: task.id, action: kind }),
              }))}
              trigger={(props) => (
                <button
                  type="button"
                  aria-label={`Actions for ${task.title}`}
                  {...props}
                  className="flex size-7 items-center justify-center rounded-md text-ink-3 hover:bg-subtle hover:text-ink"
                >
                  <MoreHorizontal className="size-4" strokeWidth={1.8} />
                </button>
              )}
            />
          ) : null}
        </div>
      </div>
    </li>
  );
}

function WorkerButton({ agent, onClick }: { agent: Agent; onClick: () => void }) {
  const label = agentLabel(agent.state);
  return (
    <button
      type="button"
      onClick={onClick}
      title={`${agent.title}: open its terminal`}
      className="flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] text-ink-2 hover:bg-subtle hover:text-ink"
    >
      <StateDot label={label} />
      <SquareTerminal className="size-3.5" strokeWidth={1.8} />
      {agent.role === "reviewer" ? "Reviewer" : "Worker"}
    </button>
  );
}

function Report({ run }: { run: Run }) {
  const report = useCos((s) => s.reports[run.id]);
  if (!report) {
    if (run.state === "reporting") {
      return (
        <section className="mb-8">
          <SectionTitle>Report</SectionTitle>
          <p className="flex items-center gap-2 text-[13px] text-ink-2">
            <Spinner /> The chief is writing the report.
          </p>
        </section>
      );
    }
    return null;
  }
  if (report.kind === "not_written") {
    if (run.state !== "reporting" && run.state !== "done") return null;
    return (
      <section className="mb-8">
        <SectionTitle>Report</SectionTitle>
        <p className="text-[13px] text-ink-3">
          {run.state === "reporting"
            ? "The chief is writing the report."
            : "No report was written."}
        </p>
      </section>
    );
  }
  if (report.kind === "failed") {
    return (
      <section className="mb-8">
        <SectionTitle>Report</SectionTitle>
        <p className="text-[13px] text-tone-red">{report.message}</p>
      </section>
    );
  }
  return (
    <section className="mb-8">
      <SectionTitle>
        <span className="flex items-center gap-1.5">
          <FileText className="size-4" strokeWidth={1.8} /> Report
        </span>
      </SectionTitle>
      <article className="prose-cos rounded-xl border border-line bg-surface px-6 py-5 shadow-card select-text">
        <Markdown remarkPlugins={[remarkGfm]}>{report.markdown}</Markdown>
      </article>
    </section>
  );
}

function MessageChief({ run }: { run: Run }) {
  const { cos } = useApp();
  const canChange = useCanChange();
  const [text, setText] = useState("");
  const [state, setState] = useState<
    { kind: "idle" } | { kind: "sending" } | { kind: "sent" } | { kind: "error"; message: string }
  >({ kind: "idle" });
  const [unsupported, setUnsupported] = useState(false);
  const trimmed = text.trim();

  const send = () => {
    if (!trimmed || state.kind === "sending") return;
    setState({ kind: "sending" });
    cos.messageChief(run.id, trimmed).then(
      () => {
        setText("");
        setState({ kind: "sent" });
      },
      (error: unknown) => {
        if (isNotFound(error)) setUnsupported(true);
        setState({ kind: "error", message: messageOf(error) });
      },
    );
  };

  if (unsupported) {
    return (
      <p className="mb-8 text-[12.5px] text-ink-3">
        This daemon can't pass messages to the chief. Update cos to talk to it from here.
      </p>
    );
  }

  return (
    <section className="mb-8">
      <SectionTitle>Message the chief</SectionTitle>
      <div className="flex items-end gap-2 rounded-2xl border border-line-strong bg-raised py-2 pr-2 pl-4 shadow-composer focus-within:border-ink-3/50">
        <AutoTextarea
          aria-label="Message to the chief"
          value={text}
          disabled={!canChange}
          maxRows={8}
          onChange={(event) => {
            setText(event.target.value);
            if (state.kind !== "sending") setState({ kind: "idle" });
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              send();
            }
          }}
          placeholder="Steer the run: “skip the docs task”, “use the existing helper”…"
          className="min-h-[24px] flex-1 bg-transparent py-1 text-[14px] text-ink outline-none placeholder:text-ink-3"
        />
        <button
          type="button"
          aria-label="Send to the chief"
          disabled={!canChange || !trimmed || state.kind === "sending"}
          onClick={send}
          className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent text-white hover:bg-accent-strong disabled:bg-accent/35"
        >
          {state.kind === "sending" ? (
            <Spinner className="text-white" />
          ) : (
            <SendHorizontal className="size-4" strokeWidth={2} />
          )}
        </button>
      </div>
      {state.kind === "sent" ? (
        <p className="mt-1.5 px-1 text-[12px] text-tone-green">
          Sent. The chief reads it at its next turn.
        </p>
      ) : state.kind === "error" ? (
        <p className="mt-1.5 px-1 text-[12px] text-tone-red">{state.message}</p>
      ) : null}
    </section>
  );
}

function Limits({ run }: { run: Run }) {
  const { ui } = useApp();
  const canChange = useCanChange();
  const limits = useCos((s) => s.limits[run.id]);
  const supported = useCos((s) => s.limitsSupported);
  const preset = limits?.preset ?? run.preset;
  const cap = limits?.approvalCap ?? run.approvalCap;
  const used = limits?.chiefApprovals ?? null;
  return (
    <section>
      <SectionTitle
        actions={
          supported && !isRunFinished(run.state) ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={!canChange}
              onClick={() => ui.open({ kind: "limits", runId: run.id })}
            >
              Change…
            </Button>
          ) : null
        }
      >
        Limits
      </SectionTitle>
      <dl className="rounded-xl border border-line bg-surface px-4 py-3 text-[12.5px] shadow-card">
        <div className="flex justify-between py-1">
          <dt className="text-ink-3">Preset</dt>
          <dd className="font-medium text-ink">{presetTitle(preset)}</dd>
        </div>
        <div className="flex justify-between py-1">
          <dt className="text-ink-3">Chief approvals</dt>
          <dd className="font-medium text-ink tabular-nums">
            {used !== null ? `${used} of ${cap}` : `up to ${cap}`}
          </dd>
        </div>
        {used !== null && cap > 0 ? (
          <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-subtle">
            <div
              className={cx("h-full rounded-full", used >= cap ? "bg-tone-red" : "bg-ink-3")}
              style={{ width: `${Math.min(100, Math.round((used / cap) * 100))}%` }}
            />
          </div>
        ) : null}
      </dl>
    </section>
  );
}

function People({ run, onTerminal }: { run: Run; onTerminal: (id: number) => void }) {
  const agents = useCos((s) => agentsForRun(s, run.id));
  if (agents.length === 0) return null;
  const order = { chief: 0, worker: 1, reviewer: 2 } as Record<string, number>;
  const sorted = [...agents].sort(
    (a, b) => (order[a.role] ?? 3) - (order[b.role] ?? 3) || a.id - b.id,
  );
  return (
    <section>
      <SectionTitle>Agents</SectionTitle>
      <ul className="overflow-hidden rounded-xl border border-line bg-surface shadow-card">
        {sorted.map((agent) => (
          <li key={agent.id} className="border-b border-line last:border-b-0">
            <button
              type="button"
              onClick={() => onTerminal(agent.id)}
              className="flex w-full items-center gap-2 px-3.5 py-2 text-left text-[12.5px] hover:bg-subtle/60"
            >
              <StateDot label={agentLabel(agent.state)} />
              <span className="min-w-0 flex-1 truncate text-ink">{agent.title}</span>
              <span className="shrink-0 text-ink-3 capitalize">{agent.role}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Every permission request of the run's agents and what became of it. */
function Activity({ run, onTerminal }: { run: Run; onTerminal: (id: number) => void }) {
  const approvals = useCos((s) => approvalsForRun(s, run.id));
  return (
    <section>
      <SectionTitle>Permission requests</SectionTitle>
      {approvals.length === 0 ? (
        <p className="px-1 text-[13px] text-ink-3">
          None yet. Requests the rules don't settle go to the chief or to you, and show up here.
        </p>
      ) : (
        <ul className="overflow-hidden rounded-xl border border-line bg-surface shadow-card">
          {approvals.slice(0, 50).map((approval) => (
            <ActivityRow key={approval.id} approval={approval} onTerminal={onTerminal} />
          ))}
        </ul>
      )}
    </section>
  );
}

function ActivityRow({
  approval,
  onTerminal,
}: {
  approval: Approval;
  onTerminal: (id: number) => void;
}) {
  const now = useNow();
  const agent = useCos((s) => agentOf(s, approval.agentId));
  const tone = toneClasses[approvalTone(approval)];
  return (
    <li className="flex items-center gap-3 border-b border-line px-3.5 py-2 text-[12.5px] last:border-b-0">
      <span className={cx("size-1.5 shrink-0 rounded-full", tone.dot)} aria-hidden />
      <span className="w-[150px] shrink-0 truncate text-ink-2" title={approvalOutcome(approval)}>
        {approvalOutcome(approval)}
      </span>
      <span
        className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink select-text"
        title={approvalDetail(approval)}
      >
        {approval.toolName}: {approvalDetail(approval)}
      </span>
      {agent ? (
        <button
          type="button"
          className="shrink-0 truncate text-ink-3 hover:text-ink hover:underline"
          onClick={() => onTerminal(agent.id)}
        >
          {agent.role}
        </button>
      ) : null}
      <span className="w-[72px] shrink-0 text-right text-ink-3">
        {relativeTime(approval.createdMs, now)}
      </span>
    </li>
  );
}

/** A worker's or the chief's terminal over the right of the run page. */
function TerminalPanel({ agentId, onClose }: { agentId: number; onClose: () => void }) {
  const { ui } = useApp();
  const agent = useCos((s) => agentOf(s, agentId));
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        event.key === "Escape" &&
        event.target instanceof HTMLElement &&
        !event.target.closest(".terminal-host")
      ) {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <aside
      aria-label="Agent terminal"
      className="absolute inset-y-0 right-0 z-20 flex w-[min(760px,62%)] flex-col border-l border-line bg-page shadow-pop animate-fade-in"
    >
      <div className="flex h-[52px] shrink-0 items-center gap-2 px-4" data-tauri-drag-region>
        {agent ? (
          <>
            <span className="truncate text-[13px] font-medium text-ink">{agent.title}</span>
            <StateBadge label={agentLabel(agent.state)} />
          </>
        ) : (
          <span className="text-[13px] text-ink-3">Agent {agentId}</span>
        )}
        <div className="ml-auto flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            icon={ArrowUpRight}
            onClick={() => ui.navigate({ kind: "agent", id: agentId })}
          >
            Open
          </Button>
          <IconButton icon={X} label="Close terminal" onClick={onClose} />
        </div>
      </div>
      <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">
        <AgentTerminal
          key={agentId}
          agentId={agentId}
          alive={agent ? isAlive(agent.state) : false}
          className="flex-1"
        />
      </div>
    </aside>
  );
}
