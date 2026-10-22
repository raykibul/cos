// Devices that tried this Mac's tailnet listener: a row on the remote access
// page, and a card in "Waiting on you" for one asking to be let in.

import { Laptop, MonitorSmartphone, Smartphone } from "lucide-react";
import { useState } from "react";
import type { Device } from "../api/models";
import { useApp, useCanChange, useNow } from "../app/context";
import { deviceLabel } from "../domain/hosts";
import { relativeTime } from "../ui/format";
import { Button, StateBadge } from "../ui/kit";

function DeviceIcon({ device, className }: { device: Device; className?: string }) {
  const os = device.os.toLowerCase();
  const Icon = /ios|ipados|android/.test(os)
    ? Smartphone
    : /mac|windows|linux/.test(os)
      ? Laptop
      : MonitorSmartphone;
  return <Icon className={className} strokeWidth={1.7} aria-hidden />;
}

function title(device: Device): string {
  return device.hostname || device.name;
}

type Action = "approve" | "deny" | "remove";

/** Runs one device action at a time, with its own busy state and error toast. */
function useDeviceAction(device: Device) {
  const { cos, ui } = useApp();
  const [busy, setBusy] = useState<Action | null>(null);
  const run = (action: Action) => {
    setBusy(action);
    const perform =
      action === "approve"
        ? () => cos.approveDevice(device.id)
        : action === "deny"
          ? () => cos.denyDevice(device.id)
          : () => cos.removeDevice(device.id);
    const failure =
      action === "approve"
        ? "Couldn't approve the device"
        : action === "deny"
          ? "Couldn't deny the device"
          : "Couldn't remove the device";
    void ui.perform(failure, perform).finally(() => setBusy(null));
  };
  return { busy, run };
}

/** One device on the remote access page, with what can be done about it. */
export function DeviceRow({ device }: { device: Device }) {
  const canChange = useCanChange();
  const now = useNow();
  const { busy, run } = useDeviceAction(device);
  const name = title(device);
  const details = [
    device.name !== name ? device.name : null,
    device.os || null,
    device.loginName || null,
    device.lastIp,
    device.lastSeenAt ? `seen ${relativeTime(device.lastSeenAt, now)}` : null,
  ].filter(Boolean);
  const button = (
    action: Action,
    label: string,
    variant: "primary" | "secondary" | "ghost" | "danger",
  ) => (
    <Button
      size="sm"
      variant={variant}
      busy={busy === action}
      disabled={!canChange || (busy !== null && busy !== action)}
      aria-label={`${label} ${name}`}
      onClick={() => run(action)}
    >
      {label}
    </Button>
  );
  return (
    <li className="flex items-center gap-3 px-4 py-3" aria-label={name}>
      <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-subtle text-ink-3">
        <DeviceIcon device={device} className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-[13.5px] font-medium text-ink">{name}</span>
          <StateBadge label={deviceLabel(device)} />
        </div>
        <p className="mt-0.5 truncate text-[12px] text-ink-3" title={details.join(" · ")}>
          {details.join(" · ")}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {device.state === "pending" ? (
          <>
            {button("deny", "Deny", "ghost")}
            {button("approve", "Approve", "primary")}
          </>
        ) : device.state === "approved" ? (
          <>
            {button("deny", "Deny", "ghost")}
            {button("remove", "Remove", "danger")}
          </>
        ) : (
          <>
            {button("approve", "Approve", "secondary")}
            {button("remove", "Remove", "danger")}
          </>
        )}
      </div>
    </li>
  );
}

/** A device asking to reach this Mac, in "Waiting on you". */
export function DeviceRequestCard({ device }: { device: Device }) {
  const canChange = useCanChange();
  const now = useNow();
  const { busy, run } = useDeviceAction(device);
  const name = title(device);
  return (
    <article className="flex items-center gap-4 rounded-xl border border-line bg-surface p-4 shadow-card">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-accent/12 text-accent-ink">
        <DeviceIcon device={device} className="size-[18px]" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[12.5px] text-ink-3">
          Wants to reach this Mac over Tailscale · {relativeTime(device.firstSeenAt, now)}
        </p>
        <p className="mt-0.5 truncate text-[14.5px] font-medium text-ink">{name}</p>
        <p className="truncate text-[12px] text-ink-3">
          {[device.os, device.loginName, device.lastIp].filter(Boolean).join(" · ")}
        </p>
      </div>
      <Button
        variant="ghost"
        busy={busy === "deny"}
        disabled={!canChange || busy === "approve"}
        aria-label={`Deny ${name}`}
        onClick={() => run("deny")}
      >
        Deny
      </Button>
      <Button
        variant="accent"
        busy={busy === "approve"}
        disabled={!canChange || busy === "deny"}
        aria-label={`Approve ${name}`}
        onClick={() => run("approve")}
      >
        Approve
      </Button>
    </article>
  );
}
