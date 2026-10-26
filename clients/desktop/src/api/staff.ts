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
  reqStr,
  str,
  time,
} from "./decode";
import { type AttachedFile, decodeAttachedFiles } from "./attachments";
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

/** The most bytes a brief holds; the daemon refuses more. */
export const MAX_BRIEF_BYTES = 4 * 1024;
/** The most bytes a memory note's body holds. */
export const MAX_NOTE_BYTES = 8 * 1024;
/** The most characters a note's one-line description holds. */
export const MAX_NOTE_DESCRIPTION = 200;
/** The most notes a project's chief keeps. */
export const MAX_NOTES = 50;

/** How many bytes `text` takes as UTF-8, as the daemon counts them. */
export function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

/**
 * One of the chief's memory notes (docs/chief-of-staff.md, "Memory notes"):
 * what lasts, one subject per note. Only the name and description go in its
 * prompt; it reads the body when it needs it.
 */
export interface StaffNote {
  name: string;
  description: string;
  body: string;
  createdMs: number;
  updatedMs: number;
}

export function decodeStaffNote(value: unknown): StaffNote {
  const o = asObject(value, "note");
  return {
    name: reqStr(o, "name", "note"),
    description: str(o.description),
    body: str(o.body),
    createdMs: num(o.created_ms, 0),
    updatedMs: num(o.updated_ms, 0),
  };
}

/** `{notes: [...]}`, by name. */
export function decodeStaffMemory(value: unknown): StaffNote[] {
  return list(asObject(value, "memory").notes, decodeStaffNote).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
}

/** Why a note's name won't do, or null: 1 to 64 lowercase letters, digits and `-`, starting and ending with a letter or digit. */
export function noteNameProblem(name: string): string | null {
  if (!name) return "Give it a name.";
  if (name.length > 64) return "Keep the name to 64 characters.";
  if (!/^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(name))
    return "Use lowercase letters, digits and dashes, starting and ending with a letter or digit.";
  return null;
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
  /**
   * The agent one of cos's agent tools acted on: `agent_spawn`'s new agent
   * (once its result is in), or the one `agent_send`, `agent_read` and
   * `agent_stop` were given. Null from older daemons.
   */
  agentId: number | null;
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
  /** Files you attached, which `text` leaves out. */
  attachments: AttachedFile[];
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
    agentId: optNum(value.agent_id),
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
    attachments: decodeAttachedFiles(o.attachments),
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
