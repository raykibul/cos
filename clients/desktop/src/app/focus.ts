import { type RefObject, useEffect } from "react";
import { useApp, useUi } from "./context";

/**
 * Puts the cursor in a page's input whenever the page is opened: on mount,
 * and again when you open it while it's showing. It doesn't scroll, so an
 * input low on the page stays where it is, and it leaves an open dialog alone.
 */
export function useFocusOnOpen(ref: RefObject<HTMLElement | null>, enabled = true) {
  const { ui } = useApp();
  const opened = useUi((s) => s.opened);
  useEffect(() => {
    if (!enabled || ui.store.getState().dialog) return;
    ref.current?.focus({ preventScroll: true });
  }, [ui, opened, enabled, ref]);
}
