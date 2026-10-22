// The fake daemon's environment variables for agents: a global list and
// each project's own (docs/daemon.md, "Environment"). cos sets
// CLAUDE_CONFIG_DIR and a proxy URL, so its agents use the personal Claude
// account; every project gets DISABLE_TELEMETRY. Names cos uses itself are
// refused with a reason, as the daemon refuses them.

import type { DaemonResponse } from "../types";
import type { FakeAgent, Raw } from "./fixtures";

/** What the env desk needs from the fake daemon. */
export interface EnvHost {
  readonly projects: Raw[];
  readonly workspaces: Raw[];
  readonly staffDesk: { readonly staffs: Raw[] };
  log(kind: string, data: Raw): void;
}

export interface EnvFixtures {
  global: Record<string, string>;
  projects: Record<number, Record<string, string>>;
}

export function makeEnv(): EnvFixtures {
  return {
    global: { DISABLE_TELEMETRY: "1" },
    projects: {
      1: {
        CLAUDE_CONFIG_DIR: "~/.claude-personal",
        ANTHROPIC_BASE_URL: "https://llm-gateway.example.dev/anthropic",
      },
    },
  };
}

class EnvError {
  constructor(
    readonly status: number,
    readonly message: string,
  ) {}
}

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const encoder = new TextEncoder();

/** Why cos keeps a name to itself, or null. */
function reserved(name: string): string | null {
  if (name === "COS_HOOK_SOCKET")
    return "COS_HOOK_SOCKET is cos's own: agents use it to reach cos's hooks, so it can't be set.";
  if (name.startsWith("COS_"))
    return `${name} is cos's own: names starting with COS_ can't be set.`;
  if (name === "TERM" || name === "COLORTERM")
    return `${name} is cos's own: it describes the terminal cos runs agents in, so it can't be set.`;
  return null;
}

export class FakeEnv {
  readonly global: Record<string, string>;
  readonly projects: Record<number, Record<string, string>>;

  constructor(
    private readonly host: EnvHost,
    fixtures: EnvFixtures,
  ) {
    this.global = fixtures.global;
    this.projects = fixtures.projects;
  }

  /** What a project's agents get. */
  effective(projectId: number | null): Record<string, string> {
    if (projectId === null) return { ...this.global };
    return { ...this.global, ...(this.projects[projectId] ?? {}) };
  }

  /** The project an agent runs for: its workspace's, or the one whose chief it is. */
  private projectOf(agent: FakeAgent): number | null {
    if (agent.workspace_id !== null) {
      const ws = this.host.workspaces.find((w) => w.id === agent.workspace_id);
      if (ws) return ws.project_id as number;
    }
    const staff = this.host.staffDesk.staffs.find((s) => s.agent_id === agent.id);
    return staff ? (staff.project_id as number) : null;
  }

  /** Records the names an agent was launched with, once, or again on a resume. */
  stamp(agent: FakeAgent, relaunch = false): FakeAgent {
    if (relaunch || !Array.isArray(agent.env_names)) {
      agent.env_names = Object.keys(this.effective(this.projectOf(agent))).sort();
    }
    return agent;
  }

  /** The answer, or null when the route isn't the desk's. */
  route(method: string, path: string, b: Raw): DaemonResponse | null {
    const global = path.match(/^\/v1\/env(?:\/([^/]+))?$/);
    const project = path.match(/^\/v1\/projects\/(\d+)\/env(?:\/([^/]+))?$/);
    if (!global && !project) return null;
    const projectId = project ? Number(project[1]) : null;
    const name = global ? global[1] : project?.[2];
    try {
      return this.handle(method, projectId, name ? decodeURIComponent(name) : null, b);
    } catch (error) {
      if (error instanceof EnvError)
        return { status: error.status, body: { error: error.message } };
      throw error;
    }
  }

  private handle(
    method: string,
    projectId: number | null,
    name: string | null,
    b: Raw,
  ): DaemonResponse {
    if (projectId !== null && !this.host.projects.some((p) => p.id === projectId)) {
      throw new EnvError(404, `no project ${projectId}`);
    }
    if (name === null) {
      if (method !== "GET") throw new EnvError(404, `no route for ${method}`);
      if (projectId === null) return { status: 200, body: { variables: { ...this.global } } };
      return {
        status: 200,
        body: {
          variables: { ...(this.projects[projectId] ?? {}) },
          global: { ...this.global },
          effective: this.effective(projectId),
        },
      };
    }
    const list =
      projectId === null
        ? this.global
        : (this.projects[projectId] ??= {} as Record<string, string>);
    if (method === "PUT") {
      if (!NAME.test(name) || name.length > 128)
        throw new EnvError(400, `${name} isn't a valid name: letters, digits and _, at most 128`);
      const why = reserved(name);
      if (why) throw new EnvError(400, why);
      if (typeof b.value !== "string") throw new EnvError(400, "value must be a string");
      if (encoder.encode(b.value).length > 32 * 1024)
        throw new EnvError(400, "value is over 32 KiB");
      list[name] = b.value;
      this.host.log("env.changed", { project_id: projectId, name, action: "set" });
      return { status: 200, body: null };
    }
    if (method === "DELETE") {
      if (name in list) {
        delete list[name];
        this.host.log("env.changed", { project_id: projectId, name, action: "removed" });
      }
      return { status: 200, body: null };
    }
    throw new EnvError(404, `no route for ${method}`);
  }
}
