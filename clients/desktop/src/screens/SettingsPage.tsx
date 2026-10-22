// Settings: the machines this app talks to, the environment variables every
// project's agents get, and remote access to this one (the tailnet listener
// and the devices that tried it). Remote access is managed only on the Mac
// itself: its routes answer on the socket alone.

import { Globe, Plus, ShieldCheck, TriangleAlert } from "lucide-react";
import { useState } from "react";
import type { TailnetStatus } from "../api/models";
import { useApp, useCos } from "../app/context";
import { useSwitchHost } from "../app/hosts";
import { THIS_MAC } from "../bridge/types";
import { firewallCommand, reasonText, tailnetLabel } from "../domain/hosts";
import { isThisMac } from "../store/store";
import { CommandLine } from "../ui/CommandLine";
import { cx } from "../ui/cx";
import { Button, Card, EmptyState, SectionTitle, StateBadge, Spinner, inputClass } from "../ui/kit";
import { PageBody, PageHeader, PageTitle } from "../ui/Page";
import { DeviceRow } from "./Devices";
import { EnvironmentList } from "./Environment";
import { envNote } from "../domain/env";
import { HostAvatar } from "./Hosts";
import { JudgeSettings } from "./JudgeSettings";
import { TailnetDevices } from "./TailnetPeers";

export function SettingsPage() {
  return (
    <>
      <PageHeader>
        <span className="text-ink-2">Settings</span>
      </PageHeader>
      <PageBody>
        <PageTitle subtitle="The machines this app talks to, what agents get in their environment, and who may reach this Mac.">
          Settings
        </PageTitle>
        <MacsSection />
        <EnvironmentSection />
        <JudgeSettings />
        <RemoteAccessSection />
      </PageBody>
    </>
  );
}

