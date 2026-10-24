// The bridge to the daemon, as in docs/desktop.md. The UI never talks to the
// socket: the Tauri shell does, and in a plain browser the fake does.

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

/** `daemon_request`'s result. Any HTTP status is a normal result. */
export interface DaemonResponse {
  status: number;
  /** Parsed JSON, a string for a non-JSON answer, or null when empty. */
  body: unknown;
}

/** `daemon_fetch_bytes`'s result: a file the daemon serves, such as an attachment. */
export interface FileResponse {
  status: number;
  contentType: string;
  /** The bytes, base64. */
  data: string;
}

/** `GET /v1/status`, as the daemon sends it (snake_case). */
export type RawStatus = Record<string, unknown>;
/** One log event, as the daemon sends it. */
export type RawEvent = Record<string, unknown>;
/** `GET /v1/tailnet/peers`, as the daemon sends it: `{peers, self_name, reason}`. */
export type RawTailnetPeers = unknown;

export type EventMessage =
  | { type: "connected"; status: RawStatus }
  | { type: "event"; event: RawEvent }
  | { type: "log_reset" }
  | {
      type: "disconnected";
      reason: string;
      /** Nothing answers: no socket, or the host can't be reached. */
      notRunning: boolean;
      /** A remote daemon's refusal (`device_pending`, …); null otherwise. */
      code?: string | null;
    };

export type TerminalMessage =
  | { type: "snapshot"; rows: number; cols: number; data: string }
  | { type: "output"; data: string }
  | { type: "resize"; rows: number; cols: number }
  | { type: "exit"; exitCode: number | null }
  | { type: "disconnected"; reason: string; code?: string | null };

export type BridgeErrorKind = "not_running" | "io" | "bad_request";

/** What a rejected command carries. */
export interface BridgeError {
  kind: BridgeErrorKind;
  message: string;
  /** A daemon's refusal code (`host_only`, …), when it gave one. */
  code?: string;
}

export function isBridgeError(value: unknown): value is BridgeError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as BridgeError).kind === "string" &&
    typeof (value as BridgeError).message === "string"
  );
}

/** This Mac's id in the host list. */
export const THIS_MAC = "local";

/** A machine whose daemon the app can talk to: this Mac, or another machine over Tailscale. */
export interface Host {
  id: string;
  name: string;
  /** `host:port` over Tailscale; null for this Mac (its socket). */
  address: string | null;
}

export interface HostList {
  /** This Mac first, then the saved ones. */
  hosts: Host[];
  /** The id of the host in use. */
  current: string;
}

export interface DaemonInfo {
  /** This Mac's socket and home, whichever host is in use. */
  socket: string;
  home: string;
  /** The host in use; missing from shells older than hosts. */
  host?: Host;
}

export interface Bridge {
  /** "tauri" inside the app, "fake" in a plain browser. */
  readonly kind: "tauri" | "fake";
  daemonInfo(): Promise<DaemonInfo>;
  daemonRequest(method: HttpMethod, path: string, body?: unknown): Promise<DaemonResponse>;
  /** Uploads a file: `POST path` with `data` (base64) as the body. Any status is a normal result. */
  daemonUpload(path: string, contentType: string, data: string): Promise<DaemonResponse>;
  /** Fetches a file the daemon serves, as base64. Rejects with `io` on a non-2xx answer. */
  daemonFetchBytes(path: string): Promise<FileResponse>;
  /** Follows the event log; returns a stream id for `closeStream`. */
  followEvents(after: number | null, onMessage: (message: EventMessage) => void): Promise<number>;
  attachTerminal(agentId: number, onMessage: (message: TerminalMessage) => void): Promise<number>;
  closeStream(id: number): Promise<void>;
  hostsList(): Promise<HostList>;
  /** Saves a host without switching to it. */
  hostAdd(name: string, address: string): Promise<Host>;
  /** Forgets a host; removing the one in use switches to this Mac. */
  hostRemove(id: string): Promise<HostList>;
  /**
   * Renames a host, This Mac included (its id stays `local`). The name is
   * trimmed, at most 60 characters and unique among the hosts; anything
   * else is `bad_request`.
   */
  hostRename(id: string, name: string): Promise<HostList>;
  /** Switches host. Every stream closes. */
  hostSelect(id: string): Promise<HostList>;
  /**
   * `GET /v1/tailnet/peers` on **This Mac's** daemon, whichever host is in
   * use (the route answers on the socket only). Rejects with `not_running`
   * when that daemon is down, and with `io` for a non-2xx answer.
   */
  tailnetPeers(port?: number): Promise<RawTailnetPeers>;
  setWaitingCount(count: number): Promise<void>;
  notify(title: string, body: string): Promise<void>;
  revealPath(path: string): Promise<void>;
  openInTerminal(path: string): Promise<void>;
}
