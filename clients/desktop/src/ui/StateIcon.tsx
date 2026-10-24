import type { AgentState } from "../api/models";
import { cx } from "./cx";

/**
 * An agent's state as a shape, not only a colour: a filled mark when it
 * waits on you, a turning arc while it works, a tick when it's idle, a
 * dashed ring once it exited and a pause when it was interrupted.
 */
export function StateIcon({ state, className }: { state: AgentState; className?: string }) {
  const box = cx("size-3 shrink-0", className);
  switch (state) {
    case "waiting_approval":
    case "needs_trust":
    case "needs_login":
      return (
        <svg viewBox="0 0 12 12" className={cx(box, "text-accent")} aria-hidden>
          <circle cx="6" cy="6" r="5.5" fill="currentColor" />
          <path
            d="M6 3.1v3.4M6 8.6v.1"
            stroke="var(--c-page)"
            strokeWidth="1.7"
            strokeLinecap="round"
          />
        </svg>
      );
    case "working":
    case "starting":
      return (
        <svg
          viewBox="0 0 12 12"
          className={cx(
            box,
            "animate-spin [animation-duration:1.4s]",
            state === "working" ? "text-tone-blue" : "text-ink-3",
          )}
          aria-hidden
        >
          <circle
            cx="6"
            cy="6"
            r="4.5"
            fill="none"
            stroke="currentColor"
            strokeOpacity="0.3"
            strokeWidth="1.6"
          />
          <path
            d="M6 1.5a4.5 4.5 0 0 1 4.5 4.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
          />
        </svg>
      );
    case "idle":
      return (
        <svg viewBox="0 0 12 12" className={cx(box, "text-tone-green")} aria-hidden>
          <circle cx="6" cy="6" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path
            d="M4 6.1l1.4 1.4L8 4.8"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      );
    case "interrupted":
      return (
        <svg viewBox="0 0 12 12" className={cx(box, "text-tone-amber")} aria-hidden>
          <circle cx="6" cy="6" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <path
            d="M4.8 4.3v3.4M7.2 4.3v3.4"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 12 12" className={cx(box, "text-ink-3")} aria-hidden>
          <circle
            cx="6"
            cy="6"
            r="4.6"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeDasharray="2 2"
          />
        </svg>
      );
  }
}
