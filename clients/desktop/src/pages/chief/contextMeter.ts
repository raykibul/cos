// How full the chief's conversation is, for its header: the latest usage
// Claude reported (docs/chief-of-staff.md, "Transcript"), as in Selene.

import type { ContextUsage } from "../../api/staff";

/** At or past these percentages the meter turns amber, then accent. */
export const CONTEXT_HIGH = 70;
export const CONTEXT_FULL = 90;

export type ContextLevel = "ok" | "high" | "full";

export interface ContextMeter {
  /** "42% context", or "84K context" without a window. */
  label: string;
  /** The exact numbers, for the tooltip. */
  title: string;
  /** 0–100, when the window is known. */
  percent: number | null;
  level: ContextLevel;
}

/** 84123 → "84K", 1200000 → "1.2M", 950 → "950". */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${Number((n / 1_000_000).toFixed(1))}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(Math.round(n));
}

const exact = (n: number) => Math.round(n).toLocaleString("en-US");

export function contextMeter(usage: ContextUsage | null | undefined): ContextMeter | null {
  if (!usage || usage.usedTokens <= 0) return null;
  if (!usage.windowTokens) {
    return {
      label: `${formatTokens(usage.usedTokens)} context`,
      title: `${exact(usage.usedTokens)} tokens in context; its model's window isn't known`,
      percent: null,
      level: "ok",
    };
  }
  const percent = Math.min(100, Math.round((usage.usedTokens / usage.windowTokens) * 100));
  return {
    label: `${percent}% context`,
    title: `${exact(usage.usedTokens)} of ${exact(usage.windowTokens)} tokens in context`,
    percent,
    level: percent >= CONTEXT_FULL ? "full" : percent >= CONTEXT_HIGH ? "high" : "ok",
  };
}
