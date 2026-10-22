import { describe, expect, it } from "vitest";
import { CosError } from "../api/errors";
import { fakeStore, rawEvent, requestLines, settle } from "../test/harness";
import { CoalescedRefresh } from "./coalesce";
import { approvalsForYou } from "./selectors";

async function startedStore(options: Parameters<typeof fakeStore>[1] = {}) {
  const harness = fakeStore({}, options);
  await harness.cos.start();
  await settle();
  harness.daemon.requests.length = 0;
  return harness;
}

describe("refresh rules", () => {
  const projects = "GET /v1/projects";
  const agents = "GET /v1/agents";
  const approvals = "GET /v1/approvals?state=pending";
  const runs = "GET /v1/runs";
  const questions = "GET /v1/questions?state=open";

  const cases: [string, string[]][] = [
    ["project.added", [projects]],
    ["workspace.created", [projects]],
    ["workspace.removed", [projects, agents, runs]],
    ["agent.started", [agents]],
    ["agent.state_changed", [agents]],
    ["agent.exited", [agents, approvals]],
    ["agent.removed", [agents, approvals]],
    ["approval.requested", [agents, approvals]],
    ["approval.decided", [agents, approvals]],
    ["approval.routed", [agents, approvals]],
    ["run.created", [runs]],
    ["run.state_changed", [runs, questions]],
    ["plan.proposed", [runs]],
    ["task.state_changed", [runs]],
    ["question.asked", [questions]],
    ["question.answered", [questions]],
    ["daemon.started", []],
    ["something.new", []],
  ];

  it.each(cases)("%s refetches %j", async (kind, expected) => {
    const { cos, daemon } = await startedStore();
    cos.handle({ type: "event", event: rawEvent(1000, kind, {}) });
    await settle();
    expect(new Set(requestLines(daemon))).toEqual(new Set(expected));
  });

  it("refetches a watched run's board only for its own events", async () => {
    const { cos, daemon } = await startedStore();
    const unwatch = cos.watchRun(10);
    await settle();
    daemon.requests.length = 0;

    cos.handle({ type: "event", event: rawEvent(1000, "task.state_changed", { run_id: 11 }) });
    await settle();
    expect(requestLines(daemon)).not.toContain("GET /v1/runs/10");

    cos.handle({ type: "event", event: rawEvent(1001, "task.state_changed", { run_id: 10 }) });
    await settle();
    expect(requestLines(daemon)).toContain("GET /v1/runs/10");

    // Limits follow every approval, whichever run: the chief's count may move.
    daemon.requests.length = 0;
    cos.handle({ type: "event", event: rawEvent(1002, "approval.decided", { approval_id: 1 }) });
    await settle();
    expect(requestLines(daemon)).toContain("GET /v1/runs/10/limits");
    expect(requestLines(daemon)).toContain("GET /v1/approvals");

    unwatch();
    daemon.requests.length = 0;
    cos.handle({ type: "event", event: rawEvent(1003, "task.state_changed", { run_id: 10 }) });
    await settle();
    expect(requestLines(daemon)).not.toContain("GET /v1/runs/10");
  });

  it("asks for a report once a watched run is done, and keeps 404 as not written", async () => {
    const { cos } = await startedStore();
    cos.watchRun(7);
    await settle();
    expect(cos.get().reports[7]?.kind).toBe("markdown");

    cos.watchRun(13); // reporting, no report yet
    await settle();
    expect(cos.get().reports[13]).toEqual({ kind: "not_written" });
  });

  it("folds a burst of run events into one request after the debounce", async () => {
    const { cos, daemon } = await startedStore({ runsDebounceMs: 20 });
    for (let i = 0; i < 12; i++) {
      cos.handle({
        type: "event",
        event: rawEvent(1000 + i, "task.state_changed", { run_id: 10 }),
      });
    }
    await settle(4, 30);
    expect(requestLines(daemon).filter((l) => l === "GET /v1/runs")).toHaveLength(1);
  });

  it("appends events in order and keeps the status counts current", async () => {
    const { cos } = await startedStore();
    const before = cos.get().status!;
    cos.handle({ type: "event", event: rawEvent(before.lastSeq + 1, "daemon.ping") });
    cos.handle({ type: "event", event: rawEvent(before.lastSeq + 2, "daemon.ping") });
    await settle();
    const after = cos.get();
    expect(after.events.at(-1)?.seq).toBe(before.lastSeq + 2);
    expect(after.status?.lastSeq).toBe(before.lastSeq + 2);
    expect(after.status?.events).toBe(before.events + 2);
  });

  it("drops events that don't decode", async () => {
    const { cos } = await startedStore();
    const count = cos.get().events.length;
    cos.handle({ type: "event", event: { kind: 42 } });
    await settle();
    expect(cos.get().events).toHaveLength(count);
  });
});

