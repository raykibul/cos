// Environment variables for agents (docs/daemon.md, "Environment"): a
// global list for every project, and each project's own, which wins. Every
// agent launched for a project gets the merge: its Chief of Staff, the
// agents it starts, workspace agents and runs. Decoded leniently, like
// models.ts: a value that isn't a string is dropped, not an error.

import { asObject, isObject } from "./decode";

/** Name → value. */
export type EnvVars = Record<string, string>;

export interface ProjectEnv {
  /** The project's own variables. */
  variables: EnvVars;
  /** The list for all projects. */
  global: EnvVars;
  /** What its agents get: `global`, with `variables` on top. */
  effective: EnvVars;
}

/**
 * `{"NAME": "value"}`. A `[{name, value}]` list is read too, in case the
 * daemon's shape drifts; numbers and booleans become strings.
 */
export function decodeVars(value: unknown): EnvVars {
  const out: EnvVars = {};
  const put = (name: unknown, v: unknown) => {
    if (typeof name !== "string" || name === "") return;
    if (typeof v === "string") out[name] = v;
    else if (typeof v === "number" || typeof v === "boolean") out[name] = String(v);
  };
  if (Array.isArray(value)) {
    for (const item of value) if (isObject(item)) put(item.name, item.value);
  } else if (isObject(value)) {
    for (const [name, v] of Object.entries(value)) put(name, v);
  }
  return out;
}

/** `GET /v1/env`: `{variables}`. */
export function decodeGlobalEnv(value: unknown): EnvVars {
  return decodeVars(asObject(value, "environment").variables);
}

/** `GET /v1/projects/{id}/env`. `effective` is worked out when it's missing. */
export function decodeProjectEnv(value: unknown): ProjectEnv {
  const o = asObject(value, "project environment");
  const variables = decodeVars(o.variables);
  const global = decodeVars(o.global);
  const effective =
    isObject(o.effective) || Array.isArray(o.effective)
      ? decodeVars(o.effective)
      : { ...global, ...variables };
  return { variables, global, effective };
}

/**
 * What `PUT` and `DELETE` answer: the global list, or the project's lists.
 * Null for an empty or unreadable body, so the caller works it out itself.
 */
export type EnvAnswer = { global: EnvVars } | { project: ProjectEnv } | null;

export function decodeEnvAnswer(projectId: number | null, body: unknown): EnvAnswer {
  if (!isObject(body) || body.variables === undefined) return null;
  return projectId === null
    ? { global: decodeGlobalEnv(body) }
    : { project: decodeProjectEnv(body) };
}

// MARK: Names and values

export const NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;
export const MAX_NAME = 128;
/** 32 KiB, counted in UTF-8 bytes as the daemon counts them. */
export const MAX_VALUE_BYTES = 32 * 1024;

/** Which Claude account an agent uses. */
export const CLAUDE_CONFIG_DIR = "CLAUDE_CONFIG_DIR";

/**
 * Why a name can't be used, or null. The daemon refuses more names cos sets
 * itself, with its own reason.
 */
export function nameProblem(name: string): string | null {
  if (name === "") return "Give it a name.";
  if (name.length > MAX_NAME) return `At most ${MAX_NAME} characters.`;
  if (/^[0-9]/.test(name)) return "A name can't start with a digit.";
  if (!NAME_PATTERN.test(name)) return "Letters, digits and _ only, as in CLAUDE_CONFIG_DIR.";
  // The daemon refuses these too, with others it keeps (TERM, CLAUDECODE, …).
  if (name.startsWith("COS_")) return "Names starting with COS_ are kept for cos itself.";
  return null;
}

export function valueProblem(value: string): string | null {
  if (new TextEncoder().encode(value).length > MAX_VALUE_BYTES) return "At most 32 KiB.";
  return null;
}

/** Names sorted for lists, with CLAUDE_CONFIG_DIR first: it picks the account. */
export function sortedNames(names: Iterable<string>): string[] {
  return [...names].sort((a, b) => {
    if (a === b) return 0;
    if (a === CLAUDE_CONFIG_DIR) return -1;
    if (b === CLAUDE_CONFIG_DIR) return 1;
    return a.localeCompare(b);
  });
}

/** `env: CLAUDE_CONFIG_DIR, +2`, or null with no names. */
export function envHint(names: Iterable<string>): string | null {
  const sorted = sortedNames(names);
  if (sorted.length === 0) return null;
  const rest = sorted.length - 1;
  return `env: ${sorted[0]}${rest > 0 ? `, +${rest}` : ""}`;
}

/** The account a CLAUDE_CONFIG_DIR value names: its last path segment (`~/.claude-personal` → `.claude-personal`). */
export function accountOf(configDir: string | undefined): string | null {
  if (!configDir) return null;
  const parts = configDir.trim().split("/").filter(Boolean);
  const last = parts[parts.length - 1];
  return last && last !== "~" ? last : null;
}
