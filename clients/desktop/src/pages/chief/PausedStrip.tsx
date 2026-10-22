import { Pause, Play } from "lucide-react";
import { useState } from "react";
import { useApp, useCanChange } from "../../app/context";
import { pluralize } from "../../domain/labels";
import { Button } from "../../ui/kit";

/** "Coordination paused · 3 updates waiting", above the composer, with Resume. */
export function PausedStrip({ projectId, waiting }: { projectId: number; waiting: number }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState(false);
  return (
    <section
      aria-label="Coordination paused"
      className="mb-3 flex items-center gap-3 rounded-2xl border border-line-strong bg-subtle px-4 py-2"
    >
      <Pause className="size-3.5 shrink-0 text-ink-3" strokeWidth={2} aria-hidden />
      <p className="min-w-0 flex-1 truncate text-[13px] text-ink-2" role="status">
        <span className="font-medium text-ink">Coordination paused</span>
        {" · "}
        {waiting > 0 ? `${pluralize(waiting, "update")} waiting` : "no updates waiting"}
        <span className="text-ink-3"> · your messages still go through</span>
      </p>
      <Button
        size="sm"
        icon={Play}
        busy={busy}
        disabled={!canChange}
        onClick={() => {
          setBusy(true);
          void ui
            .perform("Couldn't resume coordination", () => cos.setStaffPaused(projectId, false))
            .finally(() => setBusy(false));
        }}
      >
        Resume
      </Button>
    </section>
  );
}
