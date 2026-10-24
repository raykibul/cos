// The chief's conversation: your messages as bubbles, its turns as
// markdown in the reading serif with compact tool rows, and inbox wakeups
// as a small "N updates from agents" row (docs/chief-of-staff.md).

import {
  ArrowDown,
  Check,
  ChevronRight,
  Inbox,
  RotateCcw,
  SquareTerminal,
  Users,
  Wrench,
  X,
} from "lucide-react";
import { memo, useCallback, useLayoutEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { type AttachedFile, sizeLabel } from "../../api/attachments";
import type { InboxItem, TranscriptMessage } from "../../api/staff";
import {
  type ConversationItem,
  type PendingMessage,
  isCosTool,
  itemsForUpdate,
  toolTitle,
  updateBody,
} from "../../domain/transcript";
import { cx } from "../../ui/cx";
import { clockTime } from "../../ui/format";
import { SentFiles } from "../../ui/attachments";
import { Spinner } from "../../ui/kit";
import { ChiefMark } from "../../screens/sidebar/ProjectSection";

const column = "mx-auto w-full max-w-[740px] px-8";

/**
 * Keeps a scroller pinned to its bottom while the content grows, unless
 * the reader scrolled up; then a button jumps back down.
 */
function useFollowBottom() {
  const scroller = useRef<HTMLDivElement | null>(null);
  const content = useRef<HTMLDivElement | null>(null);
  const follow = useRef(true);
  const [away, setAway] = useState(false);

  /** Back to the bottom, if following. Call after the content changes. */
  const pin = useCallback(() => {
    const el = scroller.current;
    if (el && follow.current) el.scrollTop = el.scrollHeight;
  }, []);

  useLayoutEffect(() => {
    pin();
    const observer = new ResizeObserver(pin);
    if (content.current) observer.observe(content.current);
    return () => observer.disconnect();
  }, [pin]);

  const onScroll = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 32;
    follow.current = bottom;
    setAway(!bottom);
  }, []);

  const jump = useCallback(() => {
    follow.current = true;
    setAway(false);
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  return { scroller, content, away, pin, onScroll, jump };
}

export function Conversation({
  items,
  inbox,
  working,
  jumpSignal,
  empty,
  onRetry,
  onDiscard,
  agentId = null,
}: {
  items: ConversationItem[];
  /** The chief's agent, whose attachments the bubbles show. */
  agentId?: number | null;
  inbox: InboxItem[];
  /** Show that the chief is at work, under its last turn. */
  working: boolean;
  /** Changing it scrolls to the bottom and follows again, as after you send. */
  jumpSignal: number;
  /** What shows when there is nothing yet. */
  empty: React.ReactNode;
  onRetry: (pending: PendingMessage) => void;
  onDiscard: (pending: PendingMessage) => void;
}) {
  const last = items[items.length - 1];
  const { scroller, content, away, pin, onScroll, jump } = useFollowBottom();
  // New messages keep the view at the bottom, unless the reader scrolled up.
  useLayoutEffect(() => pin(), [items, working, pin]);
  useLayoutEffect(() => {
    if (jumpSignal > 0) jump();
  }, [jumpSignal, jump]);
  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scroller}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto"
        role="log"
        aria-label="Conversation with the Chief of Staff"
      >
        <div ref={content} className={cx(column, "pt-6 pb-8")}>
          {items.length === 0 ? empty : null}
          {items.map((item, index) => {
            const previous = items[index - 1];
            const gap = previous ? (item.type === previous.type ? "mt-5" : "mt-7") : "";
            switch (item.type) {
              case "user":
                return (
                  <UserBubble
                    key={item.key}
                    className={gap}
                    text={item.message.text}
                    files={item.message.attachments}
                    agentId={agentId}
                  >
                    {item.message.tsMs ? clockTime(item.message.tsMs).slice(0, 5) : null}
                  </UserBubble>
                );
              case "pending":
                return (
                  <PendingBubble
                    key={item.key}
                    className={gap}
                    pending={item.pending}
                    working={working}
                    onRetry={onRetry}
                    onDiscard={onDiscard}
                  />
                );
              case "turn":
                return <Turn key={item.key} className={gap} messages={item.messages} />;
              case "update":
                return (
                  <UpdateRow
                    key={item.key}
                    className={gap}
                    count={item.count}
                    message={item.message}
                    items={itemsForUpdate(inbox, item)}
                  />
                );
            }
          })}
          {working ? <Working afterTurn={last?.type === "turn"} /> : null}
        </div>
      </div>
      {away ? (
        <button
          type="button"
          onClick={jump}
          className="absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-line bg-raised px-3.5 py-1.5 text-[12px] text-ink-2 shadow-pop hover:text-ink"
        >
          <ArrowDown className="size-3.5" strokeWidth={2} />
          Jump to latest
        </button>
      ) : null}
    </div>
  );
}

