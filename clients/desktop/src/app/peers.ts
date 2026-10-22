// The devices on this Mac's tailnet, asked of This Mac's daemon whichever
// host is in use (`tailnet_peers`), and connecting to one in a click.

import { useCallback, useEffect, useRef, useState } from "react";
import { CosError } from "../api/errors";
import type { TailnetPeer, TailnetPeers } from "../api/models";
import { useApp } from "./context";

export type PeersState =
  | { status: "loading" }
  | { status: "ready"; peers: TailnetPeers; refreshing: boolean }
  | { status: "error"; error: CosError };

/**
 * Asks for the tailnet's devices on mount (and whenever `active` turns on),
 * and again on `refresh`. The daemon caches its cos probes for 10 s, so
 * asking often is cheap. An answer that comes back after a newer request
 * is dropped.
 */
export function useTailnetPeers(active = true): {
  state: PeersState;
  refresh: () => void;
} {
  const { cos } = useApp();
  const [state, setState] = useState<PeersState>({ status: "loading" });
  const latest = useRef(0);

  const fetchPeers = useCallback(() => {
    const request = ++latest.current;
    cos.tailnetPeers().then(
      (peers) => {
        if (request === latest.current) setState({ status: "ready", peers, refreshing: false });
      },
      (error: unknown) => {
        if (request === latest.current) setState({ status: "error", error: CosError.from(error) });
      },
    );
  }, [cos]);

  useEffect(() => {
    if (active) fetchPeers();
  }, [active, fetchPeers]);

  const refresh = useCallback(() => {
    setState((s) => (s.status === "ready" ? { ...s, refreshing: true } : { status: "loading" }));
    fetchPeers();
  }, [fetchPeers]);

  return { state, refresh };
}

/**
 * Saves a peer running cos as a host and switches to it, as "Add and
 * connect" does: the window goes to the waiting page while it connects.
 */
export function useConnectPeer(): (peer: TailnetPeer) => Promise<void> {
  const { cos, ui } = useApp();
  return async (peer: TailnetPeer) => {
    const host = await cos.addHost(peer.name, peer.address);
    ui.close();
    ui.navigate({ kind: "waiting" });
    await cos.switchHost(host.id);
  };
}
