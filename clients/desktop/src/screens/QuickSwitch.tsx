// The ⌘K switcher: type to jump to any project's chief, what waits on you,
// an agent, a workspace or a page, or open a forwarded port. Return opens
// it; Tab on a chief writes to it right here, without leaving the page
// you're on.

import { ArrowUp, CornerDownLeft, Search } from "lucide-react";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStore } from "zustand";
import { useApp, useCanChange, useCos } from "../app/context";
import { type SwitchItem, filterItems, sections, switchItems } from "../domain/switcher";
import { project as projectOf } from "../store/selectors";
import { cx } from "../ui/cx";
import { ChiefMark } from "./sidebar/ProjectSection";

export function QuickSwitch() {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  // The whole state: the list is rebuilt from it while the switcher is open.
  const state = useStore(cos.state);
  const all = useMemo(() => switchItems(state), [state]);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [messaging, setMessaging] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const listId = useId();
  const chief = useCos((s) => (messaging !== null ? projectOf(s, messaging) : undefined));

  const items = useMemo(() => filterItems(all, query), [all, query]);
  const current = items[Math.min(selected, items.length - 1)];

  // Focus goes back where it was when the switcher closes.
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    return () => previous?.focus?.();
  }, []);

  const close = () => ui.openSwitcher(false);
  const open = (item: SwitchItem) => {
    close();
    const port = item.forward;
    if (port !== undefined) {
      void ui.perform(`Couldn't open ${item.title.replace(/^Open /, "")}`, () =>
        cos.openForward(port),
      );
    } else if (item.page) {
      ui.navigate(item.page);
    }
  };
  const send = () => {
    const text = message.trim();
    if (messaging === null || !text || !canChange) return;
    close();
    void cos.messageStaff(messaging, text);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (messaging !== null) setMessaging(null);
      else close();
      return;
    }
    if (messaging !== null) {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        send();
      }
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setSelected((i) => (items.length === 0 ? 0 : (i + step + items.length) % items.length));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (current) open(current);
    } else if (event.key === "Tab") {
      event.preventDefault();
      if (current?.chiefOf != null) {
        setMessage("");
        setMessaging(current.chiefOf);
      }
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-[var(--c-backdrop)] px-4 pt-[11vh] animate-fade-in"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Jump to"
        onKeyDown={onKeyDown}
        className="flex max-h-[70vh] w-full max-w-[620px] flex-col overflow-hidden rounded-2xl border border-line bg-raised shadow-pop animate-pop-in"
      >
        {messaging !== null ? (
          <div className="flex flex-col gap-3 p-4">
            <div className="flex items-center gap-2.5">
              <ChiefMark size="sm" />
              <span className="text-[13px] font-medium text-ink">
                To the chief of {chief?.name ?? "this project"}
              </span>
              <span className="ml-auto text-[11.5px] text-ink-3">Esc goes back</span>
            </div>
            <textarea
              aria-label={`Message the chief of ${chief?.name ?? "this project"}`}
              rows={3}
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              placeholder="It reads this on its next turn. You stay where you are."
              className="w-full resize-none rounded-xl border border-line-strong bg-surface px-3 py-2.5 text-[14px] leading-relaxed text-ink outline-none placeholder:text-ink-3 focus:border-ink-3/50"
              autoFocus
            />
            <div className="flex items-center gap-2 text-[11.5px] text-ink-3">
              <span>Shift-Return for a new line</span>
              <button
                type="button"
                onClick={send}
                disabled={!message.trim() || !canChange}
                className="ml-auto flex h-7 items-center gap-1.5 rounded-lg bg-ink px-2.5 text-[12.5px] font-medium text-page disabled:opacity-40"
              >
                Send
                <CornerDownLeft className="size-3.5" strokeWidth={2} aria-hidden />
              </button>
            </div>
          </div>
        ) : (
          <>
            <label className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line px-4">
              <Search className="size-4 shrink-0 text-ink-3" strokeWidth={2} aria-hidden />
              <input
                autoFocus
                role="combobox"
                aria-expanded="true"
                aria-controls={listId}
                aria-activedescendant={current ? `${listId}-${current.key}` : undefined}
                aria-label="Jump to"
                value={query}
                onChange={(event) => {
                  setQuery(event.target.value);
                  setSelected(0);
                }}
                placeholder="Jump to a chief, an agent or a page"
                spellCheck={false}
                autoComplete="off"
                className="min-w-0 flex-1 bg-transparent text-[15px] text-ink outline-none placeholder:text-ink-3"
              />
              <kbd className="rounded border border-line-strong px-1.5 font-mono text-[11px] text-ink-3">
                esc
              </kbd>
            </label>
            <div
              id={listId}
              role="listbox"
              aria-label="Results"
              className="min-h-0 flex-1 overflow-y-auto p-2"
            >
              {items.length === 0 ? (
                <p className="px-3 py-6 text-center text-[13px] text-ink-3">Nothing matches.</p>
              ) : (
                sections.map((section) => {
                  const inSection = items.filter((item) => item.section === section);
                  if (inSection.length === 0) return null;
                  return (
                    <div key={section} role="group" aria-label={section} className="mb-1">
                      <div className="px-3 pt-2 pb-1 text-[11px] font-semibold tracking-wide text-ink-3 uppercase">
                        {section}
                      </div>
                      {inSection.map((item) => (
                        <Row
                          key={item.key}
                          id={`${listId}-${item.key}`}
                          item={item}
                          selected={item === current}
                          onHover={() => setSelected(items.indexOf(item))}
                          onOpen={() => open(item)}
                        />
                      ))}
                    </div>
                  );
                })
              )}
            </div>
            <div className="flex shrink-0 flex-wrap gap-x-4 gap-y-1 border-t border-line px-4 py-2.5 text-[11.5px] text-ink-3">
              <span className="flex items-center gap-1">
                <ArrowUp className="size-3" strokeWidth={2} aria-hidden />
                <ArrowUp className="size-3 rotate-180" strokeWidth={2} aria-hidden /> move
              </span>
              <span>
                <kbd className="font-mono">↵</kbd> open
              </span>
              <span>
                <kbd className="font-mono">⇥</kbd> message the chief without opening it
              </span>
            </div>
          </>
        )}
      </div>
    </div>,
    document.body,
  );
}

function Row({
  id,
  item,
  selected,
  onHover,
  onOpen,
}: {
  id: string;
  item: SwitchItem;
  selected: boolean;
  onHover: () => void;
  onOpen: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (selected) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [selected]);
  return (
    <div
      ref={ref}
      id={id}
      role="option"
      aria-selected={selected}
      onMouseMove={onHover}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onOpen}
      className={cx(
        "flex min-h-10 cursor-default items-center gap-3 rounded-lg px-3 py-1.5",
        selected && "bg-subtle-2",
      )}
    >
      {item.chiefOf !== null ? <ChiefMark size="sm" /> : null}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13.5px] text-ink">{item.title}</span>
        {item.detail ? (
          <span className="block truncate text-[12px] text-ink-3">{item.detail}</span>
        ) : null}
      </span>
      {selected && item.chiefOf !== null ? (
        <span className="shrink-0 text-[11.5px] text-ink-3">⇥ message</span>
      ) : null}
      {item.shortcut ? (
        <kbd className="shrink-0 font-mono text-[11px] text-ink-3">{item.shortcut}</kbd>
      ) : null}
    </div>
  );
}
