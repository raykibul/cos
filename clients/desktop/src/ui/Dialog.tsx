import { X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { cx } from "./cx";

/**
 * A centred sheet over a dimmed window. Escape or a click outside closes
 * it; focus moves into it and goes back where it was on close.
 */
export function Dialog({
  title,
  description,
  onClose,
  children,
  footer,
  width = "md",
  onSubmit,
}: {
  title: ReactNode;
  description?: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  footer?: ReactNode;
  width?: "sm" | "md" | "lg";
  /** Called for Return in a field or ⌘Return anywhere. */
  onSubmit?: () => void;
}) {
  const titleId = useId();
  const panel = useRef<HTMLFormElement>(null);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const el = panel.current;
    const first =
      el?.querySelector<HTMLElement>("[data-autofocus]") ??
      el?.querySelector<HTMLElement>("input, textarea, select") ??
      el?.querySelector<HTMLElement>("button:not([data-close])");
    first?.focus();
    return () => previous?.focus?.();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-[var(--c-backdrop)] px-4 pt-[12vh] pb-8 animate-fade-in"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <form
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={cx(
          "relative w-full rounded-2xl border border-line bg-raised shadow-pop animate-pop-in",
          width === "sm" ? "max-w-[400px]" : width === "md" ? "max-w-[480px]" : "max-w-[600px]",
        )}
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit?.();
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && onSubmit) {
            event.preventDefault();
            onSubmit();
          }
        }}
      >
        <div className="px-5 pt-5">
          <h2
            id={titleId}
            className="pr-8 font-serif text-[19px] leading-snug font-medium text-ink"
          >
            {title}
          </h2>
          {description ? (
            <div className="mt-1.5 text-[13px] leading-relaxed text-ink-2">{description}</div>
          ) : null}
          <button
            type="button"
            data-close
            aria-label="Close"
            onClick={onClose}
            className="absolute top-4 right-4 flex size-7 items-center justify-center rounded-md text-ink-3 hover:bg-subtle hover:text-ink"
          >
            <X className="size-4" strokeWidth={1.8} />
          </button>
        </div>
        {children ? <div className="flex flex-col gap-4 px-5 pt-4">{children}</div> : null}
        {footer ? (
          <div className="mt-5 flex items-center justify-end gap-2 border-t border-line px-5 py-3.5">
            {footer}
          </div>
        ) : (
          <div className="h-5" />
        )}
      </form>
    </div>,
    document.body,
  );
}

/** A line of error text from the daemon, shown as-is. */
export function ErrorText({ children }: { children: ReactNode }) {
  if (!children) return null;
  return (
    <p
      role="alert"
      className="rounded-lg bg-tone-red/10 px-3 py-2 text-[12.5px] leading-snug text-tone-red select-text"
    >
      {children}
    </p>
  );
}