describe("connection", () => {
  it("is not_running when the daemon is stopped, and recovers", async () => {
    const { cos, daemon } = fakeStore({ running: false });
    await cos.start();
    await settle();
    expect(cos.get().connection).toBe("not_running");

    daemon.setRunning(true);
    await settle();
    // The fake's stream reconnects by itself, as the shell's does.
    cos.handle({ type: "connected", status: daemon.status() });
    await settle();
    expect(cos.get().connection).toBe("running");
    expect(cos.get().projects.length).toBeGreaterThan(0);
  });

  it("goes back to an empty log on log_reset", async () => {
    const { cos } = await startedStore();
    expect(cos.get().events.length).toBeGreaterThan(0);
    cos.handle({ type: "log_reset" });
    expect(cos.get().events).toHaveLength(0);
  });
});

describe("optimistic actions", () => {
  it("drops an approval as soon as it is decided", async () => {
    const { cos } = await startedStore();
    const approval = approvalsForYou(cos.get())[0]!;
    const decided = cos.decide(approval, "allow");
    await decided;
    expect(cos.get().approvals.some((a) => a.id === approval.id)).toBe(false);
  });

  it("drops an approval answered elsewhere (409) and rethrows", async () => {
    const { cos, daemon } = await startedStore();
    const approval = approvalsForYou(cos.get())[0]!;
    const raw = daemon.approvals.find((a) => a.id === approval.id)!;
    raw.state = "settled";
    await expect(cos.decide(approval, "allow")).rejects.toMatchObject({ kind: "conflict" });
    expect(cos.get().approvals.some((a) => a.id === approval.id)).toBe(false);
  });

  it("drops a question answered elsewhere (409) and rethrows", async () => {
    const { cos, daemon } = await startedStore();
    const question = cos.get().questions[0]!;
    daemon.questions.find((q) => q.id === question.id)!.state = "answered";
    await expect(cos.answer(question, "yes")).rejects.toBeInstanceOf(CosError);
    expect(cos.get().questions.some((q) => q.id === question.id)).toBe(false);
  });

  it("adds a created agent before the refetch", async () => {
    const { cos } = await startedStore();
    const agent = await cos.createAgent(3, { prompt: "Write the docs" });
    expect(cos.get().agents[0]?.id).toBe(agent.id);
  });

  it("stores the board an approved plan returns", async () => {
    const { cos } = await startedStore();
    await cos.approvePlan(11);
    expect(cos.get().boards[11]?.run.state).toBe("running");
    expect(cos.get().runs.find((r) => r.id === 11)?.state).toBe("running");
  });

  it("keeps the project list when removing fails", async () => {
    const { cos } = await startedStore();
    const before = cos.get().projects.length;
    // Project 1 has workspaces: the daemon refuses.
    await expect(cos.removeProject(1)).rejects.toMatchObject({ kind: "conflict" });
    expect(cos.get().projects).toHaveLength(before);
  });
});

describe("CoalescedRefresh", () => {
  function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return { promise, resolve };
  }

  it("runs one fetch in flight and one follow-up for a burst", async () => {
    const pending: ReturnType<typeof deferred<number>>[] = [];
    const applied: number[] = [];
    const refresh = new CoalescedRefresh(
      () => {
        const d = deferred<number>();
        pending.push(d);
        return d.promise;
      },
      (result) => result.ok && applied.push(result.value),
    );
    refresh.request();
    await settle(1);
    for (let i = 0; i < 10; i++) refresh.request();
    expect(pending).toHaveLength(1);

    pending[0]!.resolve(1);
    await settle(2);
    expect(pending).toHaveLength(2);
    // The first answer was overtaken by a newer request: dropped as stale.
    expect(applied).toEqual([]);

    pending[1]!.resolve(2);
    await settle(2);
    expect(applied).toEqual([2]);
    expect(refresh.busy).toBe(false);
  });

  it("applies nothing after dispose", async () => {
    const applied: unknown[] = [];
    const refresh = new CoalescedRefresh(
      () => Promise.resolve(1),
      (r) => applied.push(r),
    );
    refresh.request();
    refresh.dispose();
    await settle(2);
    expect(applied).toEqual([]);
    refresh.request();
    await settle(2);
    expect(applied).toEqual([]);
  });

  it("passes errors to apply", async () => {
    const results: boolean[] = [];
    const refresh = new CoalescedRefresh(
      () => Promise.reject(new Error("boom")),
      (r) => results.push(r.ok),
    );
    refresh.request();
    await settle(2);
    expect(results).toEqual([false]);
  });
});
