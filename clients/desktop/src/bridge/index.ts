import type { FakeOptions } from "./fake";
import type { Bridge } from "./types";

export type * from "./types";

/** Inside Tauri, `window.__TAURI_INTERNALS__` is set. */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * The Tauri bridge inside the app, else the fake with canned data, so the UI
 * runs in a plain browser. Each is loaded only when picked.
 */
export async function createBridge(fake: FakeOptions = {}): Promise<Bridge> {
  if (inTauri()) {
    const { TauriBridge } = await import("./tauri");
    return new TauriBridge();
  }
  const { FakeBridge } = await import("./fake");
  return new FakeBridge(fake);
}
