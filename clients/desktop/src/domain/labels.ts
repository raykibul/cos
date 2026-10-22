import type { AgentState, Preset, RunState, TaskState } from "../api/models";
import type { Tone } from "./approvals";

export type { Tone };

function titleCase(raw: string): string {
  const text = raw.replaceAll("_", " ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export interface StateLabel {
  title: string;
  tone: Tone;
  /** Something is happening without you: show motion. */
  busy: boolean;
  /** It waits on you. */
  attention: boolean;
}

export function agentLabel(state: AgentState): StateLabel {
  switch (state) {
    case "starting":
      return { title: "Starting", tone: "neutral", busy: true, attention: false };
    case "needs_trust":
      return { title: "Needs trust", tone: "accent", busy: false, attention: true };
    case "needs_login":
      return { title: "Needs Claude login", tone: "accent", busy: false, attention: true };
    case "working":
      return { title: "Working", tone: "blue", busy: true, attention: false };
    case "waiting_approval":
      return { title: "Needs approval", tone: "accent", busy: false, attention: true };
    case "idle":
      return { title: "Idle", tone: "green", busy: false, attention: false };
    case "exited":
      return { title: "Exited", tone: "neutral", busy: false, attention: false };
    case "interrupted":
      return { title: "Interrupted", tone: "amber", busy: false, attention: false };
    default:
      return { title: titleCase(state), tone: "neutral", busy: false, attention: false };
  }
}

/** Whether a Claude process runs for it. Unknown states count as alive. */
export function isAlive(state: AgentState): boolean {
  return state !== "exited" && state !== "interrupted";
}

export function canResume(state: AgentState): boolean {
  return state === "exited" || state === "interrupted";
}

export function runLabel(state: RunState): StateLabel {
  switch (state) {
    case "planning":
      return { title: "Planning", tone: "purple", busy: true, attention: false };
    case "awaiting_plan_approval":
      return { title: "Plan ready", tone: "accent", busy: false, attention: true };
    case "running":
      return { title: "Running", tone: "blue", busy: true, attention: false };
    case "reporting":
      return { title: "Writing report", tone: "teal", busy: true, attention: false };
    case "done":
      return { title: "Done", tone: "green", busy: false, attention: false };
    case "failed":
      return { title: "Failed", tone: "red", busy: false, attention: false };
    case "cancelled":
      return { title: "Cancelled", tone: "neutral", busy: false, attention: false };
    default:
      return { title: titleCase(state), tone: "neutral", busy: false, attention: false };
  }
}

export function taskLabel(state: TaskState): StateLabel {
  switch (state) {
    case "proposed":
      return { title: "Proposed", tone: "neutral", busy: false, attention: false };
    case "ready":
      return { title: "Ready", tone: "teal", busy: false, attention: false };
    case "running":
      return { title: "Working", tone: "blue", busy: true, attention: false };
    case "checking":
      return { title: "Checking", tone: "purple", busy: true, attention: false };
    case "review":
      return { title: "In review", tone: "purple", busy: true, attention: false };
    case "merged":
      return { title: "Merged", tone: "green", busy: false, attention: false };
    case "blocked":
      return { title: "Blocked", tone: "accent", busy: false, attention: true };
    case "failed":
      return { title: "Failed", tone: "red", busy: false, attention: false };
    case "cancelled":
      return { title: "Cancelled", tone: "neutral", busy: false, attention: false };
    default:
      return { title: titleCase(state), tone: "neutral", busy: false, attention: false };
  }
}

export const presets: Preset[] = ["strict", "normal", "trusted"];

export function presetTitle(preset: Preset): string {
  return preset === "strict"
    ? "Strict"
    : preset === "normal"
      ? "Normal"
      : preset === "trusted"
        ? "Trusted"
        : titleCase(preset);
}

/** One line on what the chief may approve for you. */
export function presetExplanation(preset: Preset): string {
  switch (preset) {
    case "strict":
      return "The chief approves nothing. Every request the rules don't allow asks you.";
    case "normal":
      return "The chief may approve builds, scripts and edits inside a task's worktree. Network access and installs ask you.";
    case "trusted":
      return "The chief may also approve downloads, network reads and installs into the worktree.";
    default:
      return "A preset this app doesn't know yet.";
  }
}

export const models = [
  { value: "", title: "Default model", short: "Default" },
  { value: "opus", title: "Opus", short: "Opus" },
  { value: "sonnet", title: "Sonnet", short: "Sonnet" },
  { value: "haiku", title: "Haiku", short: "Haiku" },
] as const;

export const permissionModes = [
  {
    value: "default",
    title: "Ask before acting",
    detail: "Claude asks before edits and commands.",
  },
  {
    value: "acceptEdits",
    title: "Accept edits",
    detail: "File edits go ahead; commands still ask.",
  },
  { value: "plan", title: "Plan only", detail: "Claude reads and plans, and changes nothing." },
  { value: "dontAsk", title: "Don't ask", detail: "Only what your allow rules permit runs." },
] as const;

export function permissionModeTitle(mode: string | null): string {
  return permissionModes.find((m) => m.value === mode)?.title ?? mode ?? "Ask before acting";
}

export function pluralize(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}
