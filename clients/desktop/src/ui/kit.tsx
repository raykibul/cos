// Small building blocks: buttons, badges, cards, fields and empty states.

import { LoaderCircle, type LucideIcon } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
  forwardRef,
  useLayoutEffect,
  useRef,
} from "react";
import type { StateLabel } from "../domain/labels";
import { cx } from "./cx";
import { toneClasses } from "./tones";

type Variant = "primary" | "accent" | "secondary" | "ghost" | "danger" | "destructive";

const variants: Record<Variant, string> = {
  // Claude's main buttons are ink on paper, not orange.
  primary: "bg-ink text-page hover:bg-ink/88 active:bg-ink/80 shadow-sm",
  accent: "bg-accent text-white hover:bg-accent-strong active:bg-accent-strong shadow-sm",
  secondary:
    "bg-surface text-ink border border-line-strong hover:bg-subtle active:bg-subtle-2 shadow-[0_1px_1px_rgb(0_0_0/0.03)]",
  ghost: "text-ink-2 hover:bg-subtle hover:text-ink active:bg-subtle-2",
  danger: "text-tone-red hover:bg-tone-red/10 active:bg-tone-red/15",
  destructive: "bg-tone-red text-white hover:bg-tone-red/90 active:bg-tone-red/85 shadow-sm",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: "sm" | "md";
  icon?: LucideIcon;
  busy?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon: Icon, busy, className, children, disabled, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      disabled={disabled || busy}
      className={cx(
        "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap select-none",
        "transition-colors duration-100 disabled:pointer-events-none disabled:opacity-45",
        size === "sm" ? "h-7 px-2.5 text-[12.5px]" : "h-8 px-3 text-[13px]",
        variants[variant],
        className,
      )}
      {...rest}
    >
      {busy ? (
        <LoaderCircle className="size-3.5 animate-spin" aria-hidden />
      ) : Icon ? (
        <Icon className={size === "sm" ? "size-3.5" : "size-4"} strokeWidth={1.8} aria-hidden />
      ) : null}
      {children}
    </button>
  );
});

export function IconButton({
  icon: Icon,
  label,
  className,
  size = "md",
  ...rest
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & {
  icon: LucideIcon;
  label: string;
  size?: "sm" | "md";
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex shrink-0 items-center justify-center rounded-md text-ink-3 transition-colors",
        "hover:bg-subtle-2/70 hover:text-ink disabled:pointer-events-none disabled:opacity-40",
        size === "sm" ? "size-6" : "size-7",
        className,
      )}
      {...rest}
    >
      <Icon className={size === "sm" ? "size-3.5" : "size-4"} strokeWidth={1.8} aria-hidden />
    </button>
  );
}

/** A dot in a state's colour; it breathes while something is happening. */
export function StateDot({ label, className }: { label: StateLabel; className?: string }) {
  return (
    <span
      className={cx(
        "inline-block size-2 shrink-0 rounded-full",
        toneClasses[label.tone].dot,
        label.busy && "animate-breathe",
        className,
      )}
      aria-hidden
    />
  );
}

export function StateBadge({ label, className }: { label: StateLabel; className?: string }) {
  const tone = toneClasses[label.tone];
  return (
    <span
      className={cx(
        "inline-flex h-[22px] shrink-0 items-center gap-1.5 rounded-full px-2 text-[12px] font-medium whitespace-nowrap",
        tone.soft,
        tone.text,
        className,
      )}
    >
      <StateDot label={label} />
      {label.title}
    </span>
  );
}

