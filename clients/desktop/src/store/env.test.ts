import { describe, expect, it } from "vitest";
import {
  accountOf,
  decodeGlobalEnv,
  decodeProjectEnv,
  envHint,
  nameProblem,
  sortedNames,
  valueProblem,
} from "../api/env";
import { decodeAgent } from "../api/models";
import { loginCommand } from "../domain/env";
import { agentLabel } from "../domain/labels";
import { staffLabel } from "../pages/chief/staffState";
import { fakeStore, rawEvent, requestLines, settle } from "../test/harness";
import {
  agentNeedsYou,
  approvalsForYou,
  claudeAccount,
  configDirOf,
  loginAgents,
  pendingDevices,
  projectEnvNames,
  trustAgents,
  waitingCount,
} from "./selectors";

async function startedStore() {
  const harness = fakeStore();
  await harness.cos.start();
  await settle();
  return harness;
}

describe("environment decoding and helpers", () => {
  it("reads the lists leniently", () => {
    expect(decodeGlobalEnv({ variables: { A: "1", B: 2, C: true, D: null, E: {} } })).toEqual({
      A: "1",
      B: "2",
      C: "true",
    });
    expect(decodeGlobalEnv({})).toEqual({});
    expect(() => decodeGlobalEnv(null)).toThrow();
    expect(
      decodeProjectEnv({ variables: [{ name: "X", value: "p" }], global: { X: "g", Y: "g" } }),
    ).toEqual({ variables: { X: "p" }, global: { X: "g", Y: "g" }, effective: { X: "p", Y: "g" } });
    expect(
      decodeProjectEnv({ variables: { X: "p" }, global: {}, effective: { X: "from daemon" } })
        .effective,
    ).toEqual({ X: "from daemon" });
  });

  it("gives agents their env names, or none from an older daemon", () => {
    expect(decodeAgent({ id: 1, env_names: ["B", "A", 3] }).envNames).toEqual(["B", "A"]);
    expect(decodeAgent({ id: 1 }).envNames).toEqual([]);
  });

  it("checks names and values as the daemon does", () => {
    expect(nameProblem("CLAUDE_CONFIG_DIR")).toBeNull();
    expect(nameProblem("_x9")).toBeNull();
    expect(nameProblem("")).toBe("Give it a name.");
    expect(nameProblem("9LIVES")).toMatch(/can't start with a digit/);
    expect(nameProblem("MY-VAR")).toMatch(/Letters, digits and _/);
    expect(nameProblem("A B")).toMatch(/Letters, digits and _/);
    expect(nameProblem("A".repeat(128))).toBeNull();
    expect(nameProblem("A".repeat(129))).toBe("At most 128 characters.");
    expect(nameProblem("COS_HOME")).toBe("Names starting with COS_ are kept for cos itself.");
    // The rest of what cos keeps is the daemon's to refuse.
    expect(nameProblem("CLAUDECODE")).toBeNull();
    expect(valueProblem("x".repeat(32 * 1024))).toBeNull();
    expect(valueProblem("x".repeat(32 * 1024 + 1))).toBe("At most 32 KiB.");
    // Bytes, not characters.
    expect(valueProblem("é".repeat(16 * 1024 + 1))).toBe("At most 32 KiB.");
  });

  it("hints with CLAUDE_CONFIG_DIR first and names the account", () => {
    expect(sortedNames(["Z", "CLAUDE_CONFIG_DIR", "A"])).toEqual(["CLAUDE_CONFIG_DIR", "A", "Z"]);
    expect(envHint([])).toBeNull();
    expect(envHint(["A"])).toBe("env: A");
    expect(envHint(["Z", "A", "CLAUDE_CONFIG_DIR"])).toBe("env: CLAUDE_CONFIG_DIR, +2");
    expect(accountOf("~/.claude-personal")).toBe(".claude-personal");
    expect(accountOf("/Users/me/.claude-work/")).toBe(".claude-work");
    expect(accountOf("~")).toBeNull();
    expect(accountOf("")).toBeNull();
    expect(accountOf(undefined)).toBeNull();
  });
});

describe("environment in the store", () => {
  it("loads the global list and every project's on connect", async () => {
    const { cos, daemon } = await startedStore();
    const lines = requestLines(daemon);
    expect(lines).toContain("GET /v1/env");
    expect(lines).toContain("GET /v1/projects/1/env");
    expect(lines).toContain("GET /v1/projects/2/env");
    expect(cos.get().globalEnv).toEqual({ DISABLE_TELEMETRY: "1" });
    expect(projectEnvNames(cos.get(), 1)).toEqual(["CLAUDE_CONFIG_DIR", "ANTHROPIC_BASE_URL"]);
    expect(cos.get().projectEnvs[1]?.effective.DISABLE_TELEMETRY).toBe("1");
    expect(projectEnvNames(cos.get(), 2)).toEqual([]);
    expect(claudeAccount(cos.get(), 1)).toEqual({
      account: ".claude-personal",
      value: "~/.claude-personal",
      inherited: false,
    });
    expect(claudeAccount(cos.get(), 2)).toBeNull();
  });

  it("refetches one project's list on env.changed for it", async () => {
    const { cos, daemon } = await startedStore();
    daemon.env.projects[2] = { GITHUB_TOKEN: "ghp_x" };
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5000, "env.changed", { project_id: 2, name: "GITHUB_TOKEN", action: "set" }),
    });
    await settle();
    expect(requestLines(daemon)).toEqual(["GET /v1/projects/2/env"]);
    expect(cos.get().projectEnvs[2]?.variables).toEqual({ GITHUB_TOKEN: "ghp_x" });
  });

  it("refetches the global list and every project's merge on a global env.changed", async () => {
    const { cos, daemon } = await startedStore();
    daemon.env.global.CLAUDE_CONFIG_DIR = "~/.claude-work";
    daemon.requests.length = 0;
    cos.handle({
      type: "event",
      event: rawEvent(5000, "env.changed", {
        project_id: null,
        name: "CLAUDE_CONFIG_DIR",
        action: "set",
      }),
    });
    await settle();
    expect(new Set(requestLines(daemon))).toEqual(
      new Set(["GET /v1/env", "GET /v1/projects/1/env", "GET /v1/projects/2/env"]),
    );
    expect(cos.get().globalEnv?.CLAUDE_CONFIG_DIR).toBe("~/.claude-work");
    // cos keeps its own; opal-app now inherits the work account.
    expect(claudeAccount(cos.get(), 1)?.account).toBe(".claude-personal");
    expect(claudeAccount(cos.get(), 2)).toEqual({
      account: ".claude-work",
      value: "~/.claude-work",
      inherited: true,
    });
  });

  it("sets and removes variables in either scope, showing them at once", async () => {
    const { cos, daemon } = await startedStore();
    const pending = cos.setEnv(2, "CLAUDE_CONFIG_DIR", "~/.claude-work");
    await pending;
    expect(cos.get().projectEnvs[2]?.variables).toEqual({ CLAUDE_CONFIG_DIR: "~/.claude-work" });
    expect(daemon.env.projects[2]).toEqual({ CLAUDE_CONFIG_DIR: "~/.claude-work" });

    await cos.setEnv(null, "SHARED", "yes");
    expect(cos.get().globalEnv?.SHARED).toBe("yes");
    expect(cos.get().projectEnvs[1]?.effective.SHARED).toBe("yes");

    await cos.removeEnv(1, "ANTHROPIC_BASE_URL");
    expect(cos.get().projectEnvs[1]?.variables.ANTHROPIC_BASE_URL).toBeUndefined();
    await settle();
    expect(daemon.events.filter((e) => e.kind === "env.changed").map((e) => e.data)).toEqual([
      { project_id: 2, name: "CLAUDE_CONFIG_DIR", action: "set" },
      { project_id: null, name: "SHARED", action: "set" },
      { project_id: 1, name: "ANTHROPIC_BASE_URL", action: "removed" },
    ]);
  });

  it("throws the daemon's reason for a name cos keeps to itself", async () => {
    const { cos } = await startedStore();
    await expect(cos.setEnv(1, "COS_HOOK_SOCKET", "/tmp/x")).rejects.toThrow(
      "names starting with COS_ are kept for cos itself, so COS_HOOK_SOCKET can't be set",
    );
    await expect(cos.setEnv(null, "TERM", "dumb")).rejects.toThrow(
      "cos sets TERM itself for every agent, so it can't be set here",
    );
    expect(cos.get().projectEnvs[1]?.variables.COS_HOOK_SOCKET).toBeUndefined();
    expect(cos.get().globalEnv?.TERM).toBeUndefined();
  });

  it("takes the lists from the daemon's answer to PUT and DELETE", async () => {
    const { cos, daemon } = await startedStore();
    // Changed behind the store's back: the answer brings it along.
    daemon.env.projects[2] = { FROM_CLI: "x" };
    await cos.setEnv(2, "A", "1");
    expect(cos.get().projectEnvs[2]?.variables).toEqual({ FROM_CLI: "x", A: "1" });
    // Removing a name that isn't set is fine.
    await cos.removeEnv(null, "NOT_SET");
    expect(cos.get().globalEnv).toEqual({ DISABLE_TELEMETRY: "1" });
    // An empty answer (an older daemon) still shows the change.
    const original = daemon.env.route.bind(daemon.env);
    daemon.env.route = (method, path, b) => {
      const answer = original(method, path, b);
      return answer && method !== "GET" ? { ...answer, body: null } : answer;
    };
    await cos.setEnv(null, "B", "2");
    expect(cos.get().globalEnv).toEqual({ DISABLE_TELEMETRY: "1", B: "2" });
    expect(cos.get().projectEnvs[1]?.effective.B).toBe("2");
  });

  it("marks environment unsupported when the daemon has no env routes", async () => {
    const harness = fakeStore();
    harness.daemon.env.route = () => null;
    await harness.cos.start();
    await settle();
    expect(harness.cos.get().envSupported).toBe(false);
    expect(harness.cos.get().globalEnv).toBeNull();
  });

  it("counts an agent on Claude's login screen as waiting on you", async () => {
    const { cos } = await startedStore();
    const s = cos.get();
    const agent = s.agents.find((a) => a.id === 11)!;
    expect(agent.state).toBe("needs_login");
    expect(agentLabel(agent.state)).toMatchObject({ title: "Needs Claude login", attention: true });
    expect(staffLabel("needs_login").attention).toBe(true);
    expect(loginAgents(s).map((a) => a.id)).toEqual([11]);
    expect(agentNeedsYou(s, agent)).toBe(true);
    expect(waitingCount(s)).toBe(
      approvalsForYou(s).length + trustAgents(s).length + 1 + pendingDevices(s).length,
    );
    expect(configDirOf(s, agent)).toBe("~/.claude-personal");
    expect(
      configDirOf(
        s,
        s.agents.find((a) => a.id === 3)!,
      ),
    ).toBeNull();
  });

  it("says how to log a config dir in from a shell", () => {
    expect(loginCommand("~/.claude-personal")).toBe("CLAUDE_CONFIG_DIR=~/.claude-personal claude");
    expect(loginCommand(null)).toBe("claude");
    expect(loginCommand("~/My Claude")).toBe("CLAUDE_CONFIG_DIR=~/'My Claude' claude");
    expect(loginCommand("/tmp/it's")).toBe("CLAUDE_CONFIG_DIR='/tmp/it'\\''s' claude");
  });

  it("stamps agents with the names they launched with", async () => {
    const { cos } = await startedStore();
    const lisbon = cos.get().agents.find((a) => a.id === 7);
    expect(lisbon?.envNames).toEqual([
      "ANTHROPIC_BASE_URL",
      "CLAUDE_CONFIG_DIR",
      "DISABLE_TELEMETRY",
    ]);
    const chief = cos.get().agents.find((a) => a.id === 40);
    expect(chief?.envNames).toContain("CLAUDE_CONFIG_DIR");
    const opal = cos.get().agents.find((a) => a.id === 3);
    expect(opal?.envNames).toEqual(["DISABLE_TELEMETRY"]);
  });
});
