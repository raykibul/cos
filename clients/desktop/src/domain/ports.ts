// Ports forwarded from another machine (docs/desktop.md, "Ports"): checking
// a typed port, and how a forward's state reads in the sidebar. Pure, so
// it's tested without the section.

import type { Forward } from "../bridge/types";

/** A typed port: the number once it is one, or why it isn't. Empty is neither. */
export type PortCheck =
  { port: number; problem: null } | { port: null; problem: string } | { port: null; problem: null };

function checkNumber(text: string): PortCheck {
  const trimmed = text.trim();
  if (!trimmed) return { port: null, problem: null };
  if (!/^\d+$/.test(trimmed)) return { port: null, problem: "Type a port number." };
  const port = Number(trimmed);
  if (port < 1 || port > 65535) return { port: null, problem: "A port is 1–65535." };
  return { port, problem: null };
}

/** The port to forward from the machine: 1–65535 and not forwarded yet. */
export function checkRemotePort(text: string, forwards: Forward[]): PortCheck {
  const check = checkNumber(text);
  if (check.port !== null && forwards.some((f) => f.remotePort === check.port)) {
    return { port: null, problem: `:${check.port} is already forwarded.` };
  }
  return check;
}

/** The port on this Mac, behind More: 1–65535 and not used by another forward. */
export function checkLocalPort(text: string, forwards: Forward[]): PortCheck {
  const check = checkNumber(text);
  const user = check.port !== null ? forwards.find((f) => f.localPort === check.port) : undefined;
  if (user) return { port: null, problem: `localhost:${user.localPort} is :${user.remotePort}.` };
  return check;
}

export type PortTone = "green" | "amber" | "red";

/**
 * Green when something runs there; amber when nothing does, or nobody has
 * tried yet; red when the machine refused, can't be reached, or this Mac
 * couldn't listen.
 */
export function portTone(forward: Forward): PortTone {
  if (forward.state === "error") return "red";
  switch (forward.remote) {
    case "listening":
      return "green";
    case "nothing_listening":
    case "unknown":
      return "amber";
    case "refused":
    case "unreachable":
      return "red";
  }
}

/** A word or two after the port when it isn't simply working. */
export function portLabel(forward: Forward): string | null {
  if (forward.state === "error") return "not listening here";
  switch (forward.remote) {
    case "nothing_listening":
      return "not running";
    case "refused":
      return forward.code === "too_old" ? "too old" : "refused";
    case "unreachable":
      return "can't reach";
    default:
      return null;
  }
}

/** Whether this Mac's port differs from the machine's, because it was busy here. */
export function moved(forward: Forward): boolean {
  return forward.localPort !== forward.remotePort;
}

/** The row's tooltip: where it goes, the latest problem or the moved-port note. */
export function portTitle(forward: Forward, machine: string): string {
  const where = `localhost:${forward.localPort} on this Mac → localhost:${forward.remotePort} on ${machine}`;
  return forward.message ? `${where}\n${forward.message}` : where;
}
