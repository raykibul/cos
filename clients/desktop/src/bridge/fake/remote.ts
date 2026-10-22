// Canned remote access for the fake bridge: this Mac's tailnet listener and
// the devices that tried it (docs/daemon.md, "Remote access"), and the other
// Macs the app can switch to.

import type { Host } from "../types";
import type { Raw } from "./fixtures";

export const firewallReason =
  "the macOS firewall blocks incoming connections to /Users/me/.cargo/bin/cos, so other devices can't connect. " +
  "Allow it in System Settings > Network > Firewall > Options, or run " +
  "`sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp /Users/me/.cargo/bin/cos`";

export function tailnetStatus(enabled: boolean, port: number, reason: string | null = null): Raw {
  if (!enabled) return { enabled, port, state: "off", addresses: [] };
  const status: Raw = {
    enabled,
    port,
    state: "listening",
    addresses: [`100.101.102.104:${port}`, `[fd7a:115c:a1e0::4]:${port}`],
    host: "my-mac",
    owner: "me@example.com",
  };
  if (reason) status.reason = reason;
  return status;
}

export function makeDevices(now: number): Raw[] {
  const iso = (agoSeconds: number) => new Date(now - agoSeconds * 1000).toISOString();
  return [
    {
      id: 1,
      node_id: "nPh0n3CNTRL",
      name: "phone",
      hostname: "Me's iPhone",
      os: "iOS",
      login_name: "me@example.com",
      state: "approved",
      last_ip: "100.101.102.103",
      first_seen_at: iso(86_400 * 12),
      last_seen_at: iso(240),
      decided_at: iso(86_400 * 12 - 30),
    },
    {
      id: 2,
      node_id: "nSTud10CNTRL",
      name: "studio",
      hostname: "Studio",
      os: "macOS",
      login_name: "me@example.com",
      state: "pending",
      last_ip: "100.101.102.110",
      first_seen_at: iso(95),
      last_seen_at: iso(35),
    },
    {
      id: 3,
      node_id: "nOldL4ptopCNTRL",
      name: "old-laptop",
      hostname: "Old MacBook",
      os: "macOS",
      login_name: "me@example.com",
      state: "denied",
      last_ip: "100.101.102.120",
      first_seen_at: iso(86_400 * 40),
      last_seen_at: iso(86_400 * 39),
      decided_at: iso(86_400 * 39),
    },
  ];
}

/** Why the daemon can't list the tailnet, when Tailscale is off (`?tailscale=off`). */
export const tailscaleReason = "Tailscale is not running (the Tailscale app is not open)";

/**
 * The devices on this Mac's tailnet, as `GET /v1/tailnet/peers` lists them:
 * cos running first, then online, then by name. Studio and the travel
 * laptop are saved hosts too (by name and by address).
 */
export function makePeers(now: number, port = 7700): Raw[] {
  const peer = (
    name: string,
    os: string,
    n: number,
    fields: { online: boolean; cos: string; agoMs?: number; mine?: boolean; tagged?: boolean },
  ): Raw => ({
    id: `n${name.replace(/[^a-z]/g, "")}${n}CNTRL`,
    name,
    hostname: name,
    dns_name: `${name}.tail0000.ts.net`,
    os,
    ips: [`100.101.102.${n}`, `fd7a:115c:a1e0::${n}`],
    online: fields.online,
    last_seen_ms: fields.online ? null : now - (fields.agoMs ?? 0),
    mine: fields.mine ?? true,
    tagged: fields.tagged ?? false,
    cos: fields.cos,
    address: `100.101.102.${n}:${port}`,
  });
  return [
    peer("mac-mini", "macOS", 112, { online: true, cos: "running" }),
    peer("studio", "macOS", 110, { online: true, cos: "running" }),
    peer("build-box", "linux", 115, { online: true, cos: "not_running" }),
    peer("ci-runner", "linux", 130, {
      online: true,
      cos: "not_running",
      mine: false,
      tagged: true,
    }),
    peer("phone", "iOS", 103, { online: false, cos: "unknown", agoMs: 2 * 3_600_000 }),
    peer("travel-laptop", "macOS", 105, { online: false, cos: "unknown", agoMs: 86_400_000 }),
  ];
}

/** How a saved host answers in the fake. */
export type Access =
  | { kind: "open" }
  /** Answers `device_pending` until approved (by hand, or after `approveAt`). */
  | { kind: "pending"; approveAt: number | null }
  | { kind: "refused"; status: number; code: string; message: string }
  /** Nothing answers at its address. */
  | { kind: "unreachable" };

export interface FakeHost extends Host {
  access: Access;
}

export function makeHosts(): FakeHost[] {
  return [
    { id: "local", name: "This Mac", address: null, access: { kind: "open" } },
    {
      id: "h1",
      name: "Studio",
      address: "studio:7700",
      access: { kind: "pending", approveAt: null },
    },
    {
      id: "h2",
      name: "Travel laptop",
      address: "100.101.102.105:7700",
      access: { kind: "unreachable" },
    },
  ];
}

/** The refusal a host answers with now, or null if it lets this Mac in. */
export function refusalOf(
  host: FakeHost,
): { status: number; code: string; message: string } | null {
  switch (host.access.kind) {
    case "pending":
      return {
        status: 403,
        code: "device_pending",
        message: `this device (my-mac) is waiting for approval. On ${hostName(host)}, run \`cos devices approve 4\``,
      };
    case "refused":
      return host.access;
    default:
      return null;
  }
}

/** The host's MagicDNS name, as the daemon names itself in messages. */
function hostName(host: FakeHost): string {
  return (host.address ?? "this-mac").replace(/:\d+$/, "");
}
