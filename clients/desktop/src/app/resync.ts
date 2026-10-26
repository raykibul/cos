// The event stream says when something changed, but an event can be missed
// (a connection that died quietly, a reply Claude wrote after its turn
// ended). So the app also catches up when its window comes back, and every
// so often while it's shown. Each catch-up reads only what's new.

/** How often to catch up while the window is shown. */
export const RESYNC_MS = 15_000;

/** Calls `cos.resync()` on focus, on becoming visible, and every `everyMs` while visible. */
export function followWindow(
  cos: { resync(): void },
  win: Window = window,
  everyMs = RESYNC_MS,
): () => void {
  const doc = win.document;
  const shown = () => doc.visibilityState !== "hidden";
  const onFocus = () => cos.resync();
  const onVisibility = () => {
    if (shown()) cos.resync();
  };
  win.addEventListener("focus", onFocus);
  doc.addEventListener("visibilitychange", onVisibility);
  const timer = win.setInterval(() => {
    if (shown()) cos.resync();
  }, everyMs);
  return () => {
    win.removeEventListener("focus", onFocus);
    doc.removeEventListener("visibilitychange", onVisibility);
    win.clearInterval(timer);
  };
}
