// The devices on this Mac's tailnet: a row for each, with whether cos runs
// there. The "Add a Mac" sheet lists them above its fields (Connect for a
// Mac running cos, a click to fill the form for the others), and Settings
// shows them read-only. They always come from This Mac's daemon.

import { Check, ChevronDown, Laptop, RotateCw, Smartphone, TriangleAlert } from "lucide-react";
import { type ReactNode, useState } from "react";
import { messageOf } from "../api/errors";
import type { TailnetPeer } from "../api/models";
import { useCos, useNow } from "../app/context";
import { useSwitchHost } from "../app/hosts";
import { type PeersState, useConnectPeer, useTailnetPeers } from "../app/peers";
import type { Host } from "../bridge/types";
import {
  type PeerChip,
  isPhoneOs,
  peerChip,
  peerChipText,
  peerDetail,
  reasonText,
  savedHostFor,
} from "../domain/hosts";
import { cx } from "../ui/cx";
import { ErrorText } from "../ui/Dialog";
import { Button, Card, IconButton, SectionTitle, Spinner } from "../ui/kit";

export function PeerIcon({ os, className }: { os: string; className?: string }) {
  const Icon = isPhoneOs(os) ? Smartphone : Laptop;
  return <Icon className={className} strokeWidth={1.7} aria-hidden />;
}

const chipClasses: Record<PeerChip, { chip: string; dot: string | null }> = {
  running: { chip: "bg-tone-green/12 text-tone-green", dot: "bg-tone-green" },
  online: { chip: "bg-subtle text-ink-2", dot: "bg-ink-3" },
  offline: { chip: "text-ink-3 ring-1 ring-inset ring-line", dot: null },
  added: { chip: "bg-subtle text-ink-2", dot: null },
};

export function PeerStatus({ peer, chip }: { peer: TailnetPeer; chip: PeerChip }) {
  const now = useNow(60_000);
  const style = chipClasses[chip];
  return (
    <span
      className={cx(
        "inline-flex h-[22px] shrink-0 items-center gap-1.5 rounded-full px-2 text-[12px] font-medium whitespace-nowrap",
        style.chip,
      )}
    >
      {chip === "added" ? (
        <Check className="size-3" strokeWidth={2.4} aria-hidden />
      ) : style.dot ? (
        <span className={cx("size-1.5 rounded-full", style.dot)} aria-hidden />
      ) : null}
      {peerChipText(peer, chip, now)}
    </span>
  );
}

/**
 * One device. With `onPick` the row is a button (it fills the form);
 * `action` sits at its end, outside that button.
 */
export function PeerRow({
  peer,
  chip,
  selected = false,
  muted = false,
  onPick,
  action,
}: {
  peer: TailnetPeer;
  chip: PeerChip;
  selected?: boolean;
  muted?: boolean;
  onPick?: () => void;
  action?: ReactNode;
}) {
  const body = (
    <>
      <span
        className={cx(
          "flex size-8 shrink-0 items-center justify-center rounded-lg",
          chip === "running" ? "bg-tone-green/10 text-tone-green" : "bg-subtle text-ink-3",
        )}
      >
        <PeerIcon os={peer.os} className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13.5px] font-medium text-ink">{peer.name}</span>
        <span className="block truncate text-[12px] text-ink-3">
          {peerDetail(peer)}
          {peer.tagged ? " · tagged" : ""}
        </span>
      </span>
      <PeerStatus peer={peer} chip={chip} />
    </>
  );
  return (
    <li
      aria-label={peer.name}
      aria-current={selected || undefined}
      className={cx(
        "flex items-center gap-2 pr-3 transition-colors",
        selected && "bg-accent/[0.07]",
        muted && "opacity-60",
      )}
    >
      {onPick ? (
        <button
          type="button"
          onClick={onPick}
          title={`Use ${peer.address}`}
          className="flex min-w-0 flex-1 items-center gap-3 py-2 pl-3 text-left outline-none hover:opacity-90 focus-visible:bg-subtle"
        >
          {body}
        </button>
      ) : (
        <div className="flex min-w-0 flex-1 items-center gap-3 py-2 pl-3">{body}</div>
      )}
      {action}
    </li>
  );
}

