// Pieces several pages share: approval buttons, question cards and agent rows.

import { MessageCircleQuestion, SendHorizontal } from "lucide-react";
import { useState } from "react";
import { isConflict } from "../api/errors";
import type { Agent, Approval, Question } from "../api/models";
import { useApp, useCanChange, useCos, useNow } from "../app/context";
import { approvalDetail } from "../domain/approvals";
import { agentLabel } from "../domain/labels";
import { run as runOf } from "../store/selectors";
import { cx } from "../ui/cx";
import { relativeTime } from "../ui/format";
import { AutoTextarea, Button, StateBadge } from "../ui/kit";

/** Deny and Allow. A request already answered in the terminal just goes away. */
export function ApprovalButtons({
  approval,
  size = "md",
}: {
  approval: Approval;
  size?: "sm" | "md";
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState<"allow" | "deny" | null>(null);
  const decide = (decision: "allow" | "deny") => {
    setBusy(decision);
    void ui
      .perform("Couldn't answer the request", () =>
        cos.decide(approval, decision).catch((error: unknown) => {
          // Answered in the terminal meanwhile; nothing left to do.
          if (!isConflict(error)) throw error;
        }),
      )
      .finally(() => setBusy(null));
  };
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Button
        size={size}
        disabled={!canChange || busy !== null}
        busy={busy === "deny"}
        onClick={() => decide("deny")}
      >
        Deny
      </Button>
      <Button
        size={size}
        variant="accent"
        disabled={!canChange || busy !== null}
        busy={busy === "allow"}
        onClick={() => decide("allow")}
      >
        Allow
      </Button>
    </div>
  );
}

/** A chief's question with its choices and a field for your own answer. */
export function QuestionCard({
  question,
  showRun = true,
}: {
  question: Question;
  showRun?: boolean;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const now = useNow();
  const run = useCos((s) => (question.runId !== null ? runOf(s, question.runId) : undefined));
  const [answer, setAnswer] = useState("");
  const [sending, setSending] = useState(false);
  const trimmed = answer.trim();

  const send = (text: string) => {
    if (!text) return;
    setSending(true);
    void ui
      .perform("Couldn't answer the question", () =>
        cos.answer(question, text).catch((error: unknown) => {
          if (!isConflict(error)) throw error;
        }),
      )
      .finally(() => setSending(false));
  };

  return (
    <article className="rounded-xl border border-line bg-surface p-4 shadow-card">
      <div className="flex items-center gap-2 text-[12.5px] text-ink-3">
        <MessageCircleQuestion className="size-4 text-accent" strokeWidth={1.8} />
        {showRun && run ? (
          <span className="min-w-0 truncate">
            The chief of{" "}
            <button
              type="button"
              className="font-medium text-ink-2 hover:text-ink hover:underline"
              onClick={() => ui.navigate({ kind: "run", id: run.id })}
            >
              {run.goal}
            </button>{" "}
            asks
          </span>
        ) : (
          <span>The chief asks</span>
        )}
        <span className="ml-auto shrink-0">{relativeTime(question.createdMs, now)}</span>
      </div>
      <p className="mt-2 text-[14.5px] leading-relaxed text-ink select-text">{question.question}</p>
      {question.choices.length > 0 ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {question.choices.map((choice) => (
            <Button key={choice} disabled={!canChange || sending} onClick={() => send(choice)}>
              {choice}
            </Button>
          ))}
        </div>
      ) : null}
      <div className="mt-3 flex items-end gap-2 rounded-xl border border-line-strong bg-page/60 py-1.5 pr-1.5 pl-3 focus-within:border-accent/50 focus-within:ring-3 focus-within:ring-accent/12">
        <AutoTextarea
          aria-label="Your answer"
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              send(trimmed);
              setAnswer("");
            }
          }}
          placeholder={question.choices.length ? "Or answer in your own words" : "Your answer"}
          maxRows={6}
          className="min-h-[22px] flex-1 bg-transparent py-0.5 text-[13.5px] outline-none placeholder:text-ink-3"
        />
        <button
          type="button"
          aria-label="Send answer"
          disabled={!canChange || sending || !trimmed}
          onClick={() => {
            send(trimmed);
            setAnswer("");
          }}
          className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-accent text-white transition-opacity disabled:opacity-30"
        >
          <SendHorizontal className="size-3.5" strokeWidth={2} />
        </button>
      </div>
    </article>
  );
}

/** One agent in a list: state, title, details and the last thing it said. */
export function AgentListRow({ agent, approval }: { agent: Agent; approval?: Approval }) {
  const { ui } = useApp();
  const now = useNow();
  const label = agentLabel(agent.state);
  return (
    <button
      type="button"
      onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
      className="group flex w-full flex-col gap-1 px-4 py-3 text-left transition-colors hover:bg-subtle/60"
    >
      <div className="flex w-full items-center gap-2.5">
        <span className="truncate text-[13.5px] font-medium text-ink">{agent.title}</span>
        <StateBadge label={label} />
        <span className="ml-auto shrink-0 text-[12px] text-ink-3">
          {[agent.role !== "user" ? agent.role : null, agent.model ?? "default model"]
            .filter(Boolean)
            .join(" · ")}
          {" · "}
          {relativeTime(agent.lastActivityMs ?? agent.startedMs, now)}
        </span>
      </div>
      {approval ? (
        <p className="truncate font-mono text-[12px] text-accent-ink">
          Wants to run: {approvalDetail(approval)}
        </p>
      ) : agent.lastMessage ? (
        <p className={cx("line-clamp-2 text-[12.5px] leading-snug text-ink-2")}>
          {agent.lastMessage}
        </p>
      ) : null}
    </button>
  );
}
