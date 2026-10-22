// Adding and removing a Mac. These work whether or not a daemon answers,
// since the host list lives in the app.

import { useState } from "react";
import { messageOf } from "../api/errors";
import { useApp, useCos } from "../app/context";
import { nameFromAddress } from "../domain/hosts";
import { TailnetPicker } from "../screens/TailnetPeers";
import { cx } from "../ui/cx";
import { Dialog, ErrorText } from "../ui/Dialog";
import { Button, Field, inputClass } from "../ui/kit";

function useSheetAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    action().then(
      () => setBusy(false),
      (failure: unknown) => {
        setBusy(false);
        setError(messageOf(failure));
      },
    );
  };
  return { busy, error, run };
}

function Code({ children }: { children: string }) {
  return (
    <code className="rounded bg-subtle px-1 py-px font-mono text-[12px] text-ink">{children}</code>
  );
}

export function AddHostSheet() {
  const { cos, ui } = useApp();
  const { busy, error, run } = useSheetAction();
  const [name, setName] = useState("");
  const [address, setAddress] = useState("");
  const typed = address.trim();
  const fallbackName = typed ? nameFromAddress(typed) : "";
  const submit = () => {
    if (!typed) return;
    run(async () => {
      const host = await cos.addHost(name.trim() || fallbackName, typed);
      ui.close();
      ui.navigate({ kind: "waiting" });
      await cos.switchHost(host.id);
    });
  };
  return (
    <Dialog
      title="Add a Mac"
      description="Use cos on another Mac of yours, over Tailscale."
      onClose={ui.close}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={ui.close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={busy} disabled={!typed}>
            Add and connect
          </Button>
        </>
      }
    >
      <TailnetPicker
        address={address}
        onFill={(peer) => {
          setName(peer.name);
          setAddress(peer.address);
        }}
      />
      <div className="flex items-center gap-3 text-[12px] text-ink-3" aria-hidden>
        <span className="h-px flex-1 bg-line" />
        or enter an address
        <span className="h-px flex-1 bg-line" />
      </div>
      <div className="flex flex-col gap-1.5">
        <div className="grid grid-cols-[minmax(0,1fr)_150px] gap-3">
          <Field label="Address" htmlFor="host-address">
            <input
              id="host-address"
              data-autofocus
              className={cx(inputClass, "font-mono text-[13px]")}
              placeholder="my-mac:7700 or 100.x.y.z"
              value={address}
              onChange={(event) => setAddress(event.target.value)}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </Field>
          <Field label="Name" htmlFor="host-name">
            <input
              id="host-name"
              className={inputClass}
              placeholder={fallbackName || "Studio"}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
        </div>
        <p className="text-[12px] leading-snug text-ink-3">
          Its Tailscale name or IP address; the port is 7700 unless you chose another. The name is
          how the app shows it, and optional.
        </p>
      </div>
      <div className="rounded-lg bg-subtle px-3 py-2.5 text-[12.5px] leading-relaxed text-ink-2">
        On that machine, install cos and run <Code>cos daemon install</Code> first. When this Mac
        first connects, approve it there with <Code>cos devices approve</Code> or in its cos app.
      </div>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

export function RemoveHostSheet({ hostId }: { hostId: string }) {
  const { cos, ui } = useApp();
  const host = useCos((s) => s.hosts.find((h) => h.id === hostId));
  const inUse = useCos((s) => s.host?.id === hostId);
  const { busy, error, run } = useSheetAction();
  if (!host) return null;
  const submit = () =>
    run(async () => {
      await cos.removeHost(hostId);
      ui.close();
      if (inUse) ui.navigate({ kind: "waiting" });
    });
  return (
    <Dialog
      title={`Remove ${host.name}?`}
      description={
        <>
          The app forgets <span className="font-mono text-[12.5px]">{host.address}</span>
          {inUse ? " and goes back to This Mac" : ""}. Nothing changes on that Mac: this Mac stays
          approved there until you remove it with <Code>cos devices</Code>.
        </>
      }
      width="sm"
      onClose={ui.close}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={ui.close}>
            Cancel
          </Button>
          <Button type="submit" variant="destructive" busy={busy}>
            Remove
          </Button>
        </>
      }
    >
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
