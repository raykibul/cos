import { createContext, useContext, useEffect, useState } from "react";
import { useStore } from "zustand";
import { useShallow } from "zustand/react/shallow";
import type { CosStore, DaemonState } from "../store/store";
import type { UiState, UiStore } from "./ui";

export interface AppContextValue {
  cos: CosStore;
  ui: UiStore;
}

export const AppContext = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const value = useContext(AppContext);
  if (!value) throw new Error("useApp outside <AppContext>");
  return value;
}

/** Reads the daemon state. Arrays and objects compare shallowly, so derived lists don't loop. */
export function useCos<T>(selector: (state: DaemonState) => T): T {
  const { cos } = useApp();
  return useStore(cos.state, useShallow(selector));
}

export function useUi<T>(selector: (state: UiState) => T): T {
  const { ui } = useApp();
  return useStore(ui.store, useShallow(selector));
}

/** Whether changes can be made now: the daemon answers. */
export function useCanChange(): boolean {
  return useCos((s) => s.connection === "running");
}

/** The current time, ticking, for "3 min ago" labels. */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}
