import { describe, expect, it } from "vitest";
import {
  decodeJudge,
  decodeJudgeProject,
  keyFound,
  keySourceLabel,
  pointStats,
} from "../api/judge";
import { changesJudge } from "../domain/events";
import { fakeStore, requestLines, settle } from "../test/harness";

async function startedStore() {
  const harness = fakeStore();
  await harness.cos.start();
  await settle();
  return harness;
}

describe("jev decoding", () => {
  it("reads the overview the daemon sends", () => {
    const judge = decodeJudge({
      key: { source: "daemon_env", detail: null, probing: false },
      active: true,
      off_reason: null,
      endpoint: "https://api.typesafe.ai/v1/systemone",
      model: "jev-latest",
      projects: [{ project_id: 1, name: "cos", enabled: true }],
      points: [
        {
          point: "permission",
          decisions: 5,
          errors: 1,
          no_opinion: 2,
          would: { approve: 3, ask_you: 1 },
          answered: 4,
          would_approve: 3,
          agreed: 2,
          disagreed: 1,
          missed: 1,
          disagreements: [
            {
              id: 9,
              point: "permission",
              project_id: 1,
              subject: "approval:12",
              summary: "Bash rm -rf build",
              would: "approve",
              why: "read_only (0.97)",
              actual: "denied",
              answers: { a: 1 },
              created_ms: 1_700_000_000_000,
            },
          ],
        },
      ],
    });
    expect(judge.key).toEqual({ source: "daemon_env", detail: null, probing: false });
    expect(judge.active).toBe(true);
    expect(judge.model).toBe("jev-latest");
    expect(judge.projects).toEqual([{ projectId: 1, name: "cos", enabled: true }]);
    const permission = pointStats(judge, "permission")!;
    expect(permission).toMatchObject({
      decisions: 5,
      errors: 1,
      noOpinion: 2,
      would: { approve: 3, ask_you: 1 },
      answered: 4,
      wouldApprove: 3,
      agreed: 2,
      disagreed: 1,
      missed: 1,
    });
    expect(permission.disagreements[0]).toEqual({
      id: 9,
      point: "permission",
      projectId: 1,
      subject: "approval:12",
      summary: "Bash rm -rf build",
      would: "approve",
      why: "read_only (0.97)",
      actual: "denied",
      answers: { a: 1 },
      createdMs: 1_700_000_000_000,
    });
    expect(pointStats(judge, "wake")).toBeNull();
  });

  it("is lenient with missing and odd fields", () => {
    const judge = decodeJudge({
      key: { source: "keychain", probing: "yes" },
      projects: [{ project_id: "2", name: "opal-app" }, { name: "no id" }],
      points: [
        { point: "wake", would: { now: 2, batch: "3", none: null }, disagreements: "nope" },
        { decisions: 4 },
      ],
    });
    expect(judge.key).toEqual({ source: "keychain", detail: null, probing: false });
    expect(judge.active).toBe(false);
    expect(judge.offReason).toBeNull();
    expect(judge.projects).toEqual([{ projectId: 2, name: "opal-app", enabled: false }]);
    // The point without a name is dropped; the rest default to 0.
    expect(judge.points).toHaveLength(1);
    expect(judge.points[0]).toMatchObject({
      point: "wake",
      decisions: 0,
      would: { now: 2, batch: 3 },
      disagreements: [],
    });
    expect(decodeJudge({}).key.source).toBe("missing");
    expect(() => decodeJudge(null)).toThrow();
    expect(decodeJudgeProject({ project_id: 3, enabled: true })).toEqual({
      projectId: 3,
      name: "Project 3",
      enabled: true,
    });
  });

  it("says where the key was found, for sources a newer daemon adds too", () => {
    const found = (source: string, active: boolean) =>
      keyFound(decodeJudge({ key: { source }, active }));
    expect(found("shell", true)).toBe(true);
    expect(found("environment", true)).toBe(true);
    expect(found("missing", false)).toBe(false);
    // An unknown source counts only when the daemon says jev is on.
    expect(found("keychain", true)).toBe(true);
    expect(found("keychain", false)).toBe(false);
    expect(keySourceLabel("shell")).toBe("your login shell");
    expect(keySourceLabel("daemon_env")).toBe("~/.cos/daemon.env");
    expect(keySourceLabel("environment")).toBe("the daemon's environment");
    expect(keySourceLabel("keychain")).toBe("keychain");
  });

  it("refetches on every judge.* kind", () => {
    expect(changesJudge("judge.decided")).toBe(true);
    expect(changesJudge("judge.changed")).toBe(true);
    expect(changesJudge("judgement.x")).toBe(false);
  });
});