function UserBubble({
  text,
  className,
  children,
  files = [],
  agentId = null,
  previews,
}: {
  text: string;
  className?: string;
  children?: React.ReactNode;
  files?: AttachedFile[];
  agentId?: number | null;
  previews?: Record<string, string | null>;
}) {
  return (
    <div className={cx("flex flex-col items-end", className)}>
      <SentFiles files={files} agentId={agentId} previews={previews} />
      {text ? (
        <div className="max-w-[82%] rounded-[18px] rounded-br-md bg-subtle-2/80 px-4 py-2.5 text-[14.5px] leading-[1.55] whitespace-pre-wrap text-ink select-text">
          {text}
        </div>
      ) : null}
      {children ? <div className="mt-1 px-1 text-[11px] text-ink-3">{children}</div> : null}
    </div>
  );
}

function PendingBubble({
  pending,
  working,
  className,
  onRetry,
  onDiscard,
}: {
  pending: PendingMessage;
  working: boolean;
  className?: string;
  onRetry: (pending: PendingMessage) => void;
  onDiscard: (pending: PendingMessage) => void;
}) {
  return (
    <div className={cx(pending.state === "sending" && "opacity-75", className)}>
      <UserBubble
        text={pending.text}
        files={(pending.attachments ?? []).map((a) => ({
          name: a.name,
          path: a.path,
          contentType: a.contentType,
          sizeLabel: sizeLabel(a.size),
        }))}
        previews={Object.fromEntries(
          (pending.attachments ?? []).map((a) => [a.name, a.previewUrl ?? null]),
        )}
      >
        {pending.state === "sending" ? (
          <span className="flex items-center gap-1" role="status">
            <Spinner className="size-3" />
            Sending…
          </span>
        ) : pending.state === "sent" ? (
          <span className="flex items-center gap-1" role="status">
            <Check className="size-3" strokeWidth={2.2} />
            {working ? "Sent · it reads this when it's free" : "Sent"}
          </span>
        ) : (
          <span className="flex items-center gap-2 text-tone-red" role="alert">
            Couldn't send{pending.error ? `: ${pending.error}` : ""}
            <button
              type="button"
              onClick={() => onRetry(pending)}
              className="flex items-center gap-1 rounded px-1 font-medium text-ink-2 hover:bg-subtle hover:text-ink"
            >
              <RotateCcw className="size-3" strokeWidth={2} />
              Retry
            </button>
            <button
              type="button"
              onClick={() => onDiscard(pending)}
              className="flex items-center gap-1 rounded px-1 font-medium text-ink-2 hover:bg-subtle hover:text-ink"
            >
              <X className="size-3" strokeWidth={2} />
              Discard
            </button>
          </span>
        )}
      </UserBubble>
    </div>
  );
}

function ChiefByline({ tsMs }: { tsMs: number }) {
  return (
    <div className="mb-2 flex items-center gap-2 text-[12px] text-ink-3">
      <ChiefMark />
      <span className="font-semibold text-tone-amber">Chief of Staff</span>
      {tsMs ? (
        <time dateTime={new Date(tsMs).toISOString()}>{clockTime(tsMs).slice(0, 5)}</time>
      ) : null}
    </div>
  );
}

/** One turn of the chief's: what it said, and its tool calls as rows. */
const Turn = memo(function Turn({
  messages,
  className,
}: {
  messages: TranscriptMessage[];
  className?: string;
}) {
  const first = messages[0];
  return (
    <article className={className} aria-label="The Chief of Staff">
      <ChiefByline tsMs={first?.tsMs ?? 0} />
      <div className="flex flex-col">
        {messages.map((message, index) => {
          const previous = messages[index - 1];
          const tool = message.kind === "tool";
          const spacing = !previous ? "" : tool && previous.kind === "tool" ? "mt-0.5" : "mt-3";
          return tool ? (
            <ToolRow key={message.n} message={message} className={spacing} />
          ) : (
            <div key={message.n} className={cx("prose-cos chief-prose select-text", spacing)}>
              <Markdown remarkPlugins={[remarkGfm]}>{message.text}</Markdown>
            </div>
          );
        })}
      </div>
    </article>
  );
});

