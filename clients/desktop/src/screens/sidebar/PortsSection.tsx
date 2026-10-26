// The Ports section at the bottom of the sidebar, while another machine is
// in use: its localhost ports forwarded to this Mac's, so a dev server
// running there opens here as localhost. It belongs to the machine, not to
// the project the sidebar shows (docs/desktop.md, "Ports").

import { ArrowUpRight, Plus, X } from "lucide-react";
import { type FormEvent, type KeyboardEvent, useState } from "react";
import { CosError } from "../../api/errors";
import { useApp, useCos } from "../../app/context";
import type { Forward } from "../../bridge/types";
import {
  type PortTone,
  checkLocalPort,
  checkRemotePort,
  moved,
  portLabel,
  portTitle,
  portTone,
} from "../../domain/ports";
import { isThisMac } from "../../store/store";
import { cx } from "../../ui/cx";

const dots: Record<PortTone, string> = {
  green: "bg-tone-green",
  amber: "bg-tone-amber",
  red: "bg-tone-red",
};

/** Hidden on This Mac, whose ports are already here. */
export function PortsSection() {
  const host = useCos((s) => s.host);
  const forwards = useCos((s) => s.forwards);
  const [adding, setAdding] = useState(false);
  const [tooOld, setTooOld] = useState<string | null>(null);
  if (isThisMac(host)) return null;
  const machine = host!.name;

  return (
    <section
      aria-label="Ports"
      className="max-h-[40%] shrink-0 overflow-y-auto border-t border-line px-2.5 pt-2 pb-2"
    >
      <div className="flex items-center px-1.5 pb-1">
        <span className="text-[11.5px] font-semibold tracking-wide text-ink-3 uppercase">
          Ports
        </span>
        <button
          type="button"
          aria-label={`Forward a port from ${machine}`}
          title={`Forward a port from ${machine} to this Mac`}
          aria-expanded={adding}
          onClick={() => {
            setTooOld(null);
            setAdding((a) => !a);
          }}
          className="ml-auto flex size-6 items-center justify-center rounded-md text-ink-3 hover:bg-subtle-2/70 hover:text-ink"
        >
          <Plus className="size-3.5" strokeWidth={2} />
        </button>
      </div>
      {tooOld ? (
        <p role="alert" className="px-1.5 pb-1 text-[12px] leading-snug text-ink-2">
          {tooOld}
        </p>
      ) : adding ? (
        <AddPort
          machine={machine}
          forwards={forwards}
          onDone={() => setAdding(false)}
          onTooOld={(message) => {
            setAdding(false);
            setTooOld(message);
          }}
        />
      ) : null}
      {forwards.length === 0 && !adding && !tooOld ? (
        <p className="px-1.5 text-[12px] leading-snug text-ink-3">
          + opens a dev server running on {machine} here, as localhost.
        </p>
      ) : forwards.length > 0 ? (
        <ul aria-label={`Ports forwarded from ${machine}`} className="flex flex-col gap-px">
          {forwards.map((forward) => (
            <PortRow key={forward.remotePort} forward={forward} machine={machine} />
          ))}
        </ul>
      ) : null}
    </section>
  );
}

function PortRow({ forward, machine }: { forward: Forward; machine: string }) {
  const { cos, ui } = useApp();
  const label = portLabel(forward);
  const local = `localhost:${forward.localPort}`;
  return (
    <li
      title={portTitle(forward, machine)}
      className="group/port flex h-8 items-center gap-2 rounded-lg pr-1 pl-2.5 text-[13px] text-ink-2 hover:bg-subtle"
    >
      <span
        aria-hidden
        className={cx("inline-block size-2 shrink-0 rounded-full", dots[portTone(forward)])}
      />
      <span className="shrink-0 font-mono text-[12.5px] text-ink">:{forward.remotePort}</span>
      {moved(forward) ? (
        <span className="shrink-0 font-mono text-[12px] whitespace-nowrap text-ink-3">
          → {forward.localPort}
        </span>
      ) : null}
      <span className="min-w-0 flex-1 truncate text-[11.5px] text-ink-3">{label}</span>
      {forward.connections > 0 ? (
        <span
          className="text-[11.5px] text-ink-3 tabular-nums"
          aria-label={`${forward.connections} open`}
        >
          {forward.connections}
        </span>
      ) : null}
      <button
        type="button"
        aria-label={`Open ${local}`}
        title={`Open ${local} in your browser`}
        onClick={() =>
          void ui.perform(`Couldn't open ${local}`, () => cos.openForward(forward.remotePort))
        }
        className="flex size-6 shrink-0 items-center justify-center rounded text-ink-3 hover:bg-subtle-2 hover:text-ink"
      >
        <ArrowUpRight className="size-3.5" strokeWidth={2} />
      </button>
      <button
        type="button"
        aria-label={`Stop forwarding :${forward.remotePort}`}
        title="Stop forwarding (drops its connections)"
        onClick={() =>
          void ui.perform(`Couldn't stop :${forward.remotePort}`, () =>
            cos.removeForward(forward.remotePort),
          )
        }
        className="hidden size-6 shrink-0 items-center justify-center rounded text-ink-3 group-focus-within/port:flex group-hover/port:flex hover:bg-subtle-2 hover:text-ink"
      >
        <X className="size-3" strokeWidth={2.2} />
      </button>
    </li>
  );
}

