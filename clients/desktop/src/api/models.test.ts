import { describe, expect, it } from "vitest";
import { page } from "./decode";
import {
  decodeAgent,
  decodeApproval,
  decodeBoard,
  decodeEvent,
  decodeProject,
  decodeRun,
  decodeStatus,
  decodeTask,
  decodeWorkspace,
  cityOf,
  projectSlug,
  workspaceName,
} from "./models";
import { decodeInbox, decodeStaff, decodeTranscript } from "./staff";

describe("lenient decoding", () => {
  it("ignores unknown fields and keeps unknown states", () => {
    const agent = decodeAgent({
      id: 1,
      workspace_id: 2,
      state: "hibernating",
      role: "auditor",
      brand_new_field: { x: 1 },
    });
    expect(agent.state).toBe("hibernating");
    expect(agent.role).toBe("auditor");
    expect(agent.title).toBe("Agent 1");
  });

  it("fills defaults for missing optional fields", () => {
    const agent = decodeAgent({ id: 1, workspace_id: 2 });
    expect(agent).toMatchObject({
      state: "starting",
      role: "user",
      runId: null,
      model: null,
      userSettings: true,
      cwd: null,
    });
  });

  it("reads ids sent as strings", () => {
    const agent = decodeAgent({ id: "5", workspace_id: "2", run_id: "9" });
    expect(agent.id).toBe(5);
    expect(agent.runId).toBe(9);
  });

  it("throws only when a required field is missing", () => {
    expect(() => decodeAgent({ workspace_id: 2 })).toThrow();
    expect(() => decodeApproval({ id: 1 })).toThrow();
  });

  it("drops list items that don't decode instead of failing the list", () => {
    const agents = page(
      { agents: [{ id: 1, workspace_id: 1 }, { nope: true }, "junk", { id: 2, workspace_id: 1 }] },
      "agents",
      decodeAgent,
    );
    expect(agents.map((a) => a.id)).toEqual([1, 2]);
  });

  it("accepts a bare array where a page is expected", () => {
    expect(page([{ id: 1, workspace_id: 1 }], "agents", decodeAgent)).toHaveLength(1);
  });

  it("decodes approvals from a daemon without routes", () => {
    const approval = decodeApproval({ id: 1, agent_id: 2, tool_name: "Write" });
    expect(approval).toMatchObject({ route: null, tier: null, reason: null, state: "pending" });
    expect(approval.summary).toBe("Write");
  });

  it("parses ISO times to milliseconds", () => {
    const status = decodeStatus({
      version: "0.1.0",
      pid: 3,
      started_at: "2026-01-02T03:04:05Z",
      last_seq: 10,
    });
    expect(status.startedAt).toBe(Date.UTC(2026, 0, 2, 3, 4, 5));
    const event = decodeEvent({ seq: 1, ts: "2026-01-02T03:04:05Z", kind: "x", data: null });
    expect(event.data).toEqual({});
    expect(event.runId).toBeNull();
  });

  it("decodes projects with their workspaces, skipping broken ones", () => {
    const project = decodeProject({
      id: 1,
      name: "cos",
      root: "/r",
      workspaces: [{ id: 2, project_id: 1, name: "lisbon", path: "/w" }, { id: "x" }],
    });
    expect(project.workspaces.map((w) => w.id)).toEqual([2]);
    // An older daemon without `city` or `branch`: the name stands in.
    expect(project.workspaces[0]!.title).toBeNull();
    expect(workspaceName(project.workspaces[0]!)).toBe("lisbon");
  });
});

describe("workspace names", () => {
  const ws = (fields: Record<string, unknown>) =>
    decodeWorkspace({ id: 3, project_id: 1, path: "/w", ...fields });

  it("shows the title, else the city", () => {
    expect(workspaceName(ws({ title: "Fix the login flow", city: "lisbon" }))).toBe(
      "Fix the login flow",
    );
    expect(workspaceName(ws({ title: null, city: "lisbon" }))).toBe("lisbon");
    // A blank title is no title.
    expect(workspaceName(ws({ title: "  ", city: "lisbon" }))).toBe("lisbon");
  });

  it("takes the city from the branch's last segment when the daemon doesn't send it", () => {
    const w = ws({ title: null, branch: "topflexi/lisbon-v1" });
    expect(w.city).toBe("lisbon-v1");
    expect(workspaceName(w)).toBe("lisbon-v1");
    expect(cityOf("lisbon")).toBe("lisbon");
  });

  it("slugs project names as the daemon does", () => {
    expect(projectSlug("TopFlexi")).toBe("topflexi");
    expect(projectSlug("  My App (v2)!")).toBe("my-app-v2");
    expect(projectSlug("opal-app")).toBe("opal-app");
  });

  it("reads an agent's spawned_by and transcript_path, and a missing workspace", () => {
    const agent = decodeAgent({
      id: 14,
      role: "staff",
      spawned_by: null,
      transcript_path: "/t.jsonl",
    });
    expect(agent.workspaceId).toBeNull();
    expect(agent.transcriptPath).toBe("/t.jsonl");
    expect(decodeAgent({ id: 15, workspace_id: 3, spawned_by: "14" }).spawnedBy).toBe(14);
  });
});

