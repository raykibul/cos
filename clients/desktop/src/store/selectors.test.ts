import { describe, expect, it } from "vitest";
import { decodeAgent, decodeApproval, decodeQuestion, decodeRun } from "../api/models";
import {
  agentNeedsYou,
  approvalsForRun,
  approvalsForYou,
  sidebarRuns,
  waitingCount,
  waitingCountForRun,
} from "./selectors";
import { type DaemonState, initialState } from "./store";

const agent = (id: number, state: string, extra: Record<string, unknown> = {}) =>
  decodeAgent({ id, workspace_id: 1, state, ...extra });
const approval = (id: number, agentId: number, extra: Record<string, unknown> = {}) =>
  decodeApproval({ id, agent_id: agentId, tool_name: "Bash", state: "pending", ...extra });
const run = (id: number, state: string, extra: Record<string, unknown> = {}) =>
  decodeRun({ id, workspace_id: 1, goal: `Run ${id}`, state, ...extra });

function state(partial: Partial<DaemonState>): DaemonState {
  return { ...initialState(), connection: "running", ...partial };
}

describe("waiting count", () => {
  it("counts approvals routed to you, trust dialogs, questions and plans", () => {
    const s = state({
      agents: [agent(1, "waiting_approval"), agent(2, "needs_trust"), agent(3, "working")],
      approvals: [
        approval(10, 1, { route: "user" }),
        // Routed to the chief: it decides, not you.
        approval(11, 3, { route: "chief" }),
        // An older daemon sends no route: yours.
        approval(12, 3),
      ],
      questions: [decodeQuestion({ id: 5, run_id: 9, question: "Which?", state: "open" })],
      runs: [run(9, "awaiting_plan_approval"), run(8, "running")],
    });
    expect(approvalsForYou(s).map((a) => a.id)).toEqual([10, 12]);
    expect(waitingCount(s)).toBe(2 + 1 + 1 + 1);
  });

  it("is zero with nothing pending", () => {
    expect(waitingCount(state({}))).toBe(0);
  });

  it("leaves out decided approvals", () => {
    const s = state({ approvals: [approval(10, 1, { state: "allowed" })] });
    expect(waitingCount(s)).toBe(0);
  });

  it("says an agent needs you only for your approvals and trust", () => {
    const waiting = agent(1, "waiting_approval");
    const chiefs = agent(2, "waiting_approval");
    const s = state({
      agents: [waiting, chiefs, agent(3, "needs_trust")],
      approvals: [approval(10, 1, { route: "user" }), approval(11, 2, { route: "chief" })],
    });
    expect(agentNeedsYou(s, waiting)).toBe(true);
    expect(agentNeedsYou(s, chiefs)).toBe(false);
    expect(agentNeedsYou(s, s.agents[2]!)).toBe(true);
  });

  it("counts a run's own plan, questions and approvals", () => {
    const s = state({
      agents: [agent(1, "waiting_approval", { run_id: 4 }), agent(2, "waiting_approval")],
      approvals: [approval(10, 1, { route: "user" }), approval(11, 2, { route: "user" })],
      questions: [decodeQuestion({ id: 5, run_id: 4, question: "Which?" })],
      runs: [run(4, "awaiting_plan_approval")],
    });
    expect(waitingCountForRun(s, 4)).toBe(3);
  });
});

describe("run lists", () => {
  it("merges live and full approval lists without duplicates, newest first", () => {
    const s = state({
      agents: [agent(1, "working", { run_id: 4 })],
      approvals: [approval(10, 1, { created_ms: 10 })],
      allApprovals: [
        approval(10, 1, { created_ms: 10 }),
        approval(9, 1, { state: "allowed", created_ms: 5 }),
        approval(8, 99, { created_ms: 20 }),
      ],
    });
    expect(approvalsForRun(s, 4).map((a) => a.id)).toEqual([10, 9]);
  });

  it("shows the current run, two finished ones and the selected one in the sidebar", () => {
    const runs = [
      run(6, "running"),
      run(5, "done"),
      run(4, "failed"),
      run(3, "cancelled"),
      run(2, "done"),
    ];
    const s = state({ runs });
    expect(sidebarRuns(s, 1, null).map((r) => r.id)).toEqual([6, 5, 4]);
    expect(sidebarRuns(s, 1, 2).map((r) => r.id)).toEqual([6, 5, 4, 2]);
  });
});
