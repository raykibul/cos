import { Check, Copy, PowerOff } from "lucide-react";
import { useState } from "react";
import { useCos } from "../app/context";
import { PageHeader } from "../ui/Page";

function CommandLine({ command, note }: { command: string; note: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div>
      <div className="flex items-center gap-2 rounded-xl border border-line bg-surface py-1.5 pr-1.5 pl-4 shadow-card">
        <code className="min-w-0 flex-1 truncate font-mono text-[13.5px] text-ink select-all">
          <span className="text-ink-3 select-none">$ </span>
          {command}
        </code>
        <button
          type="button"
          aria-label={`Copy ${command}`}
          onClick={() => {
            void navigator.clipboard?.writeText(command).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            });
          }}
          className="flex size-8 items-center justify-center rounded-lg text-ink-3 hover:bg-subtle hover:text-ink"
        >
          {copied ? (
            <Check className="size-4 text-tone-green" strokeWidth={2} />
          ) : (
            <Copy className="size-4" strokeWidth={1.8} />
          )}
        </button>
      </div>
      <p className="mt-1.5 px-1 text-[12.5px] text-ink-3">{note}</p>
    </div>
  );
}

/** Shown in place of any page while the daemon doesn't answer. The app keeps trying. */
export function NotRunning() {
  const info = useCos((s) => s.info);
  const reason = useCos((s) => s.connectionReason);
  return (
    <>
      <PageHeader />
      <div className="flex flex-1 items-start justify-center overflow-y-auto px-8 pt-[12vh] pb-12">
        <div className="w-full max-w-[520px]">
          <div className="mb-5 flex size-11 items-center justify-center rounded-full bg-subtle text-ink-3">
            <PowerOff className="size-5" strokeWidth={1.7} />
          </div>
          <h1 className="font-serif text-[28px] leading-tight font-medium text-ink">
            cos isn't running
          </h1>
          <p className="mt-2 text-[14px] leading-relaxed text-ink-2">
            This app is a window onto the cos daemon, and nothing answers on its socket. Start it in
            a terminal; the app connects by itself once it's up.
          </p>
          <div className="mt-6 flex flex-col gap-4">
            <CommandLine
              command="cos daemon"
              note="Runs the daemon in this terminal until you stop it."
            />
            <CommandLine
              command="cos daemon install"
              note="Or keep it running: starts it now and at every login."
            />
          </div>
          <div className="mt-8 flex items-center gap-2 text-[12px] text-ink-3">
            <span className="size-1.5 animate-breathe rounded-full bg-ink-3" aria-hidden />
            <span>
              Waiting for the daemon
              {info ? ` at ${info.socket.replace(/^\/Users\/[^/]+/, "~")}` : ""}…
            </span>
          </div>
          {reason ? <p className="mt-1 text-[12px] text-ink-3 select-text">{reason}</p> : null}
        </div>
      </div>
    </>
  );
}
