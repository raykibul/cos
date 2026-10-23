// Pieces several pages share: approval buttons.

import { useState } from "react";
import { isConflict } from "../api/errors";
import type { Approval } from "../api/models";
import { useApp, useCanChange } from "../app/context";
import { Button } from "../ui/kit";

/** Deny and Allow. A request already answered in the terminal just goes away. */
export function ApprovalButtons({
  approval,
  size = "md",
}: {
  approval: Approval;
  size?: "sm" | "md";
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState<"allow" | "deny" | null>(null);
  const decide = (decision: "allow" | "deny") => {
    setBusy(decision);
    void ui
      .perform("Couldn't answer the request", () =>
        cos.decide(approval, decision).catch((error: unknown) => {
          // Answered in the terminal meanwhile; nothing left to do.
          if (!isConflict(error)) throw error;
        }),
      )
      .finally(() => setBusy(null));
  };
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Button
        size={size}
        disabled={!canChange || busy !== null}
        busy={busy === "deny"}
        onClick={() => decide("deny")}
      >
        Deny
      </Button>
      <Button
        size={size}
        variant="accent"
        disabled={!canChange || busy !== null}
        busy={busy === "allow"}
        onClick={() => decide("allow")}
      >
        Allow
      </Button>
    </div>
  );
}