describe("staff decoding", () => {
  it("decodes a chief before it starts", () => {
    const staff = decodeStaff({ project_id: 1, agent_id: null, state: null, model: null });
    expect(staff).toMatchObject({
      projectId: 1,
      agentId: null,
      state: null,
      model: null,
      workingAgents: 0,
      waitingOnYou: 0,
    });
  });

  it("decodes a transcript, sorting it and skipping odd lines", () => {
    const transcript = decodeTranscript({
      messages: [
        { n: 3, role: "assistant", kind: "tool", tool: { name: "Bash", summary: "npm test" } },
        { n: 1, role: "user", kind: "text", text: "Hi", ts_ms: 5 },
        { role: "assistant", text: "no n" },
        { n: 2, role: "user", text: "[cos · 2 updates]\n- …" },
        { n: 4, role: "assistant", kind: "thinking_v2", text: "?" },
      ],
      next: 4,
    });
    expect(transcript.messages.map((m) => m.n)).toEqual([1, 2, 3, 4]);
    expect(transcript.messages[1]!.kind).toBe("update");
    expect(transcript.messages[2]!.tool).toEqual({
      name: "Bash",
      summary: "npm test",
      isError: false,
    });
    expect(transcript.messages[3]!.kind).toBe("thinking_v2");
    expect(transcript.next).toBe(4);
    expect(decodeTranscript({ messages: [], next: 0 })).toEqual({ messages: [], next: 0 });
  });

  it("decodes inbox items newest first, whatever their text field is called", () => {
    const items = decodeInbox({
      items: [
        { id: 1, kind: "stop", agent_id: 7, message: "Done.", created_ms: 10 },
        { kind: "approval", agent_id: 8, summary: "npm install", ts_ms: 20 },
      ],
    });
    expect(items.map((i) => i.text)).toEqual(["npm install", "Done."]);
    expect(items[1]!.key).toBe("1");
  });
});

describe("lenient decoding of runs", () => {
  it("counts a run's tasks from its tasks object", () => {
    const run = decodeRun({
      id: 1,
      workspace_id: 1,
      goal: "G",
      state: "running",
      tasks: { merged: 2, running: 1 },
    });
    expect(run.counts).toMatchObject({ merged: 2, running: 1, total: 3 });
  });

  it("decodes a board, sorting tasks and surviving a broken workspace", () => {
    const board = decodeBoard({
      run: { id: 1, workspace_id: 1, goal: "G", state: "running" },
      workspace: { broken: true },
      tasks: [
        { id: 11, title: "B", state: "ready", position: 1 },
        { id: 10, title: "A", state: "merged", position: 0, description: "old name" },
      ],
    });
    expect(board.workspace).toBeNull();
    expect(board.tasks.map((t) => t.id)).toEqual([10, 11]);
    expect(board.tasks[0]!.instructions).toBe("old name");
    expect(board.counts).toMatchObject({ merged: 1, ready: 1, total: 2 });
  });

  it("decodes task checks and reviews", () => {
    const task = decodeTask({
      id: 1,
      title: "T",
      state: "review",
      checks: {
        scope: { ok: false, out_of_scope: ["a.rs"] },
        tests: { status: "failed", exit_code: 1 },
        commits_ahead: 2,
      },
      review: { verdict: "changes", notes: "add a test", agent_id: 4 },
      depends_on: [3, "4"],
    });
    expect(task.checks?.scope).toMatchObject({
      ok: false,
      outOfScope: ["a.rs"],
      wrongBranch: false,
    });
    expect(task.checks?.tests).toMatchObject({ status: "failed", exitCode: 1 });
    expect(task.checks?.commitsAhead).toBe(2);
    expect(task.review).toEqual({ verdict: "changes", notes: "add a test", agentId: 4 });
    expect(task.dependsOn).toEqual([3, 4]);
  });
});
