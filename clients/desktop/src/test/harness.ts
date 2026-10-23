// Shared helpers: a store on the fake bridge with no latency and no
// simulated activity, and a way to let pending timers and promises run.

import { FakeBridge, type FakeOptions } from "../bridge/fake";
import { CosStore, type StoreOptions } from "../store/store";

/** Lets queued timers (0 ms) and promise chains run. */
export async function settle(rounds = 8, ms = 0): Promise<void> {
  for (let i = 0; i < rounds; i++) await new Promise((resolve) => setTimeout(resolve, ms));
}

export function fakeStore(fake: FakeOptions = {}, store: StoreOptions = {}) {
  const bridge = new FakeBridge({ latencyMs: 0, simulate: false, ...fake });
  const cos = new CosStore(bridge, store);
  return { bridge, daemon: bridge.daemon, cos };
}

/** `GET /v1/projects`, for each request the fake daemon saw. */
export function requestLines(daemon: FakeBridge["daemon"]): string[] {
  return daemon.requests.map((r) => `${r.method} ${r.path}`);
}

/** A raw log event as the daemon sends it. */
export function rawEvent(seq: number, kind: string, data: Record<string, unknown> = {}) {
  return {
    seq,
    ts: new Date().toISOString(),
    kind,
    run_id: data.run_id !== undefined ? String(data.run_id) : null,
    data,
  };
}
