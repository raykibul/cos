// Words for hosts and remote access: what each refusal means, and how the
// tailnet listener and devices read. The daemon's side is "Remote access" in
// docs/daemon.md.

import type { Device, TailnetPeer, TailnetStatus } from "../api/models";
import type { Host } from "../bridge/types";
import type { StateLabel } from "./labels";

// MARK: Devices on this Mac's tailnet

/** A phone or tablet, by Tailscale's OS name; anything else is a computer. */
export function isPhoneOs(os: string): boolean {
  return /ios|ipados|android/i.test(os);
}

/** The first IPv4 address, else the first address, else "". */
export function firstIpv4(ips: string[]): string {
  return ips.find((ip) => !ip.includes(":")) ?? ips[0] ?? "";
}

/** `macOS · 100.101.102.110`. */
export function peerDetail(peer: TailnetPeer): string {
  return [peer.os, firstIpv4(peer.ips)].filter(Boolean).join(" · ");
}

/** The host part of a saved address: `studio:7700` → `studio`, `[fd7a::5]:7700` → `fd7a::5`. */
function addressHost(address: string): string {
  const bracketed = address.match(/^\[(.*)\](?::\d+)?$/);
  if (bracketed) return bracketed[1]!;
  return (address.match(/:/g) ?? []).length > 1 ? address : address.replace(/:\d+$/, "");
}

/**
 * The saved host that is this peer, if any: same address, one of its IPs,
 * or one of its names (the saved host's name or the name in its address).
 */
export function savedHostFor(peer: TailnetPeer, hosts: Host[]): Host | null {
  const ips = peer.ips.map((ip) => ip.toLowerCase());
  const names = [peer.name, peer.hostname, peer.dnsName, peer.dnsName.split(".")[0] ?? ""]
    .map((name) => name.trim().toLowerCase())
    .filter(Boolean);
  for (const host of hosts) {
    if (!host.address) continue;
    if (peer.address && host.address === peer.address) return host;
    const where = addressHost(host.address).toLowerCase();
    if (ips.includes(where) || names.includes(where)) return host;
    if (names.includes(host.name.trim().toLowerCase())) return host;
  }
  return null;
}

