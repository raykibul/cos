import type { Approval } from "../api/models";

/** Yours to answer: every pending approval waits for you. */
export function isForYou(approval: Approval): boolean {
  return approval.state === "pending";
}

/**
 * What the tool will do, for a banner: the command for Bash, the path for
 * file tools, otherwise the daemon's summary.
 */
export function approvalDetail(approval: Approval): string {
  const input = approval.toolInput;
  if (typeof input === "object" && input !== null) {
    for (const key of ["command", "file_path", "path", "url", "pattern"]) {
      const value = (input as Record<string, unknown>)[key];
      if (typeof value === "string" && value !== "") return value;
    }
  }
  return approval.summary;
}

/** "Claude wants to run:" for commands, "Claude wants to use Write:" otherwise. */
export function approvalHeadline(approval: Approval): string {
  return approval.toolName === "Bash"
    ? "Claude wants to run"
    : `Claude wants to use ${approval.toolName}`;
}

/** Why cos asks you, from the policy's tier and reason. Null from a daemon without them. */
export function routeExplanation(approval: Approval): string | null {
  const reason = approval.reason?.trim() ? approval.reason.trim() : null;
  if (approval.tier === "never_delegated") {
    return "cos always asks you: " + (reason ?? "this is never delegated");
  }
  return reason ? `cos asks you because ${reason}` : null;
}

function who(decidedBy: string | null): string | null {
  switch (decidedBy) {
    case null:
      return null;
    case "user":
      return "you";
    case "terminal":
      return "the terminal";
    default:
      return decidedBy;
  }
}

/** "Allowed by the chief", "Denied by you", "Waiting for you". */
export function approvalOutcome(approval: Approval): string {
  const by = who(approval.decidedBy);
  switch (approval.state) {
    case "pending":
      return "Waiting for you";
    case "expired":
      return "Expired unanswered";
    case "settled":
      return "Settled" + (by ? ` in ${by}` : "");
    default: {
      const state = approval.state.charAt(0).toUpperCase() + approval.state.slice(1);
      return state + (by ? ` by ${by}` : "");
    }
  }
}

export type Tone = "neutral" | "accent" | "blue" | "green" | "red" | "purple" | "teal" | "amber";

export function approvalTone(approval: Approval): Tone {
  switch (approval.state) {
    case "pending":
      return "accent";
    case "allowed":
      return "green";
    case "denied":
    case "expired":
      return "red";
    default:
      return "neutral";
  }
}
