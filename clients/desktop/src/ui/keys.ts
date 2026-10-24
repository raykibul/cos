// Telling whether a key press is free for a page shortcut.

/** Whether a key pressed in `target` is free for the page: not typed into anything. */
export function keysAreFree(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return true;
  if (target.closest(".terminal-host")) return false;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    return target.value === "";
  }
  return !target.isContentEditable;
}