/**
 * The inline field for a port, checked as you type. Return adds it; Esc
 * closes it. More shows a field for another port on this Mac.
 */
function AddPort({
  machine,
  forwards,
  onDone,
  onTooOld,
}: {
  machine: string;
  forwards: Forward[];
  onDone: () => void;
  onTooOld: (message: string) => void;
}) {
  const { cos } = useApp();
  const [remote, setRemote] = useState("");
  const [local, setLocal] = useState("");
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remoteCheck = checkRemotePort(remote, forwards);
  const localCheck = more ? checkLocalPort(local, forwards) : { port: null, problem: null };
  const problem = remoteCheck.problem ?? localCheck.problem ?? error;
  const ready = remoteCheck.port !== null && localCheck.problem === null && !busy;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!ready || remoteCheck.port === null) return;
    setBusy(true);
    setError(null);
    try {
      await cos.addForward(remoteCheck.port, localCheck.port ?? undefined);
      onDone();
    } catch (e) {
      const failure = CosError.from(e);
      if (failure.code === "too_old") onTooOld(failure.message);
      else setError(failure.message);
      setBusy(false);
    }
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onDone();
    }
  };

  const field =
    "h-7 min-w-0 rounded-md border border-line-strong bg-surface px-2 font-mono text-[12.5px] text-ink outline-none placeholder:font-sans placeholder:text-ink-3 focus:border-ink-3/50";
  return (
    <form
      aria-label={`Forward a port from ${machine}`}
      onSubmit={(event) => void submit(event)}
      onKeyDown={onKeyDown}
      className="flex flex-col gap-1.5 px-1.5 pb-2"
    >
      <div className="flex items-center gap-1.5">
        <input
          autoFocus
          aria-label={`Port on ${machine}`}
          aria-invalid={remoteCheck.problem !== null}
          inputMode="numeric"
          spellCheck={false}
          autoComplete="off"
          placeholder="Port, like 5173"
          value={remote}
          onChange={(event) => {
            setRemote(event.target.value);
            setError(null);
          }}
          className={cx(field, "flex-1")}
        />
        {more ? null : (
          <button
            type="button"
            onClick={() => setMore(true)}
            className="h-7 shrink-0 rounded-md px-1.5 text-[12px] text-ink-3 hover:bg-subtle-2/70 hover:text-ink"
          >
            More
          </button>
        )}
      </div>
      {more ? (
        <input
          aria-label="Local port"
          aria-invalid={localCheck.problem !== null}
          inputMode="numeric"
          spellCheck={false}
          autoComplete="off"
          placeholder="Local port (the same by default)"
          value={local}
          onChange={(event) => {
            setLocal(event.target.value);
            setError(null);
          }}
          className={field}
        />
      ) : null}
      {/* With two fields, Return submits only through a submit button. */}
      <button type="submit" disabled={!ready} className="sr-only">
        Forward
      </button>
      {problem ? (
        <p role="alert" className="text-[11.5px] leading-snug text-tone-red">
          {problem}
        </p>
      ) : (
        <p className="text-[11.5px] leading-snug text-ink-3">
          {busy ? `Asking ${machine}…` : "Return forwards it to this Mac."}
        </p>
      )}
    </form>
  );
}
