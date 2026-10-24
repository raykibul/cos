import { useSyncExternalStore } from "react";
import { SIDEBAR_AUTO_MIN_WIDTH, sidebarShown } from "./ui";
import { useUi } from "./context";

const query = `(max-width: ${SIDEBAR_AUTO_MIN_WIDTH - 1}px)`;

function media(): MediaQueryList | null {
  return typeof window.matchMedia === "function" ? window.matchMedia(query) : null;
}

function subscribe(onChange: () => void): () => void {
  const list = media();
  list?.addEventListener?.("change", onChange);
  return () => list?.removeEventListener?.("change", onChange);
}

/** Whether the window is too narrow for the sidebar to show by default. */
export function useNarrowWindow(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => media()?.matches ?? false,
    () => false,
  );
}

/** Whether the sidebar shows now, and whether the window is narrow (for toggling it). */
export function useSidebar(): { shown: boolean; narrow: boolean } {
  const narrow = useNarrowWindow();
  const mode = useUi((s) => s.sidebar);
  return { shown: sidebarShown(mode, narrow), narrow };
}
