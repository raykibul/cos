import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Bridge,
  DaemonInfo,
  DaemonResponse,
  EventMessage,
  HttpMethod,
  TerminalMessage,
} from "./types";

/** The real bridge: Tauri commands from src-tauri (docs/desktop.md). */
export class TauriBridge implements Bridge {
  readonly kind = "tauri" as const;

  daemonInfo(): Promise<DaemonInfo> {
    return invoke<DaemonInfo>("daemon_info");
  }

  daemonRequest(method: HttpMethod, path: string, body?: unknown): Promise<DaemonResponse> {
    return invoke<DaemonResponse>("daemon_request", { method, path, body: body ?? null });
  }

  followEvents(after: number | null, onMessage: (message: EventMessage) => void): Promise<number> {
    const channel = new Channel<EventMessage>();
    channel.onmessage = onMessage;
    return invoke<number>("follow_events", { after, onMessage: channel });
  }

  attachTerminal(agentId: number, onMessage: (message: TerminalMessage) => void): Promise<number> {
    const channel = new Channel<TerminalMessage>();
    channel.onmessage = onMessage;
    return invoke<number>("attach_terminal", { agentId, onMessage: channel });
  }

  closeStream(id: number): Promise<void> {
    return invoke("close_stream", { id });
  }

  setWaitingCount(count: number): Promise<void> {
    return invoke("set_waiting_count", { count });
  }

  notify(title: string, body: string): Promise<void> {
    return invoke("notify", { title, body });
  }

  revealPath(path: string): Promise<void> {
    return invoke("reveal_path", { path });
  }

  openInTerminal(path: string): Promise<void> {
    return invoke("open_in_terminal", { path });
  }
}