/** "5m ago", "3h ago", "2d ago". */
export function shortAgo(ms: number, now: number): string {
  const minutes = Math.max(0, Math.round((now - ms) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

export type PeerChip = "added" | "running" | "online" | "offline";

/**
 * Which chip a peer gets: Added wins, then whether cos answers, then online.
 * A device with no tailnet address can't be reached, so it reads as offline.
 */
export function peerChip(peer: TailnetPeer, added: boolean): PeerChip {
  if (added) return "added";
  if (!peer.address) return "offline";
  if (peer.cos === "running") return "running";
  return peer.online ? "online" : "offline";
}

/** The chip's words: "cos running", "Online", "Offline · seen 3h ago", "Added". */
export function peerChipText(peer: TailnetPeer, chip: PeerChip, now: number): string {
  switch (chip) {
    case "added":
      return "Added";
    case "running":
      return "cos running";
    case "online":
      return "Online";
    default:
      return peer.lastSeenMs ? `Offline · seen ${shortAgo(peer.lastSeenMs, now)}` : "Offline";
  }
}

/** The avatar letter of a host. */
export function hostInitial(name: string): string {
  return (name.trim().charAt(0) || "?").toUpperCase();
}

/**
 * A name for a host typed without one: `my-mac.tail1234.ts.net:7700` →
 * `my-mac`. An IP address stays as it is.
 */
export function nameFromAddress(address: string): string {
  const typed = address.trim();
  const bracketed = typed.match(/^\[(.*)\](?::\d+)?$/);
  if (bracketed) return bracketed[1]!;
  // A bare IPv6 address has no port to strip.
  if ((typed.match(/:/g) ?? []).length > 1) return typed;
  const host = typed.replace(/:\d+$/, "");
  if (/^[\d.]+$/.test(host)) return host;
  return host.split(".")[0] || host;
}

/** The commands a daemon message quotes in backticks, in order. */
export function quotedCommands(message: string): string[] {
  return [...message.matchAll(/`([^`]+)`/g)].map((m) => m[1]!.trim()).filter(Boolean);
}

export interface RefusalCopy {
  title: string;
  body: string;
  /** Whether waiting is enough: the bridge keeps asking, and it may pass. */
  waits: boolean;
}

/** What a remote daemon's refusal means, for its full-page state. */
export function refusalCopy(code: string, host: string): RefusalCopy {
  switch (code) {
    case "device_pending":
      return {
        title: `Waiting for approval on ${host}`,
        body: `${host} lets in a new device only once someone there approves it. Run this on ${host}, or approve it in its cos app under Waiting on you.`,
        waits: true,
      };
    case "device_denied":
      return {
        title: `${host} turned this Mac away`,
        body: `This Mac was denied on ${host}. If that was a mistake, approve it there:`,
        waits: true,
      };
    case "wrong_user":
      return {
        title: "Signed in to Tailscale as someone else",
        body: `${host} only lets in devices logged in to Tailscale as the same person who owns it. Log this Mac in to Tailscale as that user, or pick another Mac.`,
        waits: false,
      };
    case "tagged_device":
      return {
        title: "This Mac is a tagged Tailscale device",
        body: `Tagged devices belong to the tailnet rather than to a person, so ${host} can't tell whose this Mac is and won't let it in. Remove the tag in the Tailscale admin console, or use an untagged device.`,
        waits: false,
      };
    case "unknown_peer":
      return {
        title: `${host} doesn't know this Mac`,
        body: `Tailscale on ${host} couldn't say who this connection comes from. Check that this Mac is connected to Tailscale, on the same tailnet as ${host}.`,
        waits: false,
      };
    case "tailscale_unavailable":
      return {
        title: `Tailscale on ${host} didn't answer`,
        body: `cos asks Tailscale who each device is before letting it in, and Tailscale on ${host} didn't answer this time. This usually passes; cos keeps trying.`,
        waits: true,
      };
    case "host_only":
      return {
        title: `${host} keeps that to itself`,
        body: `Remote access and devices can only be managed on ${host} itself.`,
        waits: false,
      };
    default:
      return {
        title: `${host} refused this Mac`,
        body: `The cos daemon on ${host} answered, but won't let this Mac in.`,
        waits: true,
      };
  }
}

export function tailnetLabel(tailnet: TailnetStatus): StateLabel {
  switch (tailnet.state) {
    case "listening":
      return { title: "On", tone: "green", busy: false, attention: false };
    case "starting":
      return { title: "Starting", tone: "neutral", busy: true, attention: false };
    case "unavailable":
      return { title: "Can't listen", tone: "amber", busy: false, attention: false };
    default:
      return { title: "Off", tone: "neutral", busy: false, attention: false };
  }
}

/** The `sudo …socketfilterfw --unblockapp …` command in a firewall warning, if any. */
export function firewallCommand(reason: string | null): string | null {
  if (!reason || !reason.includes("socketfilterfw")) return null;
  return quotedCommands(reason).find((c) => c.includes("socketfilterfw")) ?? null;
}

/** The reason without its quoted firewall command, which is shown on its own. */
export function reasonText(reason: string): string {
  const command = firewallCommand(reason);
  const text = reason.charAt(0).toUpperCase() + reason.slice(1);
  if (!command) return text;
  return text
    .replace(`\`${command}\``, "the command below")
    .replace(/, or run the command below$/, ", or run the command below.");
}

export function deviceLabel(device: Device): StateLabel {
  switch (device.state) {
    case "approved":
      return { title: "Approved", tone: "green", busy: false, attention: false };
    case "denied":
      return { title: "Denied", tone: "red", busy: false, attention: false };
    default:
      return { title: "Waiting for you", tone: "accent", busy: false, attention: true };
  }
}
