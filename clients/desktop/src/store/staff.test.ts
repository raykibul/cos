import { describe, expect, it } from "vitest";
import { fakeStore, rawEvent, requestLines, settle } from "../test/harness";
import { chiefRequests, projectOfPage, staffState } from "./selectors";

async function startedStore() {
  const harness = fakeStore();
  await harness.cos.start();
  await settle();
  return harness;
}

describe("the Chief of Staff in the store", () => {
  it("fetches every project's chief on connect", async () => {
    const { cos, daemon } = await startedStore();
    const lines = requestLines(daemon);
    expect(lines).toContain("GET /v1/projects/1/staff");
    expect(lines).toContain("GET /v1/projects/2/staff");
    expect(cos.get().staffs[1]?.agentId).toBe(40);
    expect(cos.get().staffs[2]?.agentId).toBeNull();
    expect(staffState(cos.get(), 1)).toBe("idle");
    expect(staffState(cos.get(), 2)).toBeNull();
  });

  it("fetches only the new project's chief when one is added", async () => {
    const { cos, daemon } = await startedStore();
    daemon.requests.length = 0;
    const project = await cos.addProject("/Users/me/code/new-repo");
    await settle();
    const staff = requestLines(daemon).filter((l) => l.endsWith("/staff"));
    expect(staff).toEqual([`GET /v1/projects/${project.id}/staff`]);
  });

  it("refetches one chief on staff.started for its project", async () => {
    const { cos, daemon } = await startedStore();
    daemon.requests.length = 0;
    cos.handle({ type: "event", event: rawEvent(5000, "staff.started", { project_id: 2 }) });
    await settle();
    expect(new Set(requestLines(daemon))).toEqual(
      new Set(["GET /v1/projects/2/staff", "GET /v1/agents"]),
    );
  });

  it("refetches a watched inbox on staff.inbox, and only that one, with its chief's count", async () => {
    const { cos, daemon } = await startedStore();
    const unwatch = cos.watchInbox(1);
    await settle();
    expect(cos.get().inboxes[1]?.length).toBe(2);
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5000, "staff.inbox", { project_id: 2, count: 1 }),
    });
    await settle();
    // Its chief's updates_waiting moved; nobody watches its inbox.
    expect(requestLines(daemon)).toEqual(["GET /v1/projects/2/staff"]);
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5001, "staff.inbox", { project_id: 1, count: 3 }),
    });
    await settle();
    expect(new Set(requestLines(daemon))).toEqual(
      new Set(["GET /v1/projects/1/staff", "GET /v1/projects/1/staff/inbox"]),
    );
    unwatch();
  });

  it("reads a watched transcript, then only what follows on agent.transcript", async () => {
    const { cos, daemon } = await startedStore();
    daemon.requests.length = 0;
    const unwatch = cos.watchTranscript(40);
    await settle();
    expect(requestLines(daemon)).toEqual(["GET /v1/agents/40/transcript"]);
    const first = cos.get().transcripts[40]!;
    expect(first.loaded).toBe(true);
    expect(first.messages.length).toBe(12);
    expect(first.next).toBe(12);

    // Nothing new: no request.
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5000, "agent.transcript", { agent_id: 40, next: 12 }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual([]);

    // Another agent's transcript: not watched.
    cos.handle({
      type: "event",
      event: rawEvent(5001, "agent.transcript", { agent_id: 7, next: 3 }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual([]);

    daemon.staffDesk.transcripts[40]!.push({
      n: 13,
      role: "assistant",
      kind: "text",
      text: "One more thing",
      ts_ms: Date.now(),
    });
    cos.handle({
      type: "event",
      event: rawEvent(5002, "agent.transcript", { agent_id: 40, next: 13 }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual(["GET /v1/agents/40/transcript?after=12"]);
    expect(cos.get().transcripts[40]!.messages.at(-1)?.text).toBe("One more thing");
    unwatch();

    // After the page leaves, events cost nothing.
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5003, "agent.transcript", { agent_id: 40, next: 14 }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual([]);
  });

  it("reads a transcript that started over from the top", async () => {
    const { cos, daemon } = await startedStore();
    cos.watchTranscript(40);
    await settle();
    daemon.staffDesk.transcripts[40] = [
      { n: 1, role: "user", kind: "text", text: "Fresh start", ts_ms: Date.now() },
    ];
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5000, "agent.transcript", { agent_id: 40, next: 1 }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual(["GET /v1/agents/40/transcript"]);
    expect(cos.get().transcripts[40]!.messages.map((m) => m.text)).toEqual(["Fresh start"]);
  });

  it("shows a message at once, then drops the pending copy when the transcript has it", async () => {
    const { cos, daemon } = await startedStore();
    cos.watchTranscript(40);
    await settle();
    const sent = cos.messageStaff(1, "Start on the release notes");
    // Before the daemon answers.
    expect(cos.get().pending[1]).toMatchObject([
      { text: "Start on the release notes", state: "sending" },
    ]);
    await sent;
    await settle();
    expect(daemon.requests).toContainEqual({
      method: "POST",
      path: "/v1/projects/1/staff/message",
      body: { text: "Start on the release notes" },
    });
    expect(cos.get().pending[1]).toEqual([]);
    const messages = cos.get().transcripts[40]!.messages;
    expect(messages.filter((m) => m.text === "Start on the release notes")).toHaveLength(1);
  });

  it("starts a chief that hasn't started with the first message", async () => {
    const { cos } = await startedStore();
    await cos.messageStaff(2, "Hello");
    await settle();
    const agentId = cos.get().staffs[2]?.agentId;
    expect(agentId).not.toBeNull();
    cos.watchTranscript(agentId!);
    await settle();
    expect(cos.get().transcripts[agentId!]!.messages.map((m) => m.text)).toEqual(["Hello"]);
    expect(cos.get().pending[2]).toEqual([]);
  });

  it("keeps a message that failed, to retry", async () => {
    const { cos, daemon } = await startedStore();
    daemon.setRunning(false);
    await cos.messageStaff(1, "Hi");
    expect(cos.get().pending[1]).toMatchObject([{ state: "failed" }]);
    daemon.setRunning(true);
    await cos.retryPending(1, cos.get().pending[1]![0]!.id);
    expect(cos.get().pending[1]![0]?.state).toBe("sent");
    cos.discardPending(1, cos.get().pending[1]![0]!.id);
    expect(cos.get().pending[1]).toEqual([]);
  });

  it("lists what the chief's agents wait on you for", async () => {
    const { cos } = await startedStore();
    const requests = chiefRequests(cos.get(), 1);
    expect(requests.map((r) => [r.agent.id, r.approval?.id])).toEqual([[8, 12]]);
    expect(chiefRequests(cos.get(), 2)).toEqual([]);
  });

  it("finds the project a page belongs to", async () => {
    const { cos } = await startedStore();
    expect(projectOfPage(cos.get(), { kind: "chief", id: 2 })).toBe(2);
    expect(projectOfPage(cos.get(), { kind: "workspace", id: 8 })).toBe(2);
    expect(projectOfPage(cos.get(), { kind: "agent", id: 7 })).toBe(1);
    expect(projectOfPage(cos.get(), { kind: "agent", id: 40 })).toBe(1);
    expect(projectOfPage(cos.get(), { kind: "waiting" })).toBeNull();
  });

  it("sets the chief's model", async () => {
    const { cos, daemon } = await startedStore();
    await cos.setStaffModel(1, null);
    expect(daemon.requests).toContainEqual({
      method: "PUT",
      path: "/v1/projects/1/staff",
      body: { model: null },
    });
    expect(cos.get().staffs[1]?.model).toBeNull();
  });
});
