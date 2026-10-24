import { describe, expect, it } from "vitest";
import type { Bridge, DaemonResponse, HttpMethod } from "../bridge/types";
import { DaemonClient, fromBase64, toBase64 } from "./client";
import { CosError } from "./errors";

interface Call {
  method: HttpMethod;
  path: string;
  body: unknown;
}

const agent = { id: 1, workspace_id: 3, state: "starting", title: "A" };
const approval = { id: 4, agent_id: 1, tool_name: "Bash", state: "allowed" };
const workspace = { id: 3, project_id: 1, name: "lisbon", title: "lisbon", path: "/w" };

/** Answers every request with `body`, or `respond`'s answer, and records it. */
function recording(respond: (call: Call) => DaemonResponse | Promise<DaemonResponse>) {
  const calls: Call[] = [];
  const bridge = {
    daemonRequest: async (method: HttpMethod, path: string, body?: unknown) => {
      const call = { method, path, body };
      calls.push(call);
      return respond(call);
    },
  } as unknown as Bridge;
  return { calls, client: new DaemonClient(bridge) };
}

const ok = (body: unknown): DaemonResponse => ({ status: 200, body });

describe("DaemonClient routes", () => {
  type Case = [string, (c: DaemonClient) => Promise<unknown>, unknown, HttpMethod, string, unknown];
  const cases: Case[] = [
    ["status", (c) => c.status(), { version: "1", pid: 1 }, "GET", "/v1/status", undefined],
    [
      "events",
      (c) => c.events(5, 10),
      { events: [] },
      "GET",
      "/v1/events?after=5&limit=10",
      undefined,
    ],
    ["projects", (c) => c.projects(), { projects: [] }, "GET", "/v1/projects", undefined],
    [
      "addProject",
      (c) => c.addProject("/repo"),
      { id: 1, name: "r", root: "/repo" },
      "POST",
      "/v1/projects",
      { path: "/repo" },
    ],
    ["removeProject", (c) => c.removeProject(2), null, "DELETE", "/v1/projects/2", undefined],
    ["env", (c) => c.env(), { variables: {} }, "GET", "/v1/env", undefined],
    [
      "projectEnv",
      (c) => c.projectEnv(1),
      { variables: {}, global: {}, effective: {} },
      "GET",
      "/v1/projects/1/env",
      undefined,
    ],
    [
      "setEnv (global)",
      (c) => c.setEnv(null, "CLAUDE_CONFIG_DIR", "~/.claude-personal"),
      null,
      "PUT",
      "/v1/env/CLAUDE_CONFIG_DIR",
      { value: "~/.claude-personal" },
    ],
    [
      "setEnv (project)",
      (c) => c.setEnv(2, "A_B", ""),
      null,
      "PUT",
      "/v1/projects/2/env/A_B",
      { value: "" },
    ],
    ["removeEnv (global)", (c) => c.removeEnv(null, "X"), null, "DELETE", "/v1/env/X", undefined],
    [
      "removeEnv (project)",
      (c) => c.removeEnv(2, "X"),
      null,
      "DELETE",
      "/v1/projects/2/env/X",
      undefined,
    ],
    ["staff", (c) => c.staff(1), { project_id: 1 }, "GET", "/v1/projects/1/staff", undefined],
    [
      "setStaff",
      (c) => c.setStaff(1, { model: "opus" }),
      { project_id: 1 },
      "PUT",
      "/v1/projects/1/staff",
      { model: "opus" },
    ],
    [
      "startStaff",
      (c) => c.startStaff(1),
      { project_id: 1, agent_id: 14 },
      "POST",
      "/v1/projects/1/staff/start",
      undefined,
    ],
    [
      "newStaffSession",
      (c) => c.newStaffSession(1),
      { project_id: 1, agent_id: 14 },
      "POST",
      "/v1/projects/1/staff/new-session",
      undefined,
    ],
    [
      "messageStaff",
      (c) => c.messageStaff(1, "Plan it"),
      null,
      "POST",
      "/v1/projects/1/staff/message",
      { text: "Plan it" },
    ],
    [
      "staffInbox",
      (c) => c.staffInbox(1),
      { items: [] },
      "GET",
      "/v1/projects/1/staff/inbox",
      undefined,
    ],
    [
      "transcript",
      (c) => c.transcript(14),
      { messages: [], next: 0 },
      "GET",
      "/v1/agents/14/transcript",
      undefined,
    ],
    [
      "transcript after",
      (c) => c.transcript(14, 12),
      { messages: [], next: 12 },
      "GET",
      "/v1/agents/14/transcript?after=12",
      undefined,
    ],
    [
      "createWorkspace without a title",
      (c) => c.createWorkspace(1, "origin", "  "),
      workspace,
      "POST",
      "/v1/projects/1/workspaces",
      { base: "origin" },
    ],
    [
      "createWorkspace",
      (c) => c.createWorkspace(1, "head", "  Fix it "),
      workspace,
      "POST",
      "/v1/projects/1/workspaces",
      { base: "head", title: "Fix it" },
    ],
    [
      "renameWorkspace",
      (c) => c.renameWorkspace(3, "New"),
      workspace,
      "PATCH",
      "/v1/workspaces/3",
      { title: "New" },
    ],
    ["removeWorkspace", (c) => c.removeWorkspace(3), null, "DELETE", "/v1/workspaces/3", undefined],
    [
      "agents filtered",
      (c) => c.agents({ workspaceId: 3, state: "idle" }),
      { agents: [] },
      "GET",
      "/v1/agents?workspace_id=3&state=idle",
      undefined,
    ],
    [
      "createAgent",
      (c) =>
        c.createAgent(3, {
          prompt: " Fix ",
          model: "opus",
          permissionMode: "plan",
          userSettings: false,
          title: "",
        }),
      agent,
      "POST",
      "/v1/workspaces/3/agents",
      { prompt: "Fix", model: "opus", permission_mode: "plan", user_settings: false },
    ],
    ["sendInput", (c) => c.sendInput(1, "hi"), null, "POST", "/v1/agents/1/input", { text: "hi" }],
    [
      "sendKeys",
      (c) => c.sendKeys(1, new Uint8Array([27, 91, 65])),
      null,
      "POST",
      "/v1/agents/1/keys",
      { data: "G1tB" },
    ],
    [
      "resizeAgent",
      (c) => c.resizeAgent(1, 40, 120),
      null,
      "POST",
      "/v1/agents/1/resize",
      { rows: 40, cols: 120 },
    ],
    ["trustAgent", (c) => c.trustAgent(1), null, "POST", "/v1/agents/1/trust", undefined],
    ["stopAgent", (c) => c.stopAgent(1), null, "POST", "/v1/agents/1/stop", undefined],
    ["resumeAgent", (c) => c.resumeAgent(1), agent, "POST", "/v1/agents/1/resume", undefined],
    ["removeAgent", (c) => c.removeAgent(1), null, "DELETE", "/v1/agents/1", undefined],
    [
      "screen",
      (c) => c.screen(1),
      { rows: 2, cols: 3, text: "x", cursor: [0, 1] },
      "GET",
      "/v1/agents/1/screen",
      undefined,
    ],
    [
      "approvals pending",
      (c) => c.approvals(),
      { approvals: [] },
      "GET",
      "/v1/approvals?state=pending",
      undefined,
    ],
    [
      "approvals all",
      (c) => c.approvals(null),
      { approvals: [] },
      "GET",
      "/v1/approvals",
      undefined,
    ],
    [
      "decide",
      (c) => c.decide(4, "deny", " not now "),
      approval,
      "POST",
      "/v1/approvals/4",
      { decision: "deny", message: "not now" },
    ],
    ["devices", (c) => c.devices(), { devices: [] }, "GET", "/v1/devices", undefined],
    [
      "approveDevice",
      (c) => c.approveDevice(2),
      { id: 2, node_id: "n" },
      "POST",
      "/v1/devices/2/approve",
      undefined,
    ],
    ["removeDevice", (c) => c.removeDevice(2), null, "DELETE", "/v1/devices/2", undefined],
    ["tailnet", (c) => c.tailnet(), { enabled: false }, "GET", "/v1/tailnet", undefined],
  ];

  it.each(cases)("%s", async (_name, call, body, method, path, sent) => {
    const { calls, client } = recording(() => ok(body));
    await call(client);
    expect(calls).toEqual([{ method, path, body: sent }]);
  });
});

