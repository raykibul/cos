// The tray's count and notifications. The shell owns the tray icon and macOS
// notifications; the UI tells it what's waiting.

import { approvalDetail, isForYou } from "../domain/approvals";
import { approvalsForYou, pendingDevices, waitingCount } from "./selectors";
import type { CosStore, DaemonState } from "./store";

export interface AttentionOptions {
  /** Whether the window has focus; notifications are only for when it doesn't. */
  hasFocus?: () => boolean;
}

/**
 * Calls `set_waiting_count` whenever the count changes, and `notify` for an
 * approval routed to you, a chief's question or a device asking to reach this
 * Mac that appears while the window isn't focused. What was already there when the lists first load doesn't
 * notify. Returns a function that stops it.
 */
export function followAttention(store: CosStore, options: AttentionOptions = {}): () => void {
  const hasFocus = options.hasFocus ?? (() => document.hasFocus());
  const bridge = store.bridge;
  let lastCount: number | null = null;
  let seenApprovals: Set<number> | null = null;
  let seenQuestions: Set<number> | null = null;
  let seenDevices: Set<number> | null = null;

  const check = (s: DaemonState) => {
    const count = s.connection === "running" ? waitingCount(s) : (lastCount ?? 0);
    if (count !== lastCount) {
      lastCount = count;
      bridge.setWaitingCount(count).catch(() => undefined);
    }
    if (s.connection !== "running") return;

    const approvals = approvalsForYou(s);
    if (seenApprovals === null) {
      if (s.approvalsLoaded) seenApprovals = new Set(approvals.map((a) => a.id));
    } else {
      for (const approval of approvals) {
        if (seenApprovals.has(approval.id) || !isForYou(approval)) continue;
        seenApprovals.add(approval.id);
        if (!hasFocus()) {
          const agent = s.agents.find((a) => a.id === approval.agentId);
          const title = agent ? `${agent.title} needs approval` : "An agent needs approval";
          bridge.notify(title, approvalDetail(approval)).catch(() => undefined);
        }
      }
    }

    const devices = pendingDevices(s);
    if (seenDevices === null) {
      if (s.devicesLoaded) seenDevices = new Set(devices.map((d) => d.id));
    } else {
      for (const device of devices) {
        if (seenDevices.has(device.id)) continue;
        seenDevices.add(device.id);
        if (!hasFocus()) {
          const name = device.hostname || device.name;
          bridge
            .notify(`${name} wants to reach this Mac`, "Approve it in cos under Waiting on you.")
            .catch(() => undefined);
        }
      }
    }

    if (seenQuestions === null) {
      if (s.questionsLoaded) seenQuestions = new Set(s.questions.map((q) => q.id));
    } else {
      for (const question of s.questions) {
        if (seenQuestions.has(question.id)) continue;
        seenQuestions.add(question.id);
        if (!hasFocus()) {
          bridge.notify("The chief asks", question.question).catch(() => undefined);
        }
      }
    }
  };

  check(store.get());
  return store.state.subscribe(check);
}
