import {
  CheckCircle2,
  KeyRound,
  ShieldQuestion,
  SquareTerminal,
  TerminalSquare,
} from "lucide-react";
import { useState } from "react";
import { type Agent, type Approval, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { approvalDetail, approvalHeadline, routeExplanation } from "../domain/approvals";
import { pluralize } from "../domain/labels";
import {
  agent as agentOf,
  approvalsForYou,
  loginAgents,
  pendingDevices,
  trustAgents,
  waitingCount,
  workspace as workspaceOf,
} from "../store/selectors";
import { relativeTime } from "../ui/format";
import { Button, EmptyState, PathText, SectionTitle } from "../ui/kit";
import { PageBody, PageHeader, PageTitle } from "../ui/Page";
import { LoginHint } from "./AgentPage";
import { DeviceRequestCard } from "./Devices";
import { ApprovalButtons } from "./shared";

/**
 * Everything blocked on you, oldest first: permission requests, trust
 * dialogs, Claude login screens and devices asking to reach this Mac.
 */
export function WaitingPage() {
  const approvals = useCos(approvalsForYou);
  const trusting = useCos(trustAgents);
  const logins = useCos(loginAgents);
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
              ? "Permission requests and trust dialogs show up here."
              : `${pluralize(count, "thing")} can't go on until you answer.`
          }
        >
          Waiting on you
        </PageTitle>

        {count === 0 ? (
          <div className="rounded-2xl border border-dashed border-line-strong">
            <EmptyState icon={CheckCircle2} title="Nothing needs you right now">
              When an agent asks for permission or Claude asks to trust a folder, it appears here
              and in the menu bar.
            </EmptyState>
          </div>
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

        {logins.length > 0 ? (
          <section className="mb-8">
            <SectionTitle>Claude logins</SectionTitle>
            <div className="flex flex-col gap-3">
              {logins.map((agent) => (
                <LoginCard key={agent.id} agent={agent} />
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

/** An agent on Claude's login screen: its CLAUDE_CONFIG_DIR isn't logged in. */
function LoginCard({ agent }: { agent: Agent }) {
  const { ui } = useApp();
  return (
    <article className="rounded-xl border border-line bg-surface p-4 shadow-card">
      <Meta
        agent={agent}
        workspaceId={agent.workspaceId}
        since={agent.lastActivityMs ?? agent.startedMs}
      />
      <div className="mt-2.5 flex items-center gap-4">
        <KeyRound className="size-5 shrink-0 text-accent" strokeWidth={1.7} />
        <div className="min-w-0 flex-1">
          <p className="text-[13.5px] font-medium text-ink">Claude needs you to log in</p>
          <LoginHint agent={agent} />
        </div>
        <Button
          variant="accent"
          icon={SquareTerminal}
          onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
        >
          Open terminal
        </Button>
      </div>
    </article>
  );
}