export function Pill({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cx(
        "inline-flex h-5 items-center rounded-md bg-subtle px-1.5 font-mono text-[11.5px] text-ink-2",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function Card({
  children,
  className,
  as: Tag = "div",
}: {
  children: ReactNode;
  className?: string;
  as?: "div" | "section" | "article" | "li";
}) {
  return (
    <Tag className={cx("rounded-xl border border-line bg-surface shadow-card", className)}>
      {children}
    </Tag>
  );
}

/** A section heading inside a page. */
export function SectionTitle({
  children,
  actions,
  className,
}: {
  children: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cx("mb-2.5 flex min-h-8 items-center gap-2", className)}>
      <h2 className="text-[13px] font-semibold tracking-[0.01em] text-ink-2">{children}</h2>
      <div className="ml-auto flex items-center gap-1.5">{actions}</div>
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  children,
  actions,
  className,
}: {
  icon: LucideIcon;
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx("flex flex-col items-center justify-center px-6 py-10 text-center", className)}
    >
      <div className="mb-3 flex size-10 items-center justify-center rounded-full bg-subtle text-ink-3">
        <Icon className="size-5" strokeWidth={1.7} aria-hidden />
      </div>
      <p className="font-serif text-[17px] font-medium text-ink">{title}</p>
      {children ? <div className="mt-1 max-w-md text-[13px] text-ink-2">{children}</div> : null}
      {actions ? <div className="mt-4 flex gap-2">{actions}</div> : null}
    </div>
  );
}

/** An on/off switch; it pulses while the change is on its way. */
export function Switch({
  on,
  busy = false,
  disabled,
  label,
  onChange,
}: {
  on: boolean;
  busy?: boolean;
  disabled?: boolean;
  label: string;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy}
      disabled={disabled || busy}
      onClick={() => onChange(!on)}
      className={cx(
        "relative inline-flex h-6 w-10 shrink-0 items-center rounded-full transition-colors disabled:opacity-60",
        on ? "bg-tone-green" : "bg-line-strong",
      )}
    >
      <span
        className={cx(
          "inline-block size-5 rounded-full bg-white shadow-sm transition-transform",
          on ? "translate-x-[18px]" : "translate-x-0.5",
          busy && "animate-pulse",
        )}
      />
    </button>
  );
}

export function Spinner({ className }: { className?: string }) {
  return <LoaderCircle className={cx("size-4 animate-spin text-ink-3", className)} aria-hidden />;
}

export function Field({
  label,
  hint,
  children,
  htmlFor,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
  htmlFor?: string;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-[12.5px] font-medium text-ink-2">
        {label}
      </label>
      {children}
      {hint ? <p className="text-[12px] leading-snug text-ink-3">{hint}</p> : null}
    </div>
  );
}

export const inputClass =
  "w-full rounded-lg border border-line-strong bg-surface px-3 py-2 text-[13.5px] text-ink placeholder:text-ink-3 outline-none transition-shadow focus:border-accent/60 focus:ring-3 focus:ring-accent/15";

/** A textarea that grows with its text, up to `maxRows`. */
export const AutoTextarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement> & { minRows?: number; maxRows?: number }
>(function AutoTextarea({ minRows = 1, maxRows = 12, className, value, ...rest }, forwarded) {
  const inner = useRef<HTMLTextAreaElement | null>(null);
  useLayoutEffect(() => {
    const el = inner.current;
    if (!el) return;
    const line = parseFloat(getComputedStyle(el).lineHeight) || 20;
    el.style.height = "auto";
    const padding = el.offsetHeight - el.clientHeight + 0;
    const height = Math.min(Math.max(el.scrollHeight, line * minRows), line * maxRows);
    el.style.height = `${height + padding}px`;
  }, [value, minRows, maxRows]);
  return (
    <textarea
      ref={(el) => {
        inner.current = el;
        if (typeof forwarded === "function") forwarded(el);
        else if (forwarded) forwarded.current = el;
      }}
      value={value}
      rows={minRows}
      className={cx("resize-none", className)}
      {...rest}
    />
  );
});

/** A path in mono, home shortened to ~. */
export function PathText({ path, className }: { path: string; className?: string }) {
  return (
    <span
      className={cx("font-mono text-[12.5px] break-all text-ink-2 select-text", className)}
      title={path}
    >
      {path.replace(/^\/Users\/[^/]+/, "~")}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-line-strong bg-surface px-1 font-sans text-[11px] text-ink-3">
      {children}
    </kbd>
  );
}
