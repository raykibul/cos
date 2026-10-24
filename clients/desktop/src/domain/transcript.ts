// The Chief of Staff's conversation, from its transcript: merging pages
// read with `after=next`, reconciling what you typed with what the
// transcript caught up to, and grouping messages into what the page shows.

import type { SentAttachment } from "../api/attachments";
import type { InboxItem, TranscriptMessage } from "../api/staff";

/** Adds a page of messages, dropping ones already there (by `n`), oldest first. */
export function mergeMessages(
  current: TranscriptMessage[],
  incoming: TranscriptMessage[],
): TranscriptMessage[] {
  if (incoming.length === 0) return current;
  const byN = new Map<number, TranscriptMessage>();
  for (const message of current) byN.set(message.n, message);
  for (const message of incoming) byN.set(message.n, message);
  return [...byN.values()].sort((a, b) => a.n - b.n);
}

/** A message you sent that the transcript doesn't show yet. */
export interface PendingMessage {
  id: number;
  text: string;
  sentMs: number;
  /** The transcript's `next` when it was sent: only later messages can be it. */
  afterN: number;
  state: "sending" | "sent" | "failed";
  error?: string;
  /** Files it carries, uploaded already. */
  attachments?: SentAttachment[];
}

function normalize(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/**
 * The pending messages the transcript hasn't caught up with. Each one is
 * matched, in order, to one later user message with the same text. Text
 * typed while the chief worked may arrive inside a wakeup instead, so an
 * update that contains it counts too.
 */
export function reconcilePending(
  pending: PendingMessage[],
  messages: TranscriptMessage[],
): PendingMessage[] {
  if (pending.length === 0) return pending;
  const used = new Set<number>();
  const left: PendingMessage[] = [];
  for (const item of pending) {
    const text = normalize(item.text);
    const match = messages.find(
      (m) =>
        m.n > item.afterN &&
        !used.has(m.n) &&
        m.role === "user" &&
        (m.kind === "update"
          ? normalize(m.text).includes(text)
          : normalize(m.text) === text && sameFiles(item, m)),
    );
    if (match) used.add(match.n);
    else left.push(item);
  }
  return left.length === pending.length ? pending : left;
}

/** The transcript message names the files the pending one carries. */
function sameFiles(item: PendingMessage, message: TranscriptMessage): boolean {
  const sent = (item.attachments ?? []).map((a) => a.name).join("\n");
  const seen = message.attachments.map((a) => a.name).join("\n");
  return sent === seen;
}

/** `[cos · 3 updates]` → 3; one when the header has no count. */
export function updateCount(text: string): number {
  const match = /^\[cos\s*·\s*(\d+)\s+update/i.exec(text.trim());
  return match ? Number(match[1]) : 1;
}

/** An update's text without its `[cos · N updates]` header line. */
export function updateBody(text: string): string {
  return text.replace(/^\[cos[^\]\n]*\]\s*/, "").trim();
}

export type ConversationItem =
  /** `fromChief`: the Chief of Staff wrote it, starting the agent; its `[cos]` line is cut off. */
  | { type: "user"; key: string; message: TranscriptMessage; fromChief?: boolean }
  | { type: "turn"; key: string; messages: TranscriptMessage[]; tsMs: number }
  | { type: "update"; key: string; message: TranscriptMessage; count: number; sinceMs: number }
  | { type: "pending"; key: string; pending: PendingMessage };

/** The line cos appends to the first prompt of an agent the chief starts (`spawned_line` in the daemon). */
const SPAWNED_LINE = /\s*\[cos\] The Chief of Staff of .+? started you\.[\s\S]*$/;

/** The prompt without cos's "the Chief of Staff started you" line, or null when it has none. */
export function spawnedPrompt(text: string): string | null {
  const match = SPAWNED_LINE.exec(text);
  return match ? text.slice(0, match.index) : null;
}

/**
 * Rows for the page: your messages, the chief's turns (its text and tool
 * calls between two of yours), wakeups, then what you sent that the
 * transcript doesn't show yet.
 */
export function conversationItems(
  messages: TranscriptMessage[],
  pending: PendingMessage[] = [],
): ConversationItem[] {
  const items: ConversationItem[] = [];
  let turn: Extract<ConversationItem, { type: "turn" }> | null = null;
  let lastUpdateMs = 0;
  for (const message of messages) {
    if (message.role === "user" && message.kind === "update") {
      turn = null;
      items.push({
        type: "update",
        key: `u${message.n}`,
        message,
        count: updateCount(message.text),
        sinceMs: lastUpdateMs,
      });
      lastUpdateMs = message.tsMs;
    } else if (message.role === "user") {
      // A tool result Claude logs as a user turn has no text; skip it.
      if (message.kind !== "text" || !message.text.trim()) continue;
      turn = null;
      const asked = spawnedPrompt(message.text);
      items.push(
        asked === null
          ? { type: "user", key: `m${message.n}`, message }
          : {
              type: "user",
              key: `m${message.n}`,
              message: { ...message, text: asked },
              fromChief: true,
            },
      );
    } else {
      if (message.kind !== "tool" && !message.text.trim()) continue;
      if (!turn) {
        turn = { type: "turn", key: `t${message.n}`, messages: [], tsMs: message.tsMs };
        items.push(turn);
      }
      turn.messages.push(message);
    }
  }
  for (const item of pending) items.push({ type: "pending", key: `p${item.id}`, pending: item });
  return items;
}

/**
 * The inbox items an update carried: those reported after the previous
 * update and by the time this one was delivered.
 */
export function itemsForUpdate(
  inbox: InboxItem[],
  update: { message: TranscriptMessage; sinceMs: number },
): InboxItem[] {
  const until = update.message.tsMs;
  if (!until) return [];
  // A little slack: the daemon stamps the item before it types the wakeup.
  return inbox
    .filter((item) => item.tsMs > update.sinceMs && item.tsMs <= until + 2000)
    .sort((a, b) => a.tsMs - b.tsMs);
}

const cosTools: Record<string, string> = {
  workspaces_list: "Looked at the workspaces",
  workspace_create: "Created a workspace",
  agent_spawn: "Started an agent",
  agent_send: "Messaged an agent",
  agent_read: "Checked on an agent",
  agent_stop: "Stopped an agent",
  brief_update: "Updated its brief",
};

/** A tool call's name as the page shows it: cos's own tools in words. */
export function toolTitle(name: string): string {
  const cos = /^mcp__cos__(.+)$/.exec(name);
  if (cos) return cosTools[cos[1]!] ?? cos[1]!.replaceAll("_", " ");
  const mcp = /^mcp__([^_]+(?:_[^_]+)*)__(.+)$/.exec(name);
  if (mcp) return `${mcp[2]!.replaceAll("_", " ")} (${mcp[1]})`;
  return name;
}

/** Whether a tool is one of cos's own, which the chief uses to coordinate. */
export function isCosTool(name: string): boolean {
  return name.startsWith("mcp__cos__");
}
