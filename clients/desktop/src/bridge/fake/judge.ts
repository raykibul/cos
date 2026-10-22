// The fake daemon's jev desk (`/v1/judge`): the judgment model cos runs in
// shadow mode. The key came from the login shell; cos is judged and opal-app
// isn't. Its permission stats show it would have auto-approved 41 of the 45
// requests you answered, disagreeing on 4, and its wake stats a chief's
// inbox's worth of verdicts.

import type { DaemonResponse } from "../types";
import type { Raw } from "./fixtures";

/** What the jev desk needs from the fake daemon. */
export interface JudgeHost {
  readonly projects: Raw[];
  log(kind: string, data: Raw): void;
}

export interface FakeJudgeKey {
  source: string;
  detail: string | null;
  probing: boolean;
}

const OFF_REASON =
  "no TYPESAFEAI_API_KEY found in the daemon's environment, ~/.cos/daemon.env or your login shell";

class JudgeError {
  constructor(
    readonly status: number,
    readonly message: string,
  ) {}
}

function disagreements(now: number): Raw[] {
  const ago = (minutes: number) => now - minutes * 60_000;
  return [
    {
      id: 91,
      point: "permission",
      project_id: 1,
      subject: "approval:12",
      summary: "Bash rm -rf build",
      would: "approve",
      why: "read_only (0.97), serves_task 0.91",
      actual: "denied",
      answers: null,
      created_ms: ago(14),
    },
    {
      id: 77,
      point: "permission",
      project_id: 1,
      subject: "approval:9",
      summary: "Bash git push --force origin lisbon",
      would: "approve",
      why: "serves_task 0.88, reversible 0.62",
      actual: "denied",
      answers: null,
      created_ms: ago(95),
    },
    {
      id: 63,
      point: "permission",
      project_id: 1,
      subject: "approval:7",
      summary: "Write ~/.zshrc",
      would: "approve",
      why: "serves_task 0.71",
      actual: "denied",
      answers: null,
      created_ms: ago(60 * 26),
    },
    {
      id: 40,
      point: "permission",
      project_id: 1,
      subject: "approval:4",
      summary: "WebFetch https://pastebin.com/raw/x8Kq",
      would: "approve",
      why: "read_only (0.93)",
      actual: "denied",
      answers: null,
      created_ms: ago(60 * 50),
    },
  ];
}

export class FakeJudge {
  key: FakeJudgeKey = { source: "shell", detail: null, probing: false };
  /** The projects jev judges. */
  readonly enabled = new Set<number>([1]);
  readonly endpoint = "https://api.typesafe.ai/v1/systemone";
  readonly model = "jev-latest";
  /** The raw stats, one per decision point, which tests may change. */
  points: Raw[];

  constructor(
    private readonly host: JudgeHost,
    now: number,
  ) {
    this.points = [
      {
        point: "permission",
        decisions: 52,
        errors: 1,
        no_opinion: 3,
        would: { approve: 43, ask_you: 6 },
        answered: 45,
        would_approve: 41,
        agreed: 39,
        disagreed: 4,
        missed: 2,
        disagreements: disagreements(now),
      },
      {
        point: "wake",
        decisions: 31,
        errors: 0,
        no_opinion: 2,
        would: { now: 7, batch: 15, none: 7 },
        answered: 0,
        would_approve: 0,
        agreed: 0,
        disagreed: 0,
        missed: 0,
        disagreements: [],
      },
    ];
  }

  get active(): boolean {
    return this.key.source !== "missing";
  }

  overview(): Raw {
    return {
      key: { ...this.key },
      active: this.active,
      off_reason: this.active ? null : OFF_REASON,
      endpoint: this.endpoint,
      model: this.model,
      projects: this.host.projects.map((p) => ({
        project_id: p.id,
        name: p.name,
        enabled: this.enabled.has(p.id as number),
      })),
      points: JSON.parse(JSON.stringify(this.points)) as Raw[],
    };
  }

  /** The answer, or null when the route isn't the desk's. */
  route(method: string, path: string, b: Raw): DaemonResponse | null {
    const project = path.match(/^\/v1\/projects\/(\d+)\/judge$/);
    if (path !== "/v1/judge" && path !== "/v1/judge/reload" && !project) return null;
    try {
      return { status: 200, body: this.handle(method, path, project, b) };
    } catch (error) {
      if (error instanceof JudgeError)
        return { status: error.status, body: { error: error.message } };
      throw error;
    }
  }

  private handle(method: string, path: string, project: RegExpMatchArray | null, b: Raw): Raw {
    if (path === "/v1/judge" && method === "GET") return this.overview();
    if (path === "/v1/judge/reload" && method === "POST") {
      // The daemon probes the login shell again; the fake finds it at once.
      this.key = { source: "shell", detail: null, probing: false };
      this.host.log("judge.changed", {});
      return this.overview();
    }
    if (project && method === "PUT") {
      const id = Number(project[1]);
      const found = this.host.projects.find((p) => p.id === id);
      if (!found) throw new JudgeError(404, `no project ${id}`);
      if (typeof b.enabled !== "boolean") throw new JudgeError(400, "enabled must be a boolean");
      if (b.enabled) this.enabled.add(id);
      else this.enabled.delete(id);
      this.host.log("judge.changed", { project_id: id, enabled: b.enabled });
      return { project_id: id, name: found.name, enabled: b.enabled };
    }
    throw new JudgeError(404, `no route for ${method} ${path}`);
  }
}
