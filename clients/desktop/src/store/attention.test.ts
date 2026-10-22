import { describe, expect, it } from "vitest";
import { decodeApproval, decodeQuestion } from "../api/models";
import { fakeStore, settle } from "../test/harness";
import { followAttention } from "./attention";
import { waitingCount } from "./selectors";

describe("attention", () => {
  it("tells the shell the waiting count whenever it changes", async () => {
    const { cos, bridge } = fakeStore();
    const stop = followAttention(cos, { hasFocus: () => true });
    await cos.start();
    await settle();
    const count = waitingCount(cos.get());
    expect(count).toBeGreaterThan(0);
    expect(bridge.calls.setWaitingCount.at(-1)).toBe(count);

    const calls = bridge.calls.setWaitingCount.length;
    cos.state.setState({ events: [] }); // nothing that changes the count
    expect(bridge.calls.setWaitingCount).toHaveLength(calls);
    stop();
  });

  it("notifies for new approvals and questions only while unfocused", async () => {
    const { cos, bridge } = fakeStore();
    let focused = false;
    followAttention(cos, { hasFocus: () => focused });
    await cos.start();
    await settle();
    // What was there at load doesn't notify.
    expect(bridge.calls.notify).toHaveLength(0);

    const s = cos.get();
    cos.state.setState({
      approvals: [
        ...s.approvals,
        decodeApproval({
          id: 900,
          agent_id: 7,
          tool_name: "Bash",
          tool_input: { command: "rm -rf build" },
          route: "user",
        }),
        // Routed to a chief: no notification.
        decodeApproval({ id: 901, agent_id: 21, tool_name: "Bash", route: "chief" }),
      ],
    });
    expect(bridge.calls.notify).toHaveLength(1);
    expect(bridge.calls.notify[0]!.body).toBe("rm -rf build");

    focused = true;
    cos.state.setState({
      questions: [...s.questions, decodeQuestion({ id: 77, run_id: 10, question: "Now?" })],
    });
    expect(bridge.calls.notify).toHaveLength(1);

    focused = false;
    cos.state.setState({
      questions: [
        ...cos.get().questions,
        decodeQuestion({ id: 78, run_id: 10, question: "And now?" }),
      ],
    });
    expect(bridge.calls.notify.at(-1)).toEqual({ title: "The chief asks", body: "And now?" });
  });

  it("reports zero while the daemon is away", async () => {
    const { cos, bridge } = fakeStore({ running: false });
    followAttention(cos, { hasFocus: () => true });
    await cos.start();
    await settle();
    expect(cos.get().connection).toBe("not_running");
    expect(bridge.calls.setWaitingCount).toEqual([0]);
  });
});
