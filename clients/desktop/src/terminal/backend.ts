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