function MacsSection() {
  const { ui } = useApp();
  const hosts = useCos((s) => s.hosts);
  const current = useCos((s) => s.host?.id ?? THIS_MAC);
  const switchHost = useSwitchHost();
  const list = hosts.length > 0 ? hosts : [{ id: THIS_MAC, name: "This Mac", address: null }];
  return (
    <section className="mb-10">
      <SectionTitle
        actions={
          <Button size="sm" icon={Plus} onClick={() => ui.open({ kind: "addHost" })}>
            Add a machine…
          </Button>
        }
      >
        Machines
      </SectionTitle>
      <Card>
        <ul className="divide-y divide-line" aria-label="Machines">
          {list.map((host) => (
            <li key={host.id} className="flex items-center gap-3 px-4 py-3" aria-label={host.name}>
              <HostAvatar name={host.name} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13.5px] font-medium text-ink">{host.name}</p>
                <p className="truncate text-[12px] text-ink-3">
                  {host.address ? (
                    <span className="font-mono">{host.address}</span>
                  ) : (
                    "Its own daemon, through the socket"
                  )}
                </p>
              </div>
              {host.id === current ? (
                <span className="rounded-full bg-subtle px-2 py-0.5 text-[12px] font-medium text-ink-2">
                  In use
                </span>
              ) : (
                <Button size="sm" variant="ghost" onClick={() => switchHost(host.id)}>
                  Switch
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                aria-label={`Rename ${host.name}`}
                onClick={() => ui.open({ kind: "renameHost", hostId: host.id })}
              >
                Rename
              </Button>
              {host.address ? (
                <Button
                  size="sm"
                  variant="danger"
                  aria-label={`Remove ${host.name}`}
                  onClick={() => ui.open({ kind: "removeHost", hostId: host.id })}
                >
                  Remove
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      </Card>
      <p className="mt-2 px-1 text-[12.5px] leading-snug text-ink-3">
        Another machine is reached over Tailscale. Install cos on it and run{" "}
        <code className="font-mono text-[12px] text-ink-2">cos daemon install</code>, then approve
        this Mac there the first time it connects.
      </p>
    </section>
  );
}

/** The variables every project's agents get, on the host in use. */
function EnvironmentSection() {
  const host = useCos((s) => s.host);
  return (
    <section className="mb-10" aria-label="Environment">
      <EnvironmentList
        projectId={null}
        heading={
          <span className="flex items-center gap-2">
            Environment
            <span className="rounded-full bg-subtle px-2 py-0.5 text-[11.5px] font-medium text-ink-3">
              All projects
            </span>
          </span>
        }
      />
      <p className="mt-2 px-1 text-[12.5px] leading-snug text-ink-3">
        {envNote(null)} A project's own values are under Environment in its ⋯ menu
        {isThisMac(host) ? "" : `, and these belong to ${host?.name ?? "that machine"}'s daemon`}.
      </p>
    </section>
  );
}

function RemoteAccessSection() {
  const host = useCos((s) => s.host);
  const connection = useCos((s) => s.connection);
  const supported = useCos((s) => s.remoteAccessSupported);
  const tailnet = useCos((s) => s.tailnet);
  const error = useCos((s) => s.remoteAccessError);
  const switchHost = useSwitchHost();

  let body;
  if (!isThisMac(host)) {
    body = (
      <Card>
        <EmptyState icon={Globe} title={`Managed on ${host?.name ?? "that machine"}`}>
          Who may reach a machine is decided on that machine. Switch to This Mac to manage remote
          access to it.
          <div className="mt-4">
            <Button size="sm" onClick={() => switchHost(THIS_MAC)}>
              Switch to This Mac
            </Button>
          </div>
        </EmptyState>
      </Card>
    );
  } else if (connection !== "running") {
    body = (
      <Card className="px-5 py-4 text-[13px] text-ink-2">
        Remote access is managed by the cos daemon, which isn't answering right now.
      </Card>
    );
  } else if (!supported) {
    body = (
      <Card className="px-5 py-4 text-[13px] text-ink-2">
        This cos daemon is older than remote access. Update cos and restart the daemon.
      </Card>
    );
  } else if (!tailnet) {
    body = (
      <Card className="flex items-center gap-2 px-5 py-4 text-[13px] text-ink-3">
        {error ? <span className="text-tone-red">{error}</span> : <Spinner />}
      </Card>
    );
  } else {
    body = (
      <>
        <ListenerCard tailnet={tailnet} />
        <DevicesCard />
        <TailnetDevices />
      </>
    );
  }

  return (
    <section>
      <SectionTitle>Remote access</SectionTitle>
      {body}
    </section>
  );
}

function Toggle({
  on,
  busy,
  disabled,
  label,
  onChange,
}: {
  on: boolean;
  busy: boolean;
  disabled?: boolean;
  label: string;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-busy={busy}
      disabled={disabled || busy}
      onClick={() => onChange(!on)}
      className={cx(
        "relative inline-flex h-6 w-10 shrink-0 items-center rounded-full transition-colors disabled:opacity-60",
        on ? "bg-tone-green" : "bg-line-strong",
      )}
    >
      <span
        className={cx(
          "inline-block size-5 rounded-full bg-white shadow-sm transition-transform",
          on ? "translate-x-[18px]" : "translate-x-0.5",
          busy && "animate-pulse",
        )}
      />
    </button>
  );
}

function ListenerCard({ tailnet }: { tailnet: TailnetStatus }) {
  const { cos, ui } = useApp();
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState(String(tailnet.port));
  // The daemon's port changed (here or from the CLI): show it.
  const [shownPort, setShownPort] = useState(tailnet.port);
  if (shownPort !== tailnet.port) {
    setShownPort(tailnet.port);
    setPort(String(tailnet.port));
  }
  const portNumber = Number(port);
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535;
  const portChanged = portValid && portNumber !== tailnet.port;
  const apply = (enabled: boolean, newPort?: number) => {
    setBusy(true);
    void ui
      .perform("Couldn't change remote access", () => cos.setTailnet(enabled, newPort))
      .finally(() => setBusy(false));
  };
  const command = firewallCommand(tailnet.reason);
  const label = tailnetLabel(tailnet);

  return (
    <Card className="mb-6">
      <div className="flex items-start gap-4 px-5 py-4">
        <div className="min-w-0 flex-1">
          <p className="text-[14px] font-medium text-ink">Let your other devices reach this Mac</p>
          <p className="mt-0.5 text-[12.5px] leading-snug text-ink-3">
            Over Tailscale only, from devices signed in as you
            {tailnet.owner ? ` (${tailnet.owner})` : ""}, and each one only once you approve it.
          </p>
        </div>
        <Toggle
          on={tailnet.enabled}
          busy={busy}
          label="Remote access"
          onChange={(on) => apply(on, portChanged ? portNumber : undefined)}
        />
      </div>
      <dl className="grid grid-cols-[120px_1fr] items-center gap-x-4 gap-y-3 border-t border-line px-5 py-4 text-[13px]">
        <dt className="text-ink-3">Status</dt>
        <dd>
          <StateBadge label={label} />
        </dd>
        <dt className="text-ink-3">
          <label htmlFor="tailnet-port">Port</label>
        </dt>
        <dd className="flex items-center gap-2">
          <input
            id="tailnet-port"
            inputMode="numeric"
            className={cx(inputClass, "w-24 py-1 font-mono text-[13px]")}
            value={port}
            onChange={(event) => setPort(event.target.value.replace(/[^0-9]/g, ""))}
            onKeyDown={(event) => {
              if (event.key === "Enter" && portChanged) apply(tailnet.enabled, portNumber);
            }}
          />
          {portChanged ? (
            <Button size="sm" busy={busy} onClick={() => apply(tailnet.enabled, portNumber)}>
              {tailnet.enabled ? "Listen on it" : "Save"}
            </Button>
          ) : null}
          {!portValid && port !== "" ? (
            <span className="text-[12px] text-tone-red">1 to 65535</span>
          ) : null}
        </dd>
        {tailnet.addresses.length > 0 ? (
          <>
            <dt className="self-start pt-0.5 text-ink-3">Addresses</dt>
            <dd className="flex flex-col gap-1">
              {tailnet.addresses.map((address) => (
                <code key={address} className="font-mono text-[12.5px] text-ink select-all">
                  {address}
                </code>
              ))}
              {tailnet.host ? (
                <span className="text-[12px] text-ink-3">
                  Or by name:{" "}
                  <code className="font-mono text-ink-2 select-all">
                    {tailnet.host}:{tailnet.port}
                  </code>
                </span>
              ) : null}
            </dd>
          </>
        ) : null}
      </dl>
      {tailnet.reason ? (
        <div className="flex flex-col gap-3 border-t border-line px-5 py-4">
          <p className="flex gap-2 text-[13px] leading-snug text-ink-2">
            <TriangleAlert
              className="mt-0.5 size-4 shrink-0 text-tone-amber"
              strokeWidth={1.8}
              aria-hidden
            />
            <span className="select-text">{reasonText(tailnet.reason)}</span>
          </p>
          {command ? (
            <CommandLine
              command={command}
              note="Allows incoming connections to cos in the macOS firewall. A rebuilt cos at a new path needs allowing again."
            />
          ) : null}
        </div>
      ) : null}
    </Card>
  );
}

function DevicesCard() {
  const devices = useCos((s) => s.devices);
  const loaded = useCos((s) => s.devicesLoaded);
  const tailnet = useCos((s) => s.tailnet);
  const address = tailnet?.host ? `${tailnet.host}:${tailnet.port}` : null;
  return (
    <>
      <SectionTitle>Devices</SectionTitle>
      <Card>
        {devices.length === 0 ? (
          <EmptyState icon={ShieldCheck} title={loaded ? "No device has tried yet" : "Loading…"}>
            {loaded
              ? `A device that connects shows up here, waiting for you to approve it.${address ? ` On another Mac, add this one as ${address}.` : ""}`
              : null}
          </EmptyState>
        ) : (
          <ul className="divide-y divide-line" aria-label="Devices">
            {devices.map((device) => (
              <DeviceRow key={device.id} device={device} />
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}
