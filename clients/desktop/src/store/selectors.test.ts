import { describe, expect, it } from "vitest";
import { decodeAgent, decodeApproval } from "../api/models";
import { agentNeedsYou, approvalsForYou, waitingCount } from "./selectors";
import { type DaemonState, initialState } from "./store";

const agent = (id: number, state: string, extra: Record<string, unknown> = {}) =>
  decodeAgent({ id, workspace_id: 1, state, ...extra });
const approval = (id: number, agentId: number, extra: Record<string, unknown> = {}) =>
  decodeApproval({ id, agent_id: agentId, tool_name: "Bash", state: "pending", ...extra });

function state(partial: Partial<DaemonState>): DaemonState {
  return { ...initialState(), connection: "running", ...partial };
}

describe("waiting count", () => {
  it("counts every pending approval and trust dialog", () => {
    const s = state({
      agents: [agent(1, "waiting_approval"), agent(2, "needs_trust"), agent(3, "working")],
      approvals: [approval(10, 1, { tier: "ask_user" }), approval(12, 3)],
    });
    expect(approvalsForYou(s).map((a) => a.id)).toEqual([10, 12]);
    expect(waitingCount(s)).toBe(2 + 1);
  });

  it("is zero with nothing pending", () => {
    expect(waitingCount(state({}))).toBe(0);
  });

  it("leaves out decided approvals", () => {
    const s = state({ approvals: [approval(10, 1, { state: "allowed" })] });
    expect(waitingCount(s)).toBe(0);
  });

  it("says an agent needs you for its approval or the trust dialog", () => {
    const waiting = agent(1, "waiting_approval");
    const working = agent(2, "working");
    const s = state({
      agents: [waiting, working, agent(3, "needs_trust")],
      approvals: [approval(10, 1)],
    });
    expect(agentNeedsYou(s, waiting)).toBe(true);
    expect(agentNeedsYou(s, working)).toBe(false);
    expect(agentNeedsYou(s, s.agents[2]!)).toBe(true);
  });
});
