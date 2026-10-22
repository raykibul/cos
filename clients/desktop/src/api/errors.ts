import { isBridgeError } from "../bridge/types";

export type CosErrorKind =
  /** Nothing answers on the socket. Expected whenever the daemon is stopped. */
  | "not_running"
  /** The bridge couldn't talk to the daemon for another reason. */
  | "io"
  /** The bridge refused the request (a path outside /v1/, say). */
  | "bad_request"
  /** 404: an unknown id, or a daemon without that route yet. */
  | "not_found"
  /** 409: the move isn't allowed now. */
  | "conflict"
  /** Any other non-2xx answer. */
  | "http"
  /** The answer wasn't what the route promises. */
  | "bad_response";

/**
 * An error from the daemon or the bridge. `message` is written to be shown
 * as-is: the daemon writes its `{"error": …}` messages for people.
 */
export class CosError extends Error {
  readonly kind: CosErrorKind;
  readonly status: number | null;
  /**
   * A remote daemon's refusal (`device_pending`, `host_only`, …; see
   * "Refusals" in docs/daemon.md), or null.
   */
  readonly code: string | null;

  constructor(
    kind: CosErrorKind,
    message: string,
    status: number | null = null,
    code: string | null = null,
  ) {
    super(message);
    this.name = "CosError";
    this.kind = kind;
    this.status = status;
    this.code = code;
  }

  static fromResponse(status: number, body: unknown, path: string): CosError {
    const rawCode =
      typeof body === "object" && body !== null ? (body as { code?: unknown }).code : undefined;
    const code = typeof rawCode === "string" ? rawCode : null;
    if (code) return new CosError("http", errorText(body, status, path), status, code);
    return CosError.plain(status, errorText(body, status, path));
  }

  private static plain(status: number, message: string): CosError {
    if (status === 404) return new CosError("not_found", message, status);
    if (status === 409) return new CosError("conflict", message, status);
    return new CosError("http", message, status);
  }

  /** A rejected bridge command, or anything else thrown along the way. */
  static from(error: unknown): CosError {
    if (error instanceof CosError) return error;
    if (isBridgeError(error)) return new CosError(error.kind, error.message);
    if (error instanceof Error) return new CosError("io", error.message);
    if (typeof error === "string") return new CosError("io", error);
    return new CosError("io", "Something went wrong talking to cos.");
  }
}

/** The daemon's `{"error": …}`, the body as text, or a plain description. */
function errorText(body: unknown, status: number, path: string): string {
  if (
    typeof body === "object" &&
    body !== null &&
    typeof (body as { error?: unknown }).error === "string"
  ) {
    return (body as { error: string }).error;
  }
  if (typeof body === "string" && body.trim() !== "") return body.trim();
  return `The cos daemon answered ${status} for ${path}.`;
}

export function isNotFound(error: unknown): boolean {
  return error instanceof CosError && error.kind === "not_found";
}

export function isConflict(error: unknown): boolean {
  return error instanceof CosError && error.kind === "conflict";
}

export function messageOf(error: unknown): string {
  return CosError.from(error).message;
}
