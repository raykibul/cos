import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Bridge,
  DaemonInfo,
  DaemonResponse,
  EventMessage,
  FileResponse,
  Forward,
  ForwardList,
  ForwardMessage,
  Host,
  HostList,
  HttpMethod,
  RawTailnetPeers,
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

  daemonUpload(path: string, contentType: string, data: string): Promise<DaemonResponse> {
    return invoke<DaemonResponse>("daemon_upload", { path, contentType, data });
  }

  daemonFetchBytes(path: string): Promise<FileResponse> {
    return invoke<FileResponse>("daemon_fetch_bytes", { path });
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

  attachShell(shellId: number, onMessage: (message: TerminalMessage) => void): Promise<number> {
    const channel = new Channel<TerminalMessage>();
    channel.onmessage = onMessage;
    return invoke<number>("attach_shell", { shellId, onMessage: channel });
  }

  closeStream(id: number): Promise<void> {
    return invoke("close_stream", { id });
  }

  hostsList(): Promise<HostList> {
    return invoke<HostList>("hosts_list");
  }

  hostAdd(name: string, address: string): Promise<Host> {
    return invoke<Host>("host_add", { name, address });
  }

  hostRemove(id: string): Promise<HostList> {
    return invoke<HostList>("host_remove", { id });
  }

  hostRename(id: string, name: string): Promise<HostList> {
    return invoke<HostList>("host_rename", { id, name });
  }

  hostSelect(id: string): Promise<HostList> {
    return invoke<HostList>("host_select", { id });
  }

  tailnetPeers(port?: number): Promise<RawTailnetPeers> {
    return invoke<RawTailnetPeers>("tailnet_peers", { port: port ?? null });
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

  openUrl(url: string): Promise<void> {
    return invoke("open_url", { url });
  }

  forwardsList(): Promise<ForwardList> {
    return invoke<ForwardList>("forwards_list");
  }

  forwardAdd(remotePort: number, localPort?: number): Promise<Forward> {
    return invoke<Forward>("forward_add", { remotePort, localPort: localPort ?? null });
  }

  forwardRemove(remotePort: number): Promise<ForwardList> {
    return invoke<ForwardList>("forward_remove", { remotePort });
  }

  followForwards(onMessage: (message: ForwardMessage) => void): Promise<number> {
    const channel = new Channel<ForwardMessage>();
    channel.onmessage = onMessage;
    return invoke<number>("follow_forwards", { onMessage: channel });
  }

  forwardOpen(remotePort: number): Promise<void> {
    return invoke("forward_open", { remotePort });
  }
}
