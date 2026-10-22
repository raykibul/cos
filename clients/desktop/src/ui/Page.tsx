import type { ReactNode } from "react";
import { cx } from "./cx";

/**
 * The bar at the top of a page: a drag region for the window (it has no
 * title bar), with a breadcrumb on the left and actions on the right.
 */
export function PageHeader({ children, actions }: { children?: ReactNode; actions?: ReactNode }) {
  return (
    <header
      data-tauri-drag-region
      className="flex h-[52px] shrink-0 items-center gap-3 px-5 select-none"
    >
      <div
        data-tauri-drag-region
        className="flex min-w-0 flex-1 items-center gap-1.5 text-[13px] text-ink-3"
      >
        {children}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
    </header>
  );
}

export function Crumb({ children, onClick }: { children: ReactNode; onClick?: () => void }) {
  if (!onClick) return <span className="truncate text-ink-2">{children}</span>;
  return (
    <button
      type="button"
      onClick={onClick}
      className="truncate rounded px-1 py-0.5 hover:bg-subtle hover:text-ink"
    >
      {children}
    </button>
  );
}

export function CrumbSep() {
  return <span className="text-ink-3/60">/</span>;
}

/** The scrolling body of a page, with a readable column. */
export function PageBody({
  children,
  wide,
  className,
}: {
  children: ReactNode;
  wide?: boolean;
  className?: string;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div
        className={cx(
          "mx-auto w-full px-8 pt-4 pb-20",
          wide ? "max-w-[1040px]" : "max-w-[860px]",
          className,
        )}
      >
        {children}
      </div>
    </div>
  );
}

/** A page's big serif heading, with an optional line under it. */
export function PageTitle({
  children,
  subtitle,
  badge,
  actions,
}: {
  children: ReactNode;
  subtitle?: ReactNode;
  badge?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-7 flex items-start gap-4">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <h1 className="font-serif text-[28px] leading-[1.2] font-medium tracking-[-0.012em] text-ink select-text">
            {children}
          </h1>
          {badge}
        </div>
        {subtitle ? <div className="mt-1.5 text-[13px] text-ink-2">{subtitle}</div> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2 pt-1">{actions}</div> : null}
    </div>
  );
}