function ToolRow({ message, className }: { message: TranscriptMessage; className?: string }) {
  const [open, setOpen] = useState(false);
  const tool = message.tool ?? { name: "tool", summary: message.text, isError: false };
  const Icon = isCosTool(tool.name) ? Users : tool.name === "Bash" ? SquareTerminal : Wrench;
  return (
    <div className={cx("-mx-2", className)}>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="group flex h-7 w-full items-center gap-2 rounded-lg px-2 text-left text-[12.5px] text-ink-3 hover:bg-subtle/80"
      >
        <Icon
          className={cx("size-3.5 shrink-0", tool.isError ? "text-tone-red" : "text-ink-3")}
          strokeWidth={1.9}
          aria-hidden
        />
        <span className="shrink-0 font-medium text-ink-2">{toolTitle(tool.name)}</span>
        {tool.summary ? (
          <span className="min-w-0 truncate font-mono text-[11.5px]">{tool.summary}</span>
        ) : null}
        {tool.isError ? <span className="shrink-0 text-tone-red">failed</span> : null}
        <ChevronRight
          className={cx(
            "ml-auto size-3.5 shrink-0 opacity-0 transition group-hover:opacity-100",
            open && "rotate-90 opacity-100",
          )}
          strokeWidth={2}
          aria-hidden
        />
      </button>
      {open ? (
        <div className="mx-2 mt-0.5 mb-1.5 rounded-lg bg-subtle px-3 py-2 font-mono text-[12px] leading-relaxed break-words whitespace-pre-wrap text-ink-2 select-text">
          <div className="text-[11px] text-ink-3">{tool.name}</div>
          {tool.summary || "No details."}
        </div>
      ) : null}
    </div>
  );
}

const inboxKinds: Record<string, string> = {
  stop: "Finished a turn",
  exit: "Exited",
  approval: "Waiting for you",
};

/** A wakeup: the reports from agents that the chief was handed. */
function UpdateRow({
  count,
  message,
  items,
  className,
}: {
  count: number;
  message: TranscriptMessage;
  items: InboxItem[];
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const body = updateBody(message.text);
  return (
    <div className={className}>
      <div className="flex items-center gap-3">
        <span className="h-px flex-1 bg-line" />
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] text-ink-3 hover:bg-subtle hover:text-ink-2"
        >
          <Inbox className="size-3.5" strokeWidth={1.8} aria-hidden />
          {count} {count === 1 ? "update" : "updates"} from agents
          <ChevronRight
            className={cx("size-3 transition-transform", open && "rotate-90")}
            strokeWidth={2.2}
            aria-hidden
          />
        </button>
        <span className="h-px flex-1 bg-line" />
      </div>
      {open ? (
        <div className="mt-2 rounded-xl border border-line bg-surface px-4 py-3 text-[13px] shadow-card">
          {items.length > 0 ? (
            <ul className="flex flex-col gap-2.5">
              {items.map((item) => (
                <li key={item.key} className="flex flex-col gap-0.5">
                  <span className="text-[12px] text-ink-3">
                    <span className="font-medium text-ink-2">
                      {item.title ?? (item.agentId !== null ? `Agent ${item.agentId}` : "cos")}
                    </span>
                    {" · "}
                    {inboxKinds[item.kind] ?? item.kind.replaceAll("_", " ")}
                  </span>
                  <span className="leading-snug whitespace-pre-wrap text-ink select-text">
                    {item.text}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <div className="prose-cos text-[13px] select-text">
              <Markdown remarkPlugins={[remarkGfm]}>{body || message.text}</Markdown>
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}

function Working({ afterTurn }: { afterTurn: boolean }) {
  return (
    <div
      className={cx(
        "flex items-center gap-2.5 text-[13px] text-ink-3",
        afterTurn ? "mt-4" : "mt-7",
      )}
      role="status"
    >
      {afterTurn ? null : <ChiefMark />}
      <span className="flex gap-1" aria-hidden>
        <span className="size-1.5 animate-breathe rounded-full bg-tone-amber" />
        <span className="size-1.5 animate-breathe rounded-full bg-tone-amber [animation-delay:200ms]" />
        <span className="size-1.5 animate-breathe rounded-full bg-tone-amber [animation-delay:400ms]" />
      </span>
      Working…
    </div>
  );
}