describe("DaemonClient errors", () => {
  it("carries the daemon's message and maps statuses to kinds", async () => {
    const statuses: [number, string][] = [
      [404, "not_found"],
      [409, "conflict"],
      [400, "http"],
      [500, "http"],
    ];
    for (const [status, kind] of statuses) {
      const { client } = recording(() => ({ status, body: { error: `nope ${status}` } }));
      const error = await client.stopAgent(1).catch((e: unknown) => e);
      expect(error).toBeInstanceOf(CosError);
      expect(error).toMatchObject({ kind, status, message: `nope ${status}` });
    }
  });

  it("turns the bridge's rejection into a CosError", async () => {
    const { client } = recording(() => {
      throw { kind: "not_running", message: "cos isn't running" };
    });
    await expect(client.status()).rejects.toMatchObject({
      kind: "not_running",
      message: "cos isn't running",
    });
  });

  it("calls an answer that doesn't decode a bad response", async () => {
    const { client } = recording(() => ok({ nope: true }));
    await expect(client.agent(1)).rejects.toMatchObject({ kind: "bad_response" });
  });

  it("round-trips base64", () => {
    const bytes = new Uint8Array(70_000).map((_, i) => i % 256);
    expect(fromBase64(toBase64(bytes))).toEqual(bytes);
  });
});
