// One agent's live terminal without any DOM: a port of CosKit's
// TerminalSession. It feeds the stream into a sink (the emulator), sends
// keystrokes in order (batching what's typed while a send is in flight),
// and resizes the agent to fit the view, debounced.

import { fromBase64 } from "../api/client";
import { CosError } from "../api/errors";
import type { TerminalMessage } from "../bridge/types";

export interface TerminalSize {
  rows: number;
  cols: number;
}

/** The emulator a session drives. */
export interface TerminalSink {
  /** Clear everything and switch to this size; a snapshot's bytes follow. */
  reset(size: TerminalSize): void;
  write(bytes: Uint8Array): void;
  /** The agent's PTY changed size. */
  resize(size: TerminalSize): void;
}

/** Where the stream comes from and where keys go: the bridge and the client. */
export interface TerminalBackend {
  attach(agentId: number, onMessage: (message: TerminalMessage) => void): Promise<number>;
  close(streamId: number): Promise<void>;
  sendKeys(agentId: number, bytes: Uint8Array): Promise<void>;
  resize(agentId: number, rows: number, cols: number): Promise<void>;
}

export type TerminalPhase =
  | { kind: "connecting" }
  | { kind: "live" }
  /** The stream dropped while the agent was alive; the bridge retries. */
  | { kind: "reconnecting"; reason: string }
  /** The process ended. null: killed or interrupted. */
  | { kind: "exited"; code: number | null }
  /** The stream couldn't be opened. */
  | { kind: "failed"; message: string };

/** Largest keystroke batch per request; a big paste goes in several, in order. */
export const MAX_KEY_BATCH = 32 * 1024;
export const RESIZE_DELAY_MS = 150;

const sameSize = (a: TerminalSize | null, b: TerminalSize | null) =>
  a !== null && b !== null && a.rows === b.rows && a.cols === b.cols;

export class TerminalSession {
  phase: TerminalPhase = { kind: "connecting" };
  /** The PTY's size as the daemon last said, or as we last set it. */
  ptySize: TerminalSize | null = null;
  /** The size that fits the view, as the view last reported. */
  viewSize: TerminalSize | null = null;
  /** Why the last keystroke or resize didn't go through. */
  lastError: string | null = null;
  /** How many snapshots arrived: 1 after the first connect, more after reconnects. */
  snapshots = 0;
  sink: TerminalSink | null = null;

  private streamId: number | null = null;
  private generation = 0;
  private pending: Uint8Array[] = [];
  private pendingBytes = 0;
  private sending = false;
  private resizeTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly listeners = new Set<() => void>();
  private readonly encoder = new TextEncoder();

  constructor(
    readonly agentId: number,
    private readonly backend: TerminalBackend,
    private readonly resizeDelayMs = RESIZE_DELAY_MS,
  ) {}

  /** For React's useSyncExternalStore. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getPhase = (): TerminalPhase => this.phase;

  private setPhase(phase: TerminalPhase): void {
    this.phase = phase;
    for (const listener of this.listeners) listener();
  }

  get isRunning(): boolean {
    return this.streamId !== null || this.generation > 0;
  }

  /** Opens the stream. The bridge reconnects by itself; each reconnect starts with a snapshot. */
  start(): void {
    const generation = ++this.generation;
    this.setPhase({ kind: "connecting" });
    this.backend
      .attach(this.agentId, (message) => {
        if (generation === this.generation) this.handle(message);
      })
      .then(
        (id) => {
          if (generation === this.generation) this.streamId = id;
          else void this.backend.close(id);
        },
        (error) => {
          if (generation === this.generation) {
            this.setPhase({ kind: "failed", message: CosError.from(error).message });
          }
        },
      );
  }

  stop(): void {
    this.generation += 1;
    if (this.streamId !== null) void this.backend.close(this.streamId).catch(() => undefined);
    this.streamId = null;
    this.cancelResize();
  }

  /** A fresh stream, after a resume or when the emulator was rebuilt. */
  restart(): void {
    this.stop();
    this.start();
  }

