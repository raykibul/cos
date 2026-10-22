import type { AgentState } from "../../api/models";
import type { StateLabel } from "../../domain/labels";

/** The line under the chief's name, and its dot. `null` is before it first starts. */
export function staffLabel(state: AgentState | null): StateLabel {
  switch (state) {
    case null:
      return { title: "Not started", tone: "neutral", busy: false, attention: false };
    case "starting":
      return { title: "Starting…", tone: "neutral", busy: true, attention: false };
    case "working":
      return { title: "Working…", tone: "blue", busy: true, attention: false };
    case "idle":
      return { title: "Ready for your next message", tone: "green", busy: false, attention: false };
    case "waiting_approval":
      return { title: "Waiting for your approval", tone: "accent", busy: false, attention: true };
    case "needs_trust":
      return {
        title: "Waiting for you to trust its folder",
        tone: "accent",
        busy: false,
        attention: true,
      };
    case "needs_login":
      return {
        title: "Waiting for you to log in to Claude",
        tone: "accent",
        busy: false,
        attention: true,
      };
    case "exited":
    case "interrupted":
      return {
        title: "Stopped. Send a message to resume it",
        tone: "neutral",
        busy: false,
        attention: false,
      };
    default:
      return { title: state, tone: "neutral", busy: false, attention: false };
  }
}

/** Whether a Claude process runs for it. */
export function staffRunning(state: AgentState | null): boolean {
  return state !== null && state !== "exited" && state !== "interrupted";
}

/** Openers for an empty conversation: each fills the composer, none sends. */
export const suggestions = [
  "Plan the next feature and split it across agents",
  "Review what every workspace is doing right now",
  "Collect the open questions from running agents",
];