function Note({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <p className="flex items-start gap-2 px-3 py-3 text-[12.5px] leading-snug text-ink-3">
      {icon}
      <span className="min-w-0 select-text">{children}</span>
    </p>
  );
}

function Code({ children }: { children: string }) {
  return (
    <code className="rounded bg-subtle-2 px-1 py-px font-mono text-[12px] whitespace-nowrap text-ink">
      {children}
    </code>
  );
}

/** What to show instead of rows: loading, the daemon down, an error, or Tailscale's reason. */
function stateNote(state: PeersState): ReactNode | null {
  if (state.status === "loading") {
    return <Note icon={<Spinner className="size-3.5" />}>Looking for devices…</Note>;
  }
  if (state.status === "error") {
    return (
      <Note>
        {state.error.kind === "not_running"
          ? "This Mac's cos daemon isn't running, so your tailnet can't be listed."
          : state.error.message}
      </Note>
    );
  }
  if (state.peers.reason) {
    return (
      <Note
        icon={
          <TriangleAlert
            className="mt-px size-3.5 shrink-0 text-tone-amber"
            strokeWidth={1.8}
            aria-hidden
          />
        }
      >
        {reasonText(state.peers.reason)}
      </Note>
    );
  }
  return null;
}

/** The section's title line, with a refresh button. */
function RefreshButton({ state, refresh }: { state: PeersState; refresh: () => void }) {
  const spinning = state.status === "loading" || (state.status === "ready" && state.refreshing);
  return (
    <IconButton
      icon={RotateCw}
      size="sm"
      label="Refresh the tailnet's devices"
      onClick={refresh}
      disabled={spinning}
      className={cx(spinning && "[&>svg]:animate-spin")}
    />
  );
}

function Heading({
  state,
  refresh,
  children,
}: {
  state: PeersState;
  refresh: () => void;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <h3 className="text-[12.5px] font-medium text-ink-2">{children}</h3>
      <RefreshButton state={state} refresh={refresh} />
    </div>
  );
}

/** Splits the peers into yours (connectable) and the others, tagged or shared. */
function split(state: PeersState): { mine: TailnetPeer[]; others: TailnetPeer[] } {
  if (state.status !== "ready") return { mine: [], others: [] };
  const peers = state.peers.peers;
  return { mine: peers.filter((p) => p.mine), others: peers.filter((p) => !p.mine) };
}

function OthersToggle({
  count,
  open,
  onToggle,
}: {
  count: number;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex w-full items-center gap-1.5 px-3 py-2 text-left text-[12px] text-ink-3 hover:text-ink-2"
    >
      <ChevronDown
        className={cx("size-3.5 transition-transform", open && "rotate-180")}
        strokeWidth={2}
        aria-hidden
      />
      {open
        ? `Hide ${count === 1 ? "the other device" : "other devices"}`
        : `Show ${count} other ${count === 1 ? "device" : "devices"}`}
      <span className="text-ink-3/80">· tagged or not yours, so they can't connect</span>
    </button>
  );
}

/**
 * The "On your tailnet" list in the Add a Mac sheet. A Mac running cos gets
 * Connect (add it, then switch to it); clicking any other row of yours fills
 * the form with its name and address.
 */
