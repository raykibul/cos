// Full-page states for a remote host that won't or can't talk: a refusal
// (403 or 503 with a `code`, see "Refusals" in docs/daemon.md) or nothing
// answering at its address. The bridge keeps retrying in the background, so
// these pages turn back into the app on their own once the host lets us in.

import {
  CircleHelp,
  Clock,
  type LucideIcon,
  ShieldAlert,
  ShieldX,
  Tag,
  Unplug,
  UserX,
  WifiOff,
} from "lucide-react";
import type { ReactNode } from "react";
import { useState } from "react";
import { useApp, useCos } from "../app/context";
import { useSwitchHost } from "../app/hosts";
import { THIS_MAC } from "../bridge/types";
import { quotedCommands, refusalCopy } from "../domain/hosts";
import { CommandLine } from "../ui/CommandLine";
import { Button } from "../ui/kit";
import { PageHeader } from "../ui/Page";

const icons: Record<string, LucideIcon> = {
  device_pending: Clock,
  device_denied: ShieldX,
  wrong_user: UserX,
  tagged_device: Tag,
  unknown_peer: CircleHelp,
  tailscale_unavailable: WifiOff,
};

function Layout({
  icon: Icon,
  title,
  children,
  waiting,
}: {
  icon: LucideIcon;
  title: string;
  children: ReactNode;
  /** The line under the actions while the bridge keeps trying. */
  waiting: string | null;
}) {
  const { cos } = useApp();
  const switchHost = useSwitchHost();
  const [retrying, setRetrying] = useState(false);
  const retry = () => {
    setRetrying(true);
    void cos.reconnect().finally(() => setTimeout(() => setRetrying(false), 600));
  };
  return (
    <>
      <PageHeader />
      <div className="flex flex-1 items-start justify-center overflow-y-auto px-8 pt-[11vh] pb-12">
        <div className="w-full max-w-[540px]">
          <div className="mb-5 flex size-11 items-center justify-center rounded-full bg-subtle text-ink-3">
            <Icon className="size-5" strokeWidth={1.7} aria-hidden />
          </div>
          <h1 className="font-serif text-[28px] leading-tight font-medium text-ink">{title}</h1>
          <div className="mt-2 flex flex-col gap-4 text-[14px] leading-relaxed text-ink-2">
            {children}
          </div>
          <div className="mt-7 flex items-center gap-2">
            <Button onClick={retry} busy={retrying}>
              Try now
            </Button>
            <Button variant="ghost" onClick={() => switchHost(THIS_MAC)}>
              Switch to This Mac
            </Button>
          </div>
          {waiting ? (
            <div className="mt-5 flex items-center gap-2 text-[12px] text-ink-3">
              <span className="size-1.5 animate-breathe rounded-full bg-ink-3" aria-hidden />
              <span>{waiting}</span>
            </div>
          ) : null}
        </div>
      </div>
    </>
  );
}

/** The exact words, small, for the record. */
function Details({ label, message }: { label: string; message: string }) {
  return (
    <p className="text-[12px] leading-snug text-ink-3 select-text">
      <span className="font-medium">{label}</span> {message}
    </p>
  );
}

/** A remote daemon answered with a refusal. */
export function HostRefused() {
  const refusal = useCos((s) => s.refusal);
  const host = useCos((s) => s.host);
  if (!refusal) return null;
  const name = host?.name ?? "that Mac";
  const copy = refusalCopy(refusal.code, name);
  const commands = quotedCommands(refusal.message);
  const pending = refusal.code === "device_pending";
  // A pending or denied device is let in with the command the daemon named.
  if (commands.length === 0 && (pending || refusal.code === "device_denied")) {
    commands.push("cos devices");
  }
  return (
    <Layout
      icon={icons[refusal.code] ?? ShieldAlert}
      title={copy.title}
      waiting={
        copy.waits
          ? pending
            ? `Checking again every few seconds. This page goes away once ${name} approves this Mac.`
            : "Trying again every few seconds…"
          : null
      }
    >
      <p>{copy.body}</p>
      {commands.map((command, index) => (
        <CommandLine
          key={command}
          command={command}
          note={index === 0 ? `Run it in a terminal on ${name}.` : undefined}
        />
      ))}
      <Details label={`cos on ${name} said:`} message={refusal.message} />
    </Layout>
  );
}

/** Nothing answers at a remote host's address. */
export function HostUnreachable() {
  const host = useCos((s) => s.host);
  const reason = useCos((s) => s.connectionReason);
  const name = host?.name ?? "that Mac";
  return (
    <Layout icon={Unplug} title={`Can't reach ${name}`} waiting="Trying again every few seconds…">
      <p>
        Nothing answers at{" "}
        <code className="rounded bg-subtle px-1 py-0.5 font-mono text-[13px] text-ink">
          {host?.address ?? "its address"}
        </code>
        . Check that:
      </p>
      <ul className="flex list-disc flex-col gap-1.5 pl-5 marker:text-ink-3">
        <li>{name} is awake, and both Macs are connected to Tailscale.</li>
        <li>
          cos is running there with remote access on. It listens on port 7700 unless you chose
          another with <code className="font-mono text-[13px]">--port</code>.
        </li>
        <li>
          The address is right: <code className="font-mono text-[13px]">cos remote</code> on {name}{" "}
          prints where it listens.
        </li>
      </ul>
      <CommandLine command="cos remote on" note={`Run it in a terminal on ${name}.`} />
      {reason ? <Details label="Details:" message={reason} /> : null}
    </Layout>
  );
}
