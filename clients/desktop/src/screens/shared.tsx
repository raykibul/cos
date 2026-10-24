// Pieces several pages share: approval buttons and their keys.

import { useEffect, useRef, useState } from "react";
import { isConflict } from "../api/errors";
import type { Approval } from "../api/models";
import { useApp, useCanChange } from "../app/context";
import { keysAreFree } from "../ui/keys";
import { Button } from "../ui/kit";

/**
 * Deny and Allow. A request already answered in the terminal just goes away.
 * With `keys`, ⌘↩ allows and ⌘⌫ denies it from anywhere on the page; give
 * them to one request per page.
 */
export function ApprovalButtons({
  approval,
  size = "md",
  keys = false,
}: {
  approval: Approval;
  size?: "sm" | "md";
  keys?: boolean;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState<"allow" | "deny" | null>(null);
  const disabled = !canChange || busy !== null;
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
  useApprovalKeys(keys && !disabled, decide);
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Button
        size={size}
        disabled={disabled}
        busy={busy === "deny"}
        onClick={() => decide("deny")}
        aria-keyshortcuts={keys ? "Meta+Backspace" : undefined}
      >
        Deny
        {keys ? <KeyHint>⌘⌫</KeyHint> : null}
      </Button>
      <Button
        size={size}
        variant="accent"
        disabled={disabled}
        busy={busy === "allow"}
        onClick={() => decide("allow")}
        aria-keyshortcuts={keys ? "Meta+Enter" : undefined}
      >
        Allow
        {keys ? <KeyHint>⌘↩</KeyHint> : null}
      </Button>
    </div>
  );
}

function KeyHint({ children }: { children: string }) {
  return (
    <kbd aria-hidden className="ml-0.5 font-sans text-[11px] opacity-65">
      {children}
    </kbd>
  );
}

/**
 * ⌘↩ and ⌘⌫ answer a request. They stay out of the way of a terminal, a
 * dialog and a text field you've typed in (where ⌘⌫ deletes the line).
 */
function useApprovalKeys(enabled: boolean, decide: (decision: "allow" | "deny") => void) {
  const { ui } = useApp();
  const latest = useRef(decide);
  useEffect(() => {
    latest.current = decide;
  });
  useEffect(() => {
    if (!enabled) return;
    const onKey = (event: KeyboardEvent) => {
      if (!event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
      const decision = event.key === "Enter" ? "allow" : event.key === "Backspace" ? "deny" : null;
      if (!decision || event.defaultPrevented || event.isComposing) return;
      const { dialog, switcher } = ui.store.getState();
      if (dialog || switcher || !keysAreFree(event.target)) return;
      event.preventDefault();
      latest.current(decision);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled, ui]);
}
