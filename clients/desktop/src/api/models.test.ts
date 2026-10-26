import { describe, expect, it } from "vitest";
import { page } from "./decode";
import {
  decodeAgent,
  decodeApproval,
  decodeEvent,
  decodeProject,
  decodeStatus,
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
      spawnedBy: null,
      model: null,
      userSettings: true,
      cwd: null,
    });
  });

  it("reads ids sent as strings", () => {
    const agent = decodeAgent({ id: "5", workspace_id: "2", spawned_by: "9" });
    expect(agent.id).toBe(5);
    expect(agent.spawnedBy).toBe(9);
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

  it("decodes approvals from a daemon without the approvals policy", () => {
    const approval = decodeApproval({ id: 1, agent_id: 2, tool_name: "Write" });
    expect(approval).toMatchObject({ tier: null, reason: null, state: "pending" });
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
      agentId: null,
      output: null,
    });
    expect(transcript.messages[3]!.kind).toBe("thinking_v2");
    expect(transcript.next).toBe(4);
    expect(decodeTranscript({ messages: [], next: 0 })).toEqual({
      messages: [],
      next: 0,
      context: null,
    });
  });

  it("decodes a transcript's context, with or without a window", () => {
    const full = decodeTranscript({
      messages: [],
      next: 0,
      context: { used_tokens: 84_123, window_tokens: 200_000 },
    });
    expect(full.context).toEqual({ usedTokens: 84_123, windowTokens: 200_000 });
    const unknown = decodeTranscript({
      messages: [],
      context: { used_tokens: 5, window_tokens: null },
    });
    expect(unknown.context).toEqual({ usedTokens: 5, windowTokens: null });
    expect(decodeTranscript({ messages: [], context: { window_tokens: 9 } }).context).toBeNull();
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
