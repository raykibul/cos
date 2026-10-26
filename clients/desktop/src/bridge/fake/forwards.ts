// Canned port forwards for the fake bridge (docs/desktop.md, "Ports"). They
// belong to a host, as the shell keeps them per host in hosts.json. Studio,
// once it lets this Mac in, forwards 5173 (a dev server, two connections
// open) and 3343, which was busy on this Mac and so moved to 3344, with
// nothing running there.

import type { BridgeError, Forward, ForwardMessage } from "../types";

/** Ports something listens on in the fake hosts. */
const RUNNING = new Set([3000, 5173, 8080]);
/** Ports already taken on this Mac, so a forward moves up from them. */
const BUSY_HERE = new Set([3343]);

export function tooOld(machine: string): BridgeError {
  return {
    kind: "io",
    code: "too_old",
    message: `${machine}'s cos is too old to forward ports. Run \`cos upgrade\` there.`,
  };
}

function movedNote(wanted: number, local: number): string {
  return (
    `localhost:${wanted} was taken on this Mac, so this is on ${local}. ` +
    `OAuth redirects and links that name :${wanted} won't reach it.`
  );
}

function nothingListening(port: number, machine: string): string {
  return `Nothing is listening on ${port} on ${machine}.`;
}

interface Follower {
  hostId: string;
  machine: () => string;
  onMessage: (message: ForwardMessage) => void;
}

/** Every host's forwards, and the streams following them. */
export class FakeForwards {
  private readonly byHost = new Map<string, Forward[]>();
  private readonly seeded = new Set<string>();
  private readonly followers = new Map<number, Follower>();
  /** The URLs `forward_open` opened, for tests. */
  readonly opened: string[] = [];

  /** Studio's canned forwards, the first time it lets this Mac in. */
  admitted(hostId: string, machine: string): void {
    if (this.seeded.has(hostId)) return;
    this.seeded.add(hostId);
    if (hostId !== "h1") return;
    const forwards = this.of(hostId);
    forwards.push(
      {
        remotePort: 3343,
        localPort: 3344,
        state: "listening",
        remote: "nothing_listening",
        code: null,
        message: nothingListening(3343, machine),
        connections: 0,
      },
      {
        remotePort: 5173,
        localPort: 5173,
        state: "listening",
        remote: "listening",
        code: null,
        message: null,
        connections: 2,
      },
    );
    this.changed(hostId);
  }

  list(hostId: string): Forward[] {
    return this.of(hostId).map((f) => ({ ...f }));
  }

  get(hostId: string, remotePort: number): Forward | undefined {
    return this.of(hostId).find((f) => f.remotePort === remotePort);
  }

  /**
   * Forwards `remotePort`, as `forward_add` does once the host was asked:
   * `reach` is null when it answered, or why it couldn't be reached.
   */
  add(
    hostId: string,
    machine: string,
    remotePort: number,
    localPort: number | undefined,
    reach: string | null,
  ): Forward {
    for (const port of [remotePort, localPort]) {
      if (port !== undefined && !(Number.isInteger(port) && port >= 1 && port <= 65535)) {
        throw {
          kind: "bad_request",
          message: `${port} isn't a port (1–65535)`,
        } satisfies BridgeError;
      }
    }
    const forwards = this.of(hostId);
    if (forwards.some((f) => f.remotePort === remotePort)) {
      throw {
        kind: "bad_request",
        message: `:${remotePort} is already forwarded`,
      } satisfies BridgeError;
    }
    const wanted = localPort ?? remotePort;
    let local = wanted;
    const taken = new Set([...BUSY_HERE, ...forwards.map((f) => f.localPort)]);
    while (taken.has(local)) local += 1;
    let remote: Forward["remote"] = "listening";
    let message: string | null = null;
    if (reach !== null) {
      remote = "unreachable";
      message = reach;
    } else if (!RUNNING.has(remotePort)) {
      remote = "nothing_listening";
      message = nothingListening(remotePort, machine);
    }
    const forward: Forward = {
      remotePort,
      localPort: local,
      state: "listening",
      remote,
      code: null,
      message: message ?? (local !== wanted ? movedNote(wanted, local) : null),
      connections: 0,
    };
    forwards.push(forward);
    forwards.sort((a, b) => a.remotePort - b.remotePort);
    this.changed(hostId);
    return { ...forward };
  }

  remove(hostId: string, remotePort: number): Forward[] {
    const forwards = this.of(hostId);
    const index = forwards.findIndex((f) => f.remotePort === remotePort);
    if (index >= 0) {
      forwards.splice(index, 1);
      this.changed(hostId);
    }
    return this.list(hostId);
  }

  /** Changes a forward by hand, as browser connections would: for tests and the console. */
  set(hostId: string, remotePort: number, change: Partial<Forward>): void {
    const forward = this.get(hostId, remotePort);
    if (!forward) return;
    Object.assign(forward, change);
    this.changed(hostId);
  }

  follow(id: number, follower: Follower): void {
    this.followers.set(id, follower);
    follower.onMessage(this.message(follower));
  }

  unfollow(id: number): void {
    this.followers.delete(id);
  }

  private message(follower: Follower): ForwardMessage {
    return { type: "forwards", host: follower.machine(), forwards: this.list(follower.hostId) };
  }

  private changed(hostId: string): void {
    for (const follower of this.followers.values()) {
      if (follower.hostId === hostId) follower.onMessage(this.message(follower));
    }
  }

  private of(hostId: string): Forward[] {
    let forwards = this.byHost.get(hostId);
    if (!forwards) {
      forwards = [];
      this.byHost.set(hostId, forwards);
    }
    return forwards;
  }
}
