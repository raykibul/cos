// The host picker at the top of the sidebar (This Mac, the saved machines,
// "Rename…" for the one in use and "Add a machine…"), and the footer line
// saying which host the app talks to. Self-contained, so the sidebar only
// mounts them.

import { ChevronDown, Globe, Laptop, Pencil, Plus, Settings, Smartphone } from "lucide-react";
import { useState } from "react";
import type { TailnetPeer } from "../api/models";
import { useApp, useCos, useNow } from "../app/context";
import { useSwitchHost } from "../app/hosts";
import { useConnectPeer } from "../app/peers";
import { type Host, THIS_MAC } from "../bridge/types";
import { firstIpv4, hostInitial, isPhoneOs, savedHostFor } from "../domain/hosts";
import { type Connection, isThisMac } from "../store/store";
import { cx } from "../ui/cx";
import { duration } from "../ui/format";
import { type MenuItem, Menu } from "../ui/Menu";

const thisMac: Host = { id: THIS_MAC, name: "This Mac", address: null };

function connectionDot(connection: Connection): string {
  switch (connection) {
    case "running":
      return "bg-tone-green";
    case "not_running":
      return "bg-tone-red";
    case "refused":
      return "bg-accent";
    default:
      return "bg-ink-3 animate-breathe";
  }
}

export function HostAvatar({
  name,
  connection,
  size = "md",
}: {
  name: string;
  connection?: Connection;
  size?: "sm" | "md";
}) {
  return (
    <span
      aria-hidden
      className={cx(
        "relative flex shrink-0 items-center justify-center rounded-full bg-subtle-2 font-semibold text-ink",
        size === "md" ? "size-6 text-[12px]" : "size-5 text-[10.5px]",
      )}
    >
      {hostInitial(name)}
      {connection ? (
        <span
          className={cx(
            "absolute -right-0.5 -bottom-0.5 size-2 rounded-full ring-2 ring-sidebar",
            connectionDot(connection),
          )}
        />
      ) : null}
    </span>
  );
}

/** The host in use, as a menu of every host plus "Rename…" and "Add a machine…". */
export function HostPicker() {
  const { cos, ui } = useApp();
  const host = useCos((s) => s.host) ?? thisMac;
  const saved = useCos((s) => s.hosts);
  const connection = useCos((s) => s.connection);
  const switchHost = useSwitchHost();
  const connect = useConnectPeer();
  const hosts = saved.length > 0 ? saved : [thisMac];
  // Asked of This Mac's daemon each time the menu opens; the last answer
  // stays until the next one, and a failure just hides the section.
  const [peers, setPeers] = useState<TailnetPeer[]>([]);
  const fetchPeers = () => {
    cos.tailnetPeers().then(
      (answer) => setPeers(answer.peers),
      () => setPeers([]),
    );
  };
  const addable = peers.filter(
    (peer) =>
      peer.mine &&
      peer.cos === "running" &&
      peer.address !== "" &&
      savedHostFor(peer, hosts) === null,
  );

  const items: MenuItem[] = [
    ...hosts.map((h): MenuItem => ({
      label: h.name,
      detail: h.address ?? "On this Mac",
      icon: h.address ? Globe : Laptop,
      checked: h.id === host.id,
      onSelect: () => switchHost(h.id),
    })),
    ...(addable.length > 0
      ? [
          "separator" as const,
          { heading: "On your tailnet" },
          ...addable.map((peer): MenuItem => ({
            label: peer.name,
            detail: `cos running · ${firstIpv4(peer.ips) || peer.address}`,
            icon: isPhoneOs(peer.os) ? Smartphone : Laptop,
            onSelect: () =>
              void ui.perform(`Couldn't connect to ${peer.name}`, () => connect(peer)),
          })),
        ]
      : []),
    "separator",
    {
      label: `Rename ${host.name}…`,
      icon: Pencil,
      onSelect: () => ui.open({ kind: "renameHost", hostId: host.id }),
    },
    { label: "Add a machine…", icon: Plus, onSelect: () => ui.open({ kind: "addHost" }) },
    {
      label: "Settings",
      icon: Settings,
      detail: "Machines and remote access",
      onSelect: () => ui.navigate({ kind: "settings" }),
    },
  ];

  return (
    <div className="shrink-0 px-2.5 pb-2">
      <Menu
        items={items}
        width={240}
        onOpen={fetchPeers}
        trigger={(props) => (
          <button
            type="button"
            aria-label={`Machine: ${host.name}`}
            title={host.address ?? "This Mac, through its socket"}
            {...props}
            className="flex h-9 w-full items-center gap-2.5 rounded-lg px-1.5 text-left transition-colors hover:bg-subtle aria-expanded:bg-subtle"
          >
            <HostAvatar name={host.name} connection={connection} />
            <span className="min-w-0 truncate text-[14px] font-medium text-ink">{host.name}</span>
            <ChevronDown className="size-3.5 shrink-0 text-ink-3" strokeWidth={2} aria-hidden />
          </button>
        )}
      />
    </div>
  );
}

function stateLine(
  connection: Connection,
  remote: boolean,
  code: string | null,
  version: string | null,
  up: string | null,
): string {
  switch (connection) {
    case "running":
      return version ? `cos v${version}${up ? ` · up ${up}` : ""}` : "Connected";
    case "not_running":
      return remote ? "Can't reach it" : "cos isn't running";
    case "refused":
      return code === "device_pending" ? "Waiting for approval" : "Won't let this Mac in";
    case "reconnecting":
      return "Reconnecting…";
    default:
      return "Connecting…";
  }
}

/** The footer line: which host and how it's doing. It opens Settings. */
export function HostStatus() {
  const { ui } = useApp();
  const connection = useCos((s) => s.connection);
  const status = useCos((s) => s.status);
  const code = useCos((s) => s.refusal?.code ?? null);
  const host = useCos((s) => s.host) ?? thisMac;
  const now = useNow(60_000);
  const remote = !isThisMac(host);
  const line = stateLine(
    connection,
    remote,
    code,
    status?.version ?? null,
    status ? duration(status.startedAt, now) : null,
  );
  const where = remote ? host.address : status?.home;
  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        onClick={() => ui.navigate({ kind: "settings" })}
        className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-subtle"
        title={
          [where, status ? `pid ${status.pid}` : null].filter(Boolean).join(" · ") || undefined
        }
      >
        <span
          className={cx("ml-1 size-2 shrink-0 rounded-full", connectionDot(connection))}
          aria-hidden
        />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[12.5px] font-medium text-ink-2">{host.name}</span>
          <span className="block truncate text-[11.5px] text-ink-3">{line}</span>
        </span>
      </button>
    </div>
  );
}
