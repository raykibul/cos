import { useLayoutEffect, useRef } from "react";
import { clampWidth, defaultWidth, saveWidth, savedWidth } from "./split";

const KEY_STEP = 24;

/**
 * The divider between a conversation (the chief's or an agent's) and its terminal: it sizes
 * the element right after it. Dragging sets that width straight on the
 * element, so nothing re-renders while it moves; the width is saved when the
 * drag ends. Double-click goes back to the default, and the arrow keys move
 * it too. The pane is found in the DOM, which is in place before any layout
 * effect runs (a sibling's ref may not be yet).
 */
export function SplitDivider({ label = "Resize the chief's terminal" }: { label?: string }) {
  const handle = useRef<HTMLDivElement>(null);
  /** The width asked for (null: the default); the shown one is it clamped to the window. */
  const wanted = useRef<number | null>(null);
  const shown = useRef(0);
  const dragging = useRef(false);

  const pane = () => handle.current?.nextElementSibling as HTMLElement | null | undefined;
  const total = () => handle.current?.parentElement?.clientWidth ?? 0;
  const paint = (width: number) => {
    const value = clampWidth(width, total());
    shown.current = value;
    const element = pane();
    if (element) element.style.width = `${value}px`;
    handle.current?.setAttribute("aria-valuenow", String(value));
  };
  const repaint = () => paint(wanted.current ?? defaultWidth(total()));
  const commit = (width: number) => {
    paint(width);
    wanted.current = shown.current;
    saveWidth(shown.current);
  };
  const endDrag = (target: HTMLElement) => {
    dragging.current = false;
    delete target.dataset.dragging;
    document.body.style.removeProperty("cursor");
  };

  useLayoutEffect(() => {
    wanted.current = savedWidth();
    repaint();
    const container = handle.current?.parentElement;
    if (!container || typeof ResizeObserver === "undefined") return;
    // A narrower window squeezes the terminal; it grows back after.
    const observer = new ResizeObserver(() => {
      if (!dragging.current) repaint();
    });
    observer.observe(container);
    return () => observer.disconnect();
    // Once: everything it reads is a ref or the DOM.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div
      ref={handle}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      tabIndex={0}
      title="Drag to resize · Double-click to reset"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        dragging.current = true;
        event.currentTarget.dataset.dragging = "";
        event.currentTarget.setPointerCapture?.(event.pointerId);
        document.body.style.cursor = "col-resize";
      }}
      onPointerMove={(event) => {
        if (!dragging.current) return;
        const right = handle.current?.parentElement?.getBoundingClientRect().right ?? 0;
        paint(right - event.clientX);
      }}
      onPointerUp={(event) => {
        if (!dragging.current) return;
        endDrag(event.currentTarget);
        commit(shown.current);
      }}
      onLostPointerCapture={(event) => {
        if (!dragging.current) return;
        endDrag(event.currentTarget);
        commit(shown.current);
      }}
      onDoubleClick={() => {
        wanted.current = null;
        saveWidth(null);
        repaint();
      }}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft") commit(shown.current + KEY_STEP);
        else if (event.key === "ArrowRight") commit(shown.current - KEY_STEP);
        else return;
        event.preventDefault();
      }}
      className="group relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize touch-none outline-none select-none"
    >
      <span
        aria-hidden
        className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-line transition-colors group-hover:w-0.5 group-hover:bg-accent/60 group-focus-visible:w-0.5 group-focus-visible:bg-accent group-data-[dragging]:w-0.5 group-data-[dragging]:bg-accent"
      />
    </div>
  );
}
