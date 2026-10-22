import type { ContextUsage } from "../../api/staff";
import { cx } from "../../ui/cx";
import { contextMeter } from "./contextMeter";

const RADIUS = 5.5;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** A small ring and "42% context" in the chief's header; nothing until Claude has answered. */
export function ContextRing({ usage }: { usage: ContextUsage | null | undefined }) {
  const meter = contextMeter(usage);
  if (!meter) return null;
  const stroke =
    meter.level === "full"
      ? "stroke-accent"
      : meter.level === "high"
        ? "stroke-tone-amber"
        : "stroke-ink-3";
  return (
    <span
      role="meter"
      aria-label={meter.title}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={meter.percent ?? undefined}
      title={meter.title}
      className={cx(
        "flex h-7 shrink-0 items-center gap-1.5 px-1.5 text-[12px] whitespace-nowrap tabular-nums",
        meter.level === "full" ? "text-accent-ink" : "text-ink-3",
      )}
    >
      {meter.percent !== null ? (
        <svg viewBox="0 0 14 14" className="size-3.5 -rotate-90" aria-hidden>
          <circle
            cx="7"
            cy="7"
            r={RADIUS}
            fill="none"
            strokeWidth="2"
            className="stroke-line-strong"
          />
          <circle
            cx="7"
            cy="7"
            r={RADIUS}
            fill="none"
            strokeWidth="2"
            strokeLinecap="round"
            className={stroke}
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - meter.percent / 100)}
          />
        </svg>
      ) : null}
      {/* A narrow chief column keeps only the ring; the tooltip has the numbers. */}
      <span className={meter.percent !== null ? "hidden @min-[34rem]:inline" : undefined}>
        {meter.label}
      </span>
    </span>
  );
}
