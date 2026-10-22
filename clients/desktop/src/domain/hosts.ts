// Words for hosts and remote access: what each refusal means, and how the
// tailnet listener and devices read. The daemon's side is "Remote access" in
// docs/daemon.md.

import type { Device, TailnetStatus } from "../api/models";
import type { StateLabel } from "./labels";

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