describe("jev in the store", () => {
  it("loads the overview on connect", async () => {
    const { cos } = await startedStore();
    const judge = cos.get().judge!;
    expect(judge.key.source).toBe("shell");
    expect(judge.active).toBe(true);
    expect(judge.projects.map((p) => [p.name, p.enabled])).toEqual([
      ["cos", true],
      ["opal-app", false],
    ]);
    expect(pointStats(judge, "permission")).toMatchObject({
      answered: 45,
      wouldApprove: 41,
      disagreed: 4,
    });
    expect(pointStats(judge, "permission")!.disagreements).toHaveLength(4);
    expect(pointStats(judge, "wake")!.would).toEqual({ now: 7, batch: 15, none: 7 });
  });

  it("switches a project with PUT and shows the answer", async () => {
    const { cos, daemon } = await startedStore();
    const answer = await cos.setProjectJudge(2, true);
    expect(answer).toEqual({ projectId: 2, name: "opal-app", enabled: true });
    expect(
      daemon.requests.find((r) => r.method === "PUT" && r.path === "/v1/projects/2/judge")?.body,
    ).toEqual({ enabled: true });
    expect(cos.get().judge!.projects.find((p) => p.projectId === 2)?.enabled).toBe(true);
    expect(daemon.judge.enabled.has(2)).toBe(true);
    await expect(cos.setProjectJudge(99, true)).rejects.toThrow("no project 99");
  });

  it("refetches the overview on judge.* events, coalesced", async () => {
    const { cos, daemon } = await startedStore();
    const before = requestLines(daemon).filter((l) => l === "GET /v1/judge").length;
    daemon.judge.points[0]!.decisions = 53;
    daemon.log("judge.decided", {
      decision_id: 92,
      point: "permission",
      project_id: 1,
      subject: "approval:13",
      would: "approve",
      latency_ms: 140,
      error: null,
    });
    daemon.log("judge.decided", { decision_id: 93, point: "wake", project_id: 1 });
    daemon.log("judge.changed", { project_id: 2, enabled: true });
    await settle();
    const after = requestLines(daemon).filter((l) => l === "GET /v1/judge").length;
    // One in flight and one more for the burst.
    expect(after - before).toBeGreaterThanOrEqual(1);
    expect(after - before).toBeLessThanOrEqual(2);
    expect(pointStats(cos.get().judge!, "permission")!.decisions).toBe(53);
  });

  it("reloads the key", async () => {
    const { cos, daemon } = await startedStore();
    daemon.judge.key = { source: "missing", detail: null, probing: false };
    cos.refreshJudge();
    await settle();
    expect(cos.get().judge!.active).toBe(false);
    expect(cos.get().judge!.offReason).toMatch(/no TYPESAFEAI_API_KEY found/);
    const judge = await cos.reloadJudge();
    expect(judge.key.source).toBe("shell");
    expect(cos.get().judge!.active).toBe(true);
    expect(requestLines(daemon)).toContain("POST /v1/judge/reload");
  });

  it("marks jev unsupported when the daemon has no judge routes", async () => {
    const harness = fakeStore();
    harness.daemon.judge.route = () => null;
    await harness.cos.start();
    await settle();
    expect(harness.cos.get().judgeSupported).toBe(false);
    expect(harness.cos.get().judge).toBeNull();
    // Other routes are unaffected.
    expect(harness.cos.get().projectsLoaded).toBe(true);
  });

  it("keeps the error when the overview fails otherwise", async () => {
    const harness = fakeStore();
    harness.daemon.judge.route = (method, path) =>
      path === "/v1/judge" && method === "GET"
        ? { status: 500, body: { error: "jev's database is locked" } }
        : null;
    await harness.cos.start();
    await settle();
    expect(harness.cos.get().judgeSupported).toBe(true);
    expect(harness.cos.get().judgeError).toBe("jev's database is locked");
    // A later event tries again.
    harness.daemon.log("judge.changed", {});
    await settle();
    expect(requestLines(harness.daemon).filter((l) => l === "GET /v1/judge").length).toBe(2);
  });
});
