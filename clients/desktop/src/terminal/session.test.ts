import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toBase64 } from "../api/client";
import { CosError } from "../api/errors";
import type { TerminalMessage } from "../bridge/types";
import { MAX_KEY_BATCH, type TerminalBackend, TerminalSession, type TerminalSink } from "./session";

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);
const b64 = (s: string) => toBase64(new TextEncoder().encode(s));

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A backend whose requests finish when the test says. */
class FakeBackend implements TerminalBackend {
  listener: ((m: TerminalMessage) => void) | null = null;
  attaches = 0;
  closed: number[] = [];
  keys: { bytes: string; done: ReturnType<typeof deferred> }[] = [];
  resizes: { rows: number; cols: number }[] = [];
  resizeError: unknown = null;

  attach(_agentId: number, onMessage: (m: TerminalMessage) => void): Promise<number> {
    this.attaches += 1;
    this.listener = onMessage;
    return Promise.resolve(this.attaches);
  }
  close(id: number): Promise<void> {
    this.closed.push(id);
    return Promise.resolve();
  }
  sendKeys(_agentId: number, bytes: Uint8Array): Promise<void> {
    const done = deferred();
    this.keys.push({ bytes: text(bytes), done });
    return done.promise;
  }
  resize(_agentId: number, rows: number, cols: number): Promise<void> {
    this.resizes.push({ rows, cols });
    return this.resizeError ? Promise.reject(this.resizeError) : Promise.resolve();
  }
  emit(message: TerminalMessage) {
    this.listener?.(message);
  }
}

class RecordingSink implements TerminalSink {
  log: string[] = [];
  reset(size: { rows: number; cols: number }) {
    this.log.push(`reset ${size.cols}x${size.rows}`);
  }
  write(bytes: Uint8Array) {
    this.log.push(`write ${text(bytes)}`);
  }
  resize(size: { rows: number; cols: number }) {
    this.log.push(`resize ${size.cols}x${size.rows}`);
  }
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("TerminalSession", () => {
  let backend: FakeBackend;
  let sink: RecordingSink;
  let session: TerminalSession;

  beforeEach(() => {
    backend = new FakeBackend();
    sink = new RecordingSink();
    session = new TerminalSession(7, backend, 50);
    session.sink = sink;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("writes the snapshot, then output, and resets on a new snapshot", async () => {
    session.start();
    await flush();
    expect(session.phase.kind).toBe("connecting");

    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: b64("hello") });
    backend.emit({ type: "output", data: b64(" world") });
    expect(session.phase.kind).toBe("live");

    backend.emit({ type: "disconnected", reason: "daemon restarted" });
    expect(session.phase).toEqual({ kind: "reconnecting", reason: "daemon restarted" });

    backend.emit({ type: "snapshot", rows: 30, cols: 100, data: b64("again") });
    expect(sink.log).toEqual([
      "reset 80x24",
      "write hello",
      "write  world",
      "reset 100x30",
      "write again",
    ]);
    expect(session.snapshots).toBe(2);
    expect(session.phase.kind).toBe("live");
  });

  it("follows the PTY's resizes and its exit", async () => {
    session.start();
    await flush();
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    backend.emit({ type: "resize", rows: 24, cols: 80 }); // same size: ignored
    backend.emit({ type: "resize", rows: 40, cols: 120 });
    backend.emit({ type: "exit", exitCode: 0 });
    expect(sink.log.filter((l) => l.startsWith("resize"))).toEqual(["resize 120x40"]);
    expect(session.phase).toEqual({ kind: "exited", code: 0 });
  });

  it("ignores messages from a stream it stopped", async () => {
    session.start();
    await flush();
    const old = backend.listener!;
    session.restart();
    await flush();
    old({ type: "output", data: b64("stale") });
    expect(sink.log).toEqual([]);
    expect(backend.closed).toEqual([1]);
    expect(backend.attaches).toBe(2);
  });

  it("sends keys in order, batching what's typed while a send is in flight", async () => {
    session.start();
    await flush();
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });

    session.send("a");
    session.send("b");
    session.send("c");
    expect(backend.keys.map((k) => k.bytes)).toEqual(["a"]);

    backend.keys[0]!.done.resolve();
    await flush();
    expect(backend.keys.map((k) => k.bytes)).toEqual(["a", "bc"]);

    session.send("d");
    backend.keys[1]!.done.resolve();
    await flush();
    expect(backend.keys.map((k) => k.bytes)).toEqual(["a", "bc", "d"]);
  });

  it("splits a large paste into ordered batches", async () => {
    session.start();
    await flush();
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    session.send("x");
    session.send("y".repeat(MAX_KEY_BATCH + 10));
    backend.keys[0]!.done.resolve();
    await flush();
    expect(backend.keys[1]!.bytes).toHaveLength(MAX_KEY_BATCH);
    backend.keys[1]!.done.resolve();
    await flush();
    expect(backend.keys[2]!.bytes).toBe("y".repeat(10));
  });

  it("drops queued keys once the agent is gone", async () => {
    session.start();
    await flush();
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    session.send("a");
    session.send("b");
    backend.keys[0]!.done.reject(new CosError("conflict", "The agent exited", 409));
    await flush();
    expect(backend.keys).toHaveLength(1);
    expect(session.lastError).toBe("The agent exited");
  });

  it("sends nothing before the first snapshot", async () => {
    session.start();
    await flush();
    session.send("early");
    expect(backend.keys).toHaveLength(0);
  });

  it("resizes the agent to fit the view once, after the view settles", async () => {
    vi.useFakeTimers();
    session.start();
    await vi.advanceTimersByTimeAsync(0);
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    session.viewSizeChanged({ rows: 30, cols: 90 });
    session.viewSizeChanged({ rows: 31, cols: 95 });
    session.viewSizeChanged({ rows: 32, cols: 100 });
    expect(backend.resizes).toEqual([]);
    await vi.advanceTimersByTimeAsync(60);
    expect(backend.resizes).toEqual([{ rows: 32, cols: 100 }]);
    expect(session.ptySize).toEqual({ rows: 32, cols: 100 });
    expect(sink.log.at(-1)).toBe("resize 100x32");

    // Already that size: nothing to send.
    session.viewSizeChanged({ rows: 32, cols: 100 });
    await vi.advanceTimersByTimeAsync(60);
    expect(backend.resizes).toHaveLength(1);
  });

  it("re-claims the size after a reconnect's snapshot", async () => {
    vi.useFakeTimers();
    session.start();
    await vi.advanceTimersByTimeAsync(0);
    session.viewSizeChanged({ rows: 30, cols: 90 });
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    await vi.advanceTimersByTimeAsync(60);
    expect(backend.resizes).toEqual([{ rows: 30, cols: 90 }]);
  });

  it("notifies subscribers of phase changes", async () => {
    const phases: string[] = [];
    session.subscribe(() => phases.push(session.getPhase().kind));
    session.start();
    await flush();
    backend.emit({ type: "snapshot", rows: 24, cols: 80, data: "" });
    backend.emit({ type: "exit", exitCode: null });
    expect(phases).toEqual(["connecting", "live", "exited"]);
  });

  it("fails when the stream can't open", async () => {
    backend.attach = () => Promise.reject({ kind: "io", message: "no socket" });
    session.start();
    await flush();
    expect(session.phase).toEqual({ kind: "failed", message: "no socket" });
  });
});
