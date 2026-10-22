import { ShieldQuestion } from "lucide-react";
import { useState } from "react";
import { useApp, useCanChange } from "../../app/context";
import { approvalDetail } from "../../domain/approvals";
import type { ChiefRequest } from "../../store/selectors";
import { ApprovalButtons } from "../../screens/shared";
import { Button } from "../../ui/kit";

/**
 * Approvals and trust dialogs of the agents the chief started (and its
 * own), answerable without leaving the conversation.
 */
export function WaitingStrip({
  requests,
  chiefAgentId,
}: {
  requests: ChiefRequest[];
  chiefAgentId: number | null;
}) {
  if (requests.length === 0) return null;
  return (
    <section
      aria-label="Agents waiting on you"
      className="mb-3 overflow-hidden rounded-2xl border border-accent/25 bg-accent/6"
    >
      <h2 className="px-4 pt-2.5 pb-1 text-[11.5px] font-semibold tracking-wide text-accent-ink uppercase">
        {requests.length === 1
          ? "An agent is waiting on you"
          : `${requests.length} agents waiting on you`}
      </h2>
      <ul className="max-h-[188px] divide-y divide-accent/12 overflow-y-auto">
        {requests.map((request) => (
          <Request
            key={`${request.agent.id}-${request.approval?.id ?? "trust"}`}
            request={request}
            isChief={request.agent.id === chiefAgentId}
          />
        ))}
      </ul>
    </section>
  );
}

function Request({ request, isChief }: { request: ChiefRequest; isChief: boolean }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState(false);
  const { agent, approval } = request;
  return (
    <li className="flex items-center gap-3 px-4 py-2">
      <div className="min-w-0 flex-1">
        <button
          type="button"
          onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
          className="block max-w-full truncate text-left text-[13px] font-medium text-ink hover:underline"
        >
          {isChief ? "The Chief of Staff" : agent.title}
        </button>
        <p
          className="truncate font-mono text-[12px] text-ink-2"
          title={approval ? approvalDetail(approval) : undefined}
        >
          {approval ? (
            <>
              <span className="font-sans text-ink-3">
                {approval.toolName === "Bash"
                  ? "wants to run "
                  : `wants to use ${approval.toolName}: `}
              </span>
              {approvalDetail(approval)}
            </>
          ) : (
            <span className="flex items-center gap-1 font-sans text-ink-3">
              <ShieldQuestion className="size-3.5 text-accent" strokeWidth={1.8} />
              asks whether to trust its folder
            </span>
          )}
        </p>
      </div>
      {approval ? (
        <ApprovalButtons approval={approval} size="sm" />
      ) : (
        <Button
          size="sm"
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
      )}
    </li>
  );
}
