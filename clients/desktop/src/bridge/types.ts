// The bridge to the daemon, as in docs/desktop.md. The UI never talks to the
// socket: the Tauri shell does, and in a plain browser the fake does.

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

/** `daemon_request`'s result. Any HTTP status is a normal result. */
export interface DaemonResponse {
  status: number;
  /** Parsed JSON, a string for a non-JSON answer, or null when empty. */
  body: unknown;
}

/** `GET /v1/status`, as the daemon sends it (snake_case). */
export type RawStatus = Record<string, unknown>;
/** One log event, as the daemon sends it. */
export type RawEvent = Record<string, unknown>;

export type EventMessage =
  | { type: "connected"; status: RawStatus }
  | { type: "event"; event: RawEvent }
  | { type: "log_reset" }
  | { type: "disconnected"; reason: string; notRunning: boolean };

export type TerminalMessage =
  | { type: "snapshot"; rows: number; cols: number; data: string }
  | { type: "output"; data: string }
  | { type: "resize"; rows: number; cols: number }
  | { type: "exit"; exitCode: number | null }
  | { type: "disconnected"; reason: string };

export type BridgeErrorKind = "not_running" | "io" | "bad_request";

/** What a rejected command carries. */
export interface BridgeError {
  kind: BridgeErrorKind;
  message: string;
}

export function isBridgeError(value: unknown): value is BridgeError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as BridgeError).kind === "string" &&
    typeof (value as BridgeError).message === "string"
  );
}

export interface DaemonInfo {
  socket: string;
  home: string;
}

export interface Bridge {
  /** "tauri" inside the app, "fake" in a plain browser. */
  readonly kind: "tauri" | "fake";
  daemonInfo(): Promise<DaemonInfo>;
  daemonRequest(method: HttpMethod, path: string, body?: unknown): Promise<DaemonResponse>;
  /** Follows the event log; returns a stream id for `closeStream`. */
  followEvents(after: number | null, onMessage: (message: EventMessage) => void): Promise<number>;
  attachTerminal(agentId: number, onMessage: (message: TerminalMessage) => void): Promise<number>;
  closeStream(id: number): Promise<void>;
  setWaitingCount(count: number): Promise<void>;
  notify(title: string, body: string): Promise<void>;
  revealPath(path: string): Promise<void>;
  openInTerminal(path: string): Promise<void>;
}
