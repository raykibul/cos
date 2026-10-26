// The fake daemon's shell terminals (docs/agents-api.md, "Shells"): a
// prompt that echoes what you type and runs nothing. `pwd` prints the
// folder and `exit` ends the shell. They open over the tailnet too, as on
// the daemon; `hostOnly` plays an older daemon that refuses them there.

import { toBase64 } from "../../api/client";
import type { DaemonResponse, TerminalMessage } from "../types";
import type { Raw } from "./fixtures";

const encoder = new TextEncoder();
const b64 = (text: string) => toBase64(encoder.encode(text));

interface FakeShell {
  id: number;
  projectId: number | null;
  cwd: string;
  startedMs: number;
  exitCode: number | null;
  exited: boolean;
  /** What's on the screen, to send as the snapshot. */
  screen: string;
  /** The line being typed. */
  line: string;
}

interface ShellStream {
  shellId: number;
  onMessage: (message: TerminalMessage) => void;
}

export class FakeShells {
  readonly shells: FakeShell[] = [];
  private readonly streams = new Map<number, ShellStream>();
  private nextId = 1;
  /** Answer every route 403 `host_only`, as an older cos does over the tailnet. */
  hostOnly = false;

  constructor(private readonly now: () => number) {}

  route(method: string, path: string, b: Raw): DaemonResponse | undefined {
    if (!path.startsWith("/v1/shells")) return undefined;
    if (this.hostOnly) {
      return {
        status: 403,
        body: { error: "terminals in a folder open only on the host itself", code: "host_only" },
      };
    }
    const ok = (body: unknown, status = 200): DaemonResponse => ({ status, body });
    const missing = { status: 404, body: { error: "there is no such terminal" } };
    if (method === "GET" && path === "/v1/shells") {
      return ok({ shells: this.shells.map((s) => this.json(s)) });
    }
    if (method === "POST" && path === "/v1/shells") {
      const cwd = String(b.cwd ?? "");
      if (!cwd.startsWith("/"))
        return { status: 400, body: { error: "`cwd` must be an absolute path" } };
      const shell: FakeShell = {
        id: this.nextId++,
        projectId: typeof b.project_id === "number" ? b.project_id : null,
        cwd,
        startedMs: this.now(),
        exitCode: null,
        exited: false,
        screen: "",
        line: "",
      };
      shell.screen = `Last login: Sat Oct 24 18:02:11 on ttys004\r\n${prompt(shell)}`;
      this.shells.push(shell);
      return ok(this.json(shell), 201);
    }
    const match = path.match(/^\/v1\/shells\/(\d+)(\/keys|\/resize)?$/);
    if (!match) return undefined;
    const shell = this.shells.find((s) => s.id === Number(match[1]));
    if (!shell) return missing;
    if (method === "DELETE" && !match[2]) {
      this.shells.splice(this.shells.indexOf(shell), 1);
      this.end(shell, null);
      return { status: 204, body: null };
    }
    if (method === "POST" && match[2] === "/resize") return { status: 204, body: null };
    if (method === "POST" && match[2] === "/keys") {
      const bytes = Uint8Array.from(atob(String(b.data ?? "")), (c) => c.charCodeAt(0));
      this.typed(shell, new TextDecoder().decode(bytes));
      return { status: 204, body: null };
    }
    return undefined;
  }

  attach(shellId: number, onMessage: (message: TerminalMessage) => void, id: number): void {
    const shell = this.shells.find((s) => s.id === shellId);
    if (!shell) {
      onMessage({ type: "disconnected", reason: "there is no such terminal", code: null });
      return;
    }
    onMessage({ type: "snapshot", rows: 30, cols: 100, data: b64(shell.screen) });
    if (shell.exited) {
      onMessage({ type: "exit", exitCode: shell.exitCode });
      return;
    }
    this.streams.set(id, { shellId, onMessage });
  }

  detach(id: number): void {
    this.streams.delete(id);
  }

  private typed(shell: FakeShell, text: string): void {
    let out = "";
    for (const ch of text) {
      if (ch === "\r" || ch === "\n") {
        const command = shell.line.trim();
        shell.line = "";
        out += "\r\n";
        if (command === "exit") {
          this.write(shell, out + "logout\r\n");
          this.end(shell, 0);
          return;
        }
        if (command === "pwd") out += `${shell.cwd}\r\n`;
        else if (command) out += `zsh: this is cos's pretend shell; it ran nothing\r\n`;
        out += prompt(shell);
      } else if (ch === "\u007f") {
        if (shell.line) {
          shell.line = shell.line.slice(0, -1);
          out += "\b \b";
        }
      } else if (ch >= " ") {
        shell.line += ch;
        out += ch;
      }
    }
    this.write(shell, out);
  }

  private write(shell: FakeShell, text: string): void {
    if (!text) return;
    shell.screen += text;
    for (const stream of this.streams.values()) {
      if (stream.shellId === shell.id) stream.onMessage({ type: "output", data: b64(text) });
    }
  }

  private end(shell: FakeShell, code: number | null): void {
    shell.exited = true;
    shell.exitCode = code;
    for (const [id, stream] of this.streams) {
      if (stream.shellId !== shell.id) continue;
      stream.onMessage({ type: "exit", exitCode: code });
      this.streams.delete(id);
    }
  }

  private json(shell: FakeShell): Raw {
    return {
      id: shell.id,
      project_id: shell.projectId,
      cwd: shell.cwd,
      pid: 40000 + shell.id,
      started_ms: shell.startedMs,
      exited: shell.exited,
      exit_code: shell.exitCode,
    };
  }
}

function prompt(shell: FakeShell): string {
  const folder = shell.cwd.split("/").filter(Boolean).pop() ?? "/";
  return `\u001b[1mme@mac\u001b[0m ${folder} % `;
}
