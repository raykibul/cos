import { Check, type LucideIcon } from "lucide-react";
import {
  type ReactNode,
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { cx } from "./cx";

export type MenuItem =
  | {
      label: string;
      detail?: string;
      icon?: LucideIcon;
      onSelect: () => void;
      disabled?: boolean;
      danger?: boolean;
      checked?: boolean;
    }
  | "separator";

export interface TriggerProps {
  ref: (el: HTMLButtonElement | null) => void;
  onClick: () => void;
  onKeyDown: (event: ReactKeyboardEvent) => void;
  "aria-haspopup": "menu";
  "aria-expanded": boolean;
  "aria-controls": string | undefined;
}

/**
 * A dropdown menu, drawn in a portal so scrolling panes don't clip it.
 * Arrow keys move between items, Return picks one, Escape closes.
 */
export function Menu({
  trigger,
  items,
  align = "start",
  width = 220,
}: {
  trigger: (props: TriggerProps) => ReactNode;
  items: MenuItem[];
  align?: "start" | "end";
  width?: number;
}) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  // The trigger, in state rather than a ref: it's handed out during render.
  const [button, setButton] = useState<HTMLButtonElement | null>(null);
  const list = useRef<HTMLDivElement | null>(null);
  const id = useId();

  const close = useCallback(
    (refocus: boolean) => {
      setOpen(false);
      if (refocus) button?.focus();
    },
    [button],
  );

  useLayoutEffect(() => {
    if (!open || !button) return;
    const rect = button.getBoundingClientRect();
    const height = list.current?.offsetHeight ?? 200;
    let top = rect.bottom + 4;
    if (top + height > window.innerHeight - 8) top = Math.max(8, rect.top - height - 4);
    let left = align === "end" ? rect.right - width : rect.left;
    left = Math.min(Math.max(8, left), window.innerWidth - width - 8);
    setPosition({ top, left });
  }, [open, align, width, button]);

  useEffect(() => {
    if (!open) return;
    const first = list.current?.querySelector<HTMLElement>("[role=menuitem]:not([disabled])");
    first?.focus();
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!list.current?.contains(target) && !button?.contains(target)) close(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        close(true);
      }
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", () => close(false), { once: true });
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, close, button]);

  const move = (event: ReactKeyboardEvent) => {
    const all = Array.from(
      list.current?.querySelectorAll<HTMLElement>("[role=menuitem]:not([disabled])") ?? [],
    );
    const index = all.indexOf(document.activeElement as HTMLElement);
    if (event.key === "ArrowDown") {
      event.preventDefault();
      all[(index + 1) % all.length]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      all[(index - 1 + all.length) % all.length]?.focus();
    } else if (event.key === "Tab") {
      close(false);
    }
  };

  return (
    <>
      {trigger({
        ref: setButton,
        onClick: () => setOpen((value) => !value),
        onKeyDown: (event) => {
          if (event.key === "ArrowDown" && !open) {
            event.preventDefault();
            setOpen(true);
          }
        },
        "aria-haspopup": "menu",
        "aria-expanded": open,
        "aria-controls": open ? id : undefined,
      })}
      {open
        ? createPortal(
            <div
              ref={list}
              id={id}
              role="menu"
              onKeyDown={move}
              style={{ top: position?.top ?? -9999, left: position?.left ?? -9999, width }}
              className="fixed z-[60] rounded-xl border border-line bg-raised p-1 shadow-pop animate-pop-in"
            >
              {items.map((item, index) =>
                item === "separator" ? (
                  <div key={`sep-${index}`} className="mx-2 my-1 h-px bg-line" role="separator" />
                ) : (
                  <button
                    key={item.label}
                    type="button"
                    role="menuitem"
                    disabled={item.disabled}
                    onClick={() => {
                      close(true);
                      item.onSelect();
                    }}
                    className={cx(
                      "flex w-full items-start gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px] outline-none",
                      "hover:bg-subtle focus:bg-subtle disabled:pointer-events-none disabled:opacity-40",
                      item.danger ? "text-tone-red" : "text-ink",
                    )}
                  >
                    {item.icon ? (
                      <item.icon className="mt-0.5 size-4 shrink-0 text-ink-3" strokeWidth={1.8} />
                    ) : null}
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span>{item.label}</span>
                      {item.detail ? (
                        <span className="text-[12px] leading-snug text-ink-3">{item.detail}</span>
                      ) : null}
                    </span>
                    {item.checked ? <Check className="mt-0.5 size-4 shrink-0 text-accent" /> : null}
                  </button>
                ),
              )}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
