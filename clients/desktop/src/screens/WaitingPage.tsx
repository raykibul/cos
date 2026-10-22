import { CheckCircle2, ClipboardList, ShieldQuestion, TerminalSquare } from "lucide-react";
import { useState } from "react";
import { type Agent, type Approval, type Run, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { approvalDetail, approvalHeadline, routeExplanation } from "../domain/approvals";
import { pluralize } from "../domain/labels";
import {
  agent as agentOf,
  approvalsForYou,
  pendingDevices,
  plansAwaitingApproval,
  trustAgents,
  waitingCount,
  workspace as workspaceOf,
} from "../store/selectors";
import { relativeTime } from "../ui/format";
import { Button, EmptyState, PathText, SectionTitle } from "../ui/kit";
import { PageBody, PageHeader, PageTitle } from "../ui/Page";
import { DeviceRequestCard } from "./Devices";
import { ApprovalButtons, QuestionCard } from "./shared";

/**
 * Everything blocked on you, oldest first: plans to approve, chiefs'
 * questions, permission requests routed to you, and trust dialogs. Requests
 * routed to a chief aren't here; they're on their agent and in their run's
 * activity.
 */
export function WaitingPage() {
  const plans = useCos(plansAwaitingApproval);
  const questions = useCos((s) => s.questions);
  const approvals = useCos(approvalsForYou);
  const trusting = useCos(trustAgents);
  const devices = useCos(pendingDevices);
  const count = useCos(waitingCount);

  return (
    <>
      <PageHeader>
        <span className="text-ink-2">Waiting on you</span>
      </PageHeader>
      <PageBody>
        <PageTitle
          subtitle={
            count === 0
              ? "Plans to approve, chiefs' questions and permission requests show up here."
              : `${pluralize(count, "thing")} can't go on until you answer.`
          }
        >
          Waiting on you
        </PageTitle>

        {count === 0 ? (
          <div className="rounded-2xl border border-dashed border-line-strong">
            <EmptyState icon={CheckCircle2} title="Nothing needs you right now">
              When an agent asks for permission, a chief has a question or a plan is ready, it
              appears here and in the menu bar.
            </EmptyState>
          </div>
        ) : null}

        {plans.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Plans to review</SectionTitle>
            <div className="flex flex-col gap-3">
              {plans.map((run) => (
                <PlanCard key={run.id} run={run} />
              ))}
            </div>
          </section>
        ) : null}

        {questions.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Questions from chiefs</SectionTitle>
            <div className="flex flex-col gap-3">
              {questions.map((question) => (
                <QuestionCard key={question.id} question={question} />
              ))}
            </div>
          </section>
        ) : null}

        {approvals.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Permission requests</SectionTitle>
            <div className="flex flex-col gap-3">
              {approvals.map((approval) => (
                <ApprovalCard key={approval.id} approval={approval} />
              ))}
            </div>
          </section>
        ) : null}

        {trusting.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Folders to trust</SectionTitle>
            <div className="flex flex-col gap-3">
              {trusting.map((agent) => (
                <TrustCard key={agent.id} agent={agent} />
              ))}
            </div>
          </section>
        ) : null}

        {devices.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Devices asking to reach this Mac</SectionTitle>
            <div className="flex flex-col gap-3">
              {devices.map((device) => (
                <DeviceRequestCard key={device.id} device={device} />
              ))}
            </div>
          </section>
        ) : null}
      </PageBody>
    </>
  );
}

function Meta({
  agent,
  workspaceId,
  since,
}: {
  agent?: Agent;
  workspaceId: number | null;
  since: number;
}) {
  const { ui } = useApp();
  const now = useNow();
  const ws = useCos((s) => workspaceOf(s, workspaceId ?? agent?.workspaceId));
  return (
    <div className="flex min-w-0 items-center gap-1.5 text-[12.5px] text-ink-3">
      {agent ? (
        <button
          type="button"
          className="truncate font-medium text-ink-2 hover:text-ink hover:underline"
          onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
        >
          {agent.title}
        </button>
      ) : null}
      {agent && agent.role !== "user" ? <span>· {agent.role}</span> : null}
      {ws ? <span className="truncate">· in {workspaceName(ws)}</span> : null}
      <span className="ml-auto shrink-0">{relativeTime(since, now)}</span>
    </div>
  );
}

function PlanCard({ run }: { run: Run }) {
  const { ui } = useApp();
  const now = useNow();
  const ws = useCos((s) => workspaceOf(s, run.workspaceId));
  const tasks = run.counts?.total ?? 0;
  return (
    <article className="flex items-center gap-4 rounded-xl border border-line bg-surface p-4 shadow-card">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-accent/12 text-accent-ink">
        <ClipboardList className="size-[18px]" strokeWidth={1.8} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[12.5px] text-ink-3">
          The chief's plan is ready{tasks ? ` · ${pluralize(tasks, "task")}` : ""}
          {ws ? ` · in ${workspaceName(ws)}` : ""} · {relativeTime(run.updatedAt, now)}
        </p>
        <p className="mt-0.5 truncate text-[14.5px] font-medium text-ink">{run.goal}</p>
      </div>
      <Button variant="primary" onClick={() => ui.navigate({ kind: "run", id: run.id })}>
        Review plan
      </Button>
    </article>
  );
}

export function ApprovalCard({ approval }: { approval: Approval }) {
  const agent = useCos((s) => agentOf(s, approval.agentId));
  const why = routeExplanation(approval);
  return (
    <article className="rounded-xl border border-line bg-surface p-4 shadow-card">
      <Meta agent={agent} workspaceId={approval.workspaceId} since={approval.createdMs} />
      <div className="mt-2.5 flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-2 text-[13.5px] font-medium text-ink">
            <TerminalSquare className="size-4 text-ink-3" strokeWidth={1.8} />
            {approvalHeadline(approval)}
          </p>
          <pre className="mt-2 overflow-x-auto rounded-lg bg-subtle px-3 py-2 font-mono text-[12.5px] leading-relaxed whitespace-pre-wrap text-ink select-text">
            {approvalDetail(approval)}
          </pre>
          {why ? <p className="mt-2 text-[12.5px] leading-snug text-ink-3">{why}</p> : null}
        </div>
      </div>
      <div className="mt-3 flex justify-end">
        <ApprovalButtons approval={approval} />
      </div>
    </article>
  );
}

function TrustCard({ agent }: { agent: Agent }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const ws = useCos((s) => workspaceOf(s, agent.workspaceId));
  const [busy, setBusy] = useState(false);
  return (
    <article className="rounded-xl border border-line bg-surface p-4 shadow-card">
      <Meta
        agent={agent}
        workspaceId={agent.workspaceId}
        since={agent.lastActivityMs ?? agent.startedMs}
      />
      <div className="mt-2.5 flex items-center gap-4">
        <ShieldQuestion className="size-5 shrink-0 text-accent" strokeWidth={1.7} />
        <div className="min-w-0 flex-1">
          <p className="text-[13.5px] font-medium text-ink">
            Claude asks whether to trust this folder
          </p>
          {ws ? <PathText path={agent.cwd ?? ws.path} /> : null}
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
      </div>
    </article>
  );
}
