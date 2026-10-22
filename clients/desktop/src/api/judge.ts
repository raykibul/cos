// jev, the judgment model cos runs in shadow mode (`GET /v1/judge`): where
// its API key came from, which projects are judged, and what it would have
// done at each decision point, next to what you did. Decoded leniently, like
// models.ts: points, verdicts and key sources stay open strings.

import {
  DecodeError,
  type Json,
  asObject,
  bool,
  isObject,
  list,
  num,
  optNum,
  optStr,
  reqNum,
  str,
  time,
} from "./decode";

/** Where the daemon found `TYPESAFE_API_KEY`; `missing` when it didn't. */
export type KeySource = "environment" | "daemon_env" | "shell" | "missing" | (string & {});

export interface JudgeKey {
  source: KeySource;
  /** A note on the search, like a daemon.env it wouldn't read. */
  detail: string | null;
  /** The login shell probe is still running. */
  probing: boolean;
}

export interface JudgeProject {
  projectId: number;
  name: string;
  enabled: boolean;
}

/** One decision jev made, with what you did. */
export interface JudgeExample {
  id: number;
  point: string;
  projectId: number | null;
  /** `approval:12`. */
  subject: string;
  /** `Bash rm -rf build`. */
  summary: string;
  /** `approve`, `ask_you`, `now`, … */
  would: string;
  /** `read_only (0.97), serves_task 0.91`. */
  why: string | null;
  /** `denied`, `allowed`, `terminal`, `expired`, or null before you answered. */
  actual: string | null;
  answers: unknown;
  createdMs: number;
}

/** `permission`: approvals that went to you. `wake`: items in a chief's inbox. */
export type JudgePoint = "permission" | "wake" | (string & {});

export interface JudgePointStats {
  point: JudgePoint;
  /** jev calls made. */
  decisions: number;
  /** Failed or timed out. */
  errors: number;
  /** An error, a timeout or confidence under 0.5. */
  noOpinion: number;
  /** Verdict → count: `{approve, ask_you}` or `{now, batch, none}`. */
  would: Record<string, number>;
  /** Permission requests you allowed or denied through cos. */
  answered: number;
  /** Of `answered`, how many jev would have auto-approved. */
  wouldApprove: number;
  /** jev approved and you allowed, or jev asked and you denied. */
  agreed: number;
  /** jev would have approved but you denied. */
  disagreed: number;
  /** jev would have asked but you allowed. */
  missed: number;
  /** The disagreed ones, newest first. */
  disagreements: JudgeExample[];
}

export interface JudgeOverview {
  key: JudgeKey;
  /** A key was found, so switched-on projects get judged. */
  active: boolean;
  /** Why jev is off, when it is. */
  offReason: string | null;
  endpoint: string;
  model: string;
  projects: JudgeProject[];
  points: JudgePointStats[];
}

function decodeKey(value: unknown): JudgeKey {
  const o: Json = isObject(value) ? value : {};
  return {
    source: str(o.source, "missing") || "missing",
    detail: optStr(o.detail) || null,
    probing: bool(o.probing, false),
  };
}

export function decodeJudgeProject(value: unknown): JudgeProject {
  const o = asObject(value, "judged project");
  const projectId = reqNum(o, "project_id", "judged project");
  return {
    projectId,
    name: str(o.name, `Project ${projectId}`),
    enabled: bool(o.enabled, false),
  };
}

function decodeExample(value: unknown): JudgeExample {
  const o = asObject(value, "judge example");
  return {
    id: reqNum(o, "id", "judge example"),
    point: str(o.point),
    projectId: optNum(o.project_id),
    subject: str(o.subject),
    summary: str(o.summary),
    would: str(o.would),
    why: optStr(o.why) || null,
    actual: optStr(o.actual) || null,
    answers: o.answers ?? null,
    createdMs: time(o.created_ms),
  };
}

function counts(value: unknown): Record<string, number> {
  const out: Record<string, number> = {};
  if (!isObject(value)) return out;
  for (const [key, count] of Object.entries(value)) {
    const n = optNum(count);
    if (n !== null) out[key] = n;
  }
  return out;
}

function decodePoint(value: unknown): JudgePointStats {
  const o = asObject(value, "judge point");
  // A point without a name can't be shown under one.
  if (typeof o.point !== "string" || o.point === "") {
    throw new DecodeError("judge point has no point");
  }
  return {
    point: o.point,
    decisions: num(o.decisions, 0),
    errors: num(o.errors, 0),
    noOpinion: num(o.no_opinion, 0),
    would: counts(o.would),
    answered: num(o.answered, 0),
    wouldApprove: num(o.would_approve, 0),
    agreed: num(o.agreed, 0),
    disagreed: num(o.disagreed, 0),
    missed: num(o.missed, 0),
    disagreements: list(o.disagreements, decodeExample),
  };
}

/** `GET /v1/judge` and `POST /v1/judge/reload`. */
export function decodeJudge(value: unknown): JudgeOverview {
  const o = asObject(value, "judge overview");
  return {
    key: decodeKey(o.key),
    active: bool(o.active, false),
    offReason: optStr(o.off_reason) || null,
    endpoint: str(o.endpoint),
    model: str(o.model),
    projects: list(o.projects, decodeJudgeProject),
    points: list(o.points, decodePoint),
  };
}

/** Whether a key was found, for a source a newer daemon adds too. */
export function keyFound(overview: JudgeOverview): boolean {
  const source = overview.key.source;
  if (source === "missing" || source === "") return false;
  return source in KEY_SOURCES || overview.active;
}

const KEY_SOURCES: Record<string, string> = {
  environment: "the daemon's environment",
  daemon_env: "~/.cos/daemon.env",
  shell: "your login shell",
};

/** `the daemon's environment`, `~/.cos/daemon.env`, `your login shell`, or the raw source. */
export function keySourceLabel(source: KeySource): string {
  return KEY_SOURCES[source] ?? source;
}

/** A point's stats, or null when the daemon has none for it. */
export function pointStats(overview: JudgeOverview, point: JudgePoint): JudgePointStats | null {
  return overview.points.find((p) => p.point === point) ?? null;
}
