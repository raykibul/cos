// The line along the bottom: how many agents are working, what waits on you,
// and the shortcuts that move you around.

import { useApp, useCos } from "../app/context";
import { pluralize } from "../domain/labels";
import { waitingCount, workingCount } from "../store/selectors";

export function StatusBar() {
  const { ui } = useApp();
  const running = useCos((s) => s.connection === "running");
  const working = useCos(workingCount);
  const waiting = useCos(waitingCount);
  return (
    <footer
      aria-label="Status"
      className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-sidebar px-4 text-[11.5px] whitespace-nowrap text-ink-3 select-none"
    >
      {running ? (
        <>
          <span>{working > 0 ? `${pluralize(working, "agent")} working` : "Nothing running"}</span>
          {waiting > 0 ? (
            <button
              type="button"
              onClick={() => ui.navigate({ kind: "waiting" })}
              className="rounded px-1 font-medium text-accent-ink hover:bg-subtle"
            >
              {waiting} waiting on you
            </button>
          ) : null}
        </>
      ) : null}
      <span className="ml-auto truncate font-mono text-[10.5px]">
        ⌘K jump · ⌘1–9 chiefs · ⌘W close tab
      </span>
    </footer>
  );
}