export function TailnetPicker({
  address,
  onFill,
}: {
  /** The address typed in the form, to mark the row it came from. */
  address: string;
  onFill: (peer: TailnetPeer) => void;
}) {
  const { state, refresh } = useTailnetPeers();
  const hosts = useCos((s) => s.hosts);
  const current = useCos((s) => s.host?.id ?? null);
  const switchHost = useSwitchHost();
  const connect = useConnectPeer();
  const [connecting, setConnecting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<TailnetPeer | null>(null);
  const [showOthers, setShowOthers] = useState(false);
  const { mine, others } = split(state);
  const note = stateNote(state);
  const typed = address.trim();

  const onConnect = (peer: TailnetPeer) => {
    if (connecting) return;
    setConnecting(peer.id);
    setError(null);
    connect(peer).then(
      () => setConnecting(null),
      (failure: unknown) => {
        setConnecting(null);
        setError(messageOf(failure));
      },
    );
  };

  const row = (peer: TailnetPeer, yours: boolean) => {
    // A device with no tailnet address can't be reached: a plain row.
    const connectable = yours && peer.address !== "";
    const saved: Host | null = savedHostFor(peer, hosts);
    const chip = peerChip(peer, saved !== null);
    let action: ReactNode = null;
    let onPick: (() => void) | undefined;
    if (connectable && saved) {
      if (saved.id !== current) {
        action = (
          <Button
            size="sm"
            variant="ghost"
            aria-label={`Switch to ${saved.name}`}
            onClick={() => switchHost(saved.id)}
          >
            Switch
          </Button>
        );
      }
    } else if (connectable && peer.cos === "running") {
      action = (
        <Button
          size="sm"
          variant="primary"
          aria-label={`Connect to ${peer.name}`}
          busy={connecting === peer.id}
          disabled={connecting !== null && connecting !== peer.id}
          onClick={() => onConnect(peer)}
        >
          Connect
        </Button>
      );
    } else if (connectable) {
      onPick = () => {
        setPicked(peer);
        onFill(peer);
      };
    }
    return (
      <PeerRow
        key={peer.id}
        peer={peer}
        chip={chip}
        muted={!yours}
        selected={onPick !== undefined && typed !== "" && typed === peer.address}
        onPick={onPick}
        action={action}
      />
    );
  };

  const hint = picked && picked.address === typed && picked.cos !== "running" ? picked : null;

  return (
    <section aria-label="On your tailnet" className="flex flex-col gap-2">
      <Heading state={state} refresh={refresh}>
        On your tailnet
      </Heading>
      <div className="overflow-hidden rounded-xl border border-line bg-surface">
        {note ??
          (mine.length === 0 && others.length === 0 ? (
            <Note>No other devices on your tailnet.</Note>
          ) : (
            <div className="max-h-[236px] overflow-y-auto">
              {mine.length > 0 ? (
                <ul className="divide-y divide-line" aria-label="Your devices">
                  {mine.map((peer) => row(peer, true))}
                </ul>
              ) : (
                <Note>None of the devices on your tailnet are yours.</Note>
              )}
              {others.length > 0 ? (
                <div className="border-t border-line">
                  <OthersToggle
                    count={others.length}
                    open={showOthers}
                    onToggle={() => setShowOthers((v) => !v)}
                  />
                  {showOthers ? (
                    <ul
                      className="divide-y divide-line border-t border-line"
                      aria-label="Other devices"
                    >
                      {others.map((peer) => row(peer, false))}
                    </ul>
                  ) : null}
                </div>
              ) : null}
            </div>
          ))}
      </div>
      {hint ? (
        <p className="px-1 text-[12.5px] leading-snug text-ink-2">
          {hint.name} isn&apos;t running cos where this Mac can reach it. Install cos there and run{" "}
          <Code>cos daemon install</Code> first.
        </p>
      ) : null}
      <ErrorText>{error}</ErrorText>
    </section>
  );
}

/** Settings: the tailnet's devices of yours, read-only. */
export function TailnetDevices() {
  const { state, refresh } = useTailnetPeers();
  const hosts = useCos((s) => s.hosts);
  const { mine } = split(state);
  const note = stateNote(state);
  return (
    <section aria-label="Devices on your tailnet" className="mt-6">
      <SectionTitle actions={<RefreshButton state={state} refresh={refresh} />}>
        Devices on your tailnet
      </SectionTitle>
      <Card className="overflow-hidden">
        {note ??
          (mine.length === 0 ? (
            <Note>No other devices on your tailnet.</Note>
          ) : (
            <ul className="divide-y divide-line">
              {mine.map((peer) => (
                <PeerRow
                  key={peer.id}
                  peer={peer}
                  chip={peerChip(peer, savedHostFor(peer, hosts) !== null)}
                />
              ))}
            </ul>
          ))}
      </Card>
    </section>
  );
}