  handle(message: TerminalMessage): void {
    switch (message.type) {
      case "snapshot": {
        const size = { rows: message.rows, cols: message.cols };
        this.ptySize = size;
        this.snapshots += 1;
        this.sink?.reset(size);
        this.sink?.write(fromBase64(message.data));
        this.setPhase({ kind: "live" });
        // We just (re)connected: make the agent fit this view.
        this.scheduleResize();
        break;
      }
      case "output":
        this.sink?.write(fromBase64(message.data));
        break;
      case "resize": {
        const size = { rows: message.rows, cols: message.cols };
        if (sameSize(size, this.ptySize)) return;
        this.ptySize = size;
        this.sink?.resize(size);
        break;
      }
      case "exit":
        this.cancelResize();
        this.setPhase({ kind: "exited", code: message.exitCode });
        break;
      case "disconnected":
        this.setPhase({ kind: "reconnecting", reason: message.reason });
        break;
    }
  }

  // MARK: Input

  /**
   * Keystrokes from the view. They go out in order; those typed while a
   * request is in flight go together in the next one.
   */
  send(data: string | Uint8Array): void {
    const bytes = typeof data === "string" ? this.encoder.encode(data) : data;
    if (bytes.length === 0) return;
    if (this.phase.kind !== "live" && this.phase.kind !== "reconnecting") return;
    this.pending.push(bytes);
    this.pendingBytes += bytes.length;
    // Typing here means this view is the one in use: let it own the size.
    if (this.viewSize && !sameSize(this.viewSize, this.ptySize)) this.scheduleResize();
    if (this.sending) return;
    this.sending = true;
    void this.drain();
  }

  private takeBatch(): Uint8Array {
    const size = Math.min(this.pendingBytes, MAX_KEY_BATCH);
    const batch = new Uint8Array(size);
    let offset = 0;
    while (offset < size) {
      const head = this.pending[0]!;
      const take = Math.min(head.length, size - offset);
      batch.set(head.subarray(0, take), offset);
      offset += take;
      if (take === head.length) this.pending.shift();
      else this.pending[0] = head.subarray(take);
    }
    this.pendingBytes -= size;
    return batch;
  }

  private async drain(): Promise<void> {
    while (this.pendingBytes > 0) {
      const batch = this.takeBatch();
      try {
        await this.backend.sendKeys(this.agentId, batch);
        this.lastError = null;
      } catch (error) {
        const failure = CosError.from(error);
        this.lastError = failure.message;
        // It exited or went away: what's queued can't arrive either.
        if (failure.kind === "conflict" || failure.kind === "not_found") {
          this.pending = [];
          this.pendingBytes = 0;
        }
      }
    }
    this.sending = false;
  }

  // MARK: Size

  /**
   * The view's size in cells changed. The agent is resized to match after
   * the delay, so a live resize sends one request at the end. Reporting the
   * same size again still re-claims the PTY if someone else resized it.
   */
  viewSizeChanged(size: TerminalSize): void {
    if (size.rows <= 0 || size.cols <= 0) return;
    this.viewSize = size;
    this.scheduleResize();
  }

  private cancelResize(): void {
    if (this.resizeTimer !== null) clearTimeout(this.resizeTimer);
    this.resizeTimer = null;
  }

  private scheduleResize(): void {
    this.cancelResize();
    const target = this.viewSize;
    if (this.phase.kind !== "live" || !target || sameSize(target, this.ptySize)) return;
    this.resizeTimer = setTimeout(() => {
      this.resizeTimer = null;
      void this.sendResize(target);
    }, this.resizeDelayMs);
  }

  private async sendResize(size: TerminalSize): Promise<void> {
    if (
      this.phase.kind !== "live" ||
      !sameSize(size, this.viewSize) ||
      sameSize(size, this.ptySize)
    ) {
      return;
    }
    try {
      await this.backend.resize(this.agentId, size.rows, size.cols);
      // The daemon echoes a resize event; match it now so the emulator and
      // the PTY agree without waiting.
      if (!sameSize(this.ptySize, size)) {
        this.ptySize = size;
        this.sink?.resize(size);
      }
      this.lastError = null;
    } catch (error) {
      this.lastError = CosError.from(error).message;
    }
  }
}
