// The Chief of Staff's JSON (docs/chief-of-staff.md): its record per
// project, its inbox, and any agent's transcript. Decoded leniently, like
// models.ts: unknown fields are ignored and kinds stay open strings.

import {
  type Json,
  asObject,
  bool,
  isObject,
  list,
  num,
  optNum,
  optStr,
  reqNum,
  str,
  time,
} from "./decode";
import type { AgentState } from "./models";

export interface Staff {
  projectId: number;
  /** Null before it first starts. */
  agentId: number | null;
  state: AgentState | null;
  /** Null for Claude's default (or `COS_STAFF_MODEL`). */
  model: string | null;
  brief: string;
  worktree: string | null;
  branch: string | null;
  /** Counts over the agents it started. */
  workingAgents: number;
  waitingOnYou: number;
  /** Coordination paused: news collects in its inbox, but no wakeups go in. Your messages still do. */
  paused: boolean;
  /** News from its agents not yet delivered to it. */
  updatesWaiting: number;
}

export function decodeStaff(value: unknown): Staff {
  const o = asObject(value, "staff");
  return {
    projectId: reqNum(o, "project_id", "staff"),
    agentId: optNum(o.agent_id),
    state: optStr(o.state),
    model: optStr(o.model) || null,
    brief: str(o.brief),
    worktree: optStr(o.worktree),
    branch: optStr(o.branch),
    workingAgents: num(o.working_agents, 0),
    waitingOnYou: num(o.waiting_on_you, 0),
    paused: bool(o.paused, false),
    updatesWaiting: num(o.updates_waiting, 0),
  };
}

/** How full a conversation's context is, from Claude's latest usage. */
export interface ContextUsage {
  usedTokens: number;
  /** The model's window, when the daemon knows it. */
  windowTokens: number | null;
}

function decodeContext(value: unknown): ContextUsage | null {
  if (!isObject(value)) return null;
  const used = optNum(value.used_tokens);
  if (used === null) return null;
  const window = optNum(value.window_tokens);
  return { usedTokens: used, windowTokens: window !== null && window > 0 ? window : null };
}

export type MessageRole = "user" | "assistant" | (string & {});
/** `update` is a cos inbox wakeup, a user turn that starts with `[cos ·`. */
export type MessageKind = "text" | "tool" | "update" | (string & {});

export interface ToolCall {
  name: string;
  summary: string;
  isError: boolean;
}

export interface TranscriptMessage {
  /** Its position in the transcript; `after=n` asks for what follows. */
  n: number;
  role: MessageRole;
  kind: MessageKind;
  /** Markdown, for text and updates. */
  text: string;
  tool: ToolCall | null;
  tsMs: number;
}

export interface Transcript {
  messages: TranscriptMessage[];
  next: number;
  /** For the whole file, whatever `after` was; null from older daemons or before Claude answered. */
  context: ContextUsage | null;
}

function decodeTool(value: unknown): ToolCall | null {
  if (!isObject(value)) return null;
  return {
    name: str(value.name, "tool"),
    summary: str(value.summary),
    isError: bool(value.is_error, false),
  };
}

export function decodeMessage(value: unknown): TranscriptMessage {
  const o = asObject(value, "message");
  const tool = decodeTool(o.tool);
  const text = str(o.text);
  let kind = str(o.kind) || (tool ? "tool" : "text");
  // A wakeup the daemon didn't tag still reads as one.
  if (kind === "text" && str(o.role) === "user" && text.startsWith("[cos ·")) kind = "update";
  return {
    n: reqNum(o, "n", "message"),
    role: str(o.role, "assistant"),
    kind,
    text,
    tool,
    tsMs: num(o.ts_ms, 0) || time(o.ts),
  };
}

export function decodeTranscript(value: unknown): Transcript {
  const o = asObject(value, "transcript");
  const messages = list(o.messages, decodeMessage).sort((a, b) => a.n - b.n);
  const last = messages[messages.length - 1];
  return { messages, next: num(o.next, last ? last.n : 0), context: decodeContext(o.context) };
}

/** What an agent the chief started reported. */
export interface InboxItem {
  /** Stable within a listing, for React keys. */
  key: string;
  /** `stop`, `exit`, `approval`, or something newer. */
  kind: string;
  agentId: number | null;
  /** Who it's from: the agent's title, when the daemon says. */
  title: string | null;
  text: string;
  tsMs: number;
}

export function decodeInboxItem(value: unknown): InboxItem {
  const o: Json = asObject(value, "inbox item");
  const kind = str(o.kind) || str(o.type) || "update";
  const agentId = optNum(o.agent_id);
  const tsMs = num(o.ts_ms, 0) || num(o.created_ms, 0) || time(o.created_at) || time(o.ts);
  const id = o.id;
  return {
    key:
      typeof id === "number" || typeof id === "string"
        ? String(id)
        : `${kind}-${agentId ?? "x"}-${tsMs}`,
    kind,
    agentId,
    title: optStr(o.agent_title) || optStr(o.title) || null,
    text: str(o.text) || str(o.message) || str(o.last_message) || str(o.summary),
    tsMs,
  };
}

/** `{items: [...]}`, newest first. */
export function decodeInbox(value: unknown): InboxItem[] {
  const items = Array.isArray(value)
    ? list(value, decodeInboxItem)
    : list(asObject(value, "inbox").items, decodeInboxItem);
  return items.sort((a, b) => b.tsMs - a.tsMs);
}
