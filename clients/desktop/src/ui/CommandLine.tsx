import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { cx } from "./cx";

/** A shell command to copy and run, with an optional line under it. */
export function CommandLine({
  command,
  note,
  className,
}: {
  command: string;
  note?: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={className}>
      <div className="flex items-center gap-2 rounded-xl border border-line bg-surface py-1.5 pr-1.5 pl-4 shadow-card">
        <code
          className={cx(
            "min-w-0 flex-1 font-mono text-[13.5px] text-ink select-all",
            // Long commands (the firewall's) wrap rather than hide.
            command.length > 60 ? "break-all" : "truncate",
          )}
        >
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
          className="flex size-8 shrink-0 items-center justify-center rounded-lg text-ink-3 hover:bg-subtle hover:text-ink"
        >
          {copied ? (
            <Check className="size-4 text-tone-green" strokeWidth={2} />
          ) : (
            <Copy className="size-4" strokeWidth={1.8} />
          )}
        </button>
      </div>
      {note ? <p className="mt-1.5 px-1 text-[12.5px] text-ink-3">{note}</p> : null}
    </div>
  );
}
