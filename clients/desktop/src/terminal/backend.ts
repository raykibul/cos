import type { CosStore } from "../store/store";
import type { TerminalBackend } from "./session";

/** The bridge streams the terminal; the daemon client sends keys and sizes. */
export function terminalBackend(cos: CosStore): TerminalBackend {
  return {
    attach: (agentId, onMessage) => cos.bridge.attachTerminal(agentId, onMessage),
    close: (id) => cos.bridge.closeStream(id),
    sendKeys: (agentId, bytes) => cos.client.sendKeys(agentId, bytes),
    resize: (agentId, rows, cols) => cos.client.resizeAgent(agentId, rows, cols),
  };
}

/** The same for a shell: its id in place of an agent's. */
export function shellBackend(cos: CosStore): TerminalBackend {
  return {
    attach: (shellId, onMessage) => cos.bridge.attachShell(shellId, onMessage),
    close: (id) => cos.bridge.closeStream(id),
    sendKeys: (shellId, bytes) => cos.client.shellKeys(shellId, bytes),
    resize: (shellId, rows, cols) => cos.client.resizeShell(shellId, rows, cols),
  };
}
