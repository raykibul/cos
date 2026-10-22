import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource-variable/source-serif-4";
import "./styles.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { type DialogState, type Page, createUiStore } from "./app/ui";
import { createBridge, inTauri } from "./bridge";
import { followAttention } from "./store/attention";
import { CosStore } from "./store/store";

// In a plain browser the fake bridge stands in for the shell. A few query
// parameters set it up for development and screenshots:
//   ?daemon=off      start with the daemon stopped
//   ?static          no simulated activity
//   ?page=run:10     open a page (waiting, events, settings, project:1, chief:1, projectEnv:1, workspace:3, agent:7, run:10)
//   ?dialog=newRun:3 open a sheet over it (envVar:1 adds a variable to project 1, envVar:all to all
//                    projects; envVar:1:NAME edits one)
//   ?terminal        open the chief's terminal split
//   ?host=h1         start on another machine (h1 Studio waits for approval, h2 can't be reached)
//   ?firewall        this Mac's tailnet listener is blocked by the macOS firewall
//   ?tailscale=off   Tailscale is off on this Mac, so its tailnet's devices can't be listed
//   ?paused          cos's Chief of Staff starts with coordination paused, 3 updates waiting
//   ?context=92      how full cos's chief's context is, in percent (42 by default)
//   ?theme=dark      force a colour scheme
const params = new URLSearchParams(window.location.search);

function parsePage(value: string | null): Page | undefined {
  if (!value) return undefined;
  const [kind, id] = value.split(":");
  if (kind === "waiting" || kind === "events" || kind === "settings") return { kind };
  const n = Number(id);
  if (!Number.isInteger(n)) return undefined;
  if (
    kind === "project" ||
    kind === "chief" ||
    kind === "projectEnv" ||
    kind === "workspace" ||
    kind === "agent" ||
    kind === "run"
  ) {
    return { kind, id: n };
  }
  return undefined;
}

function parseDialog(value: string | null): DialogState | null {
  if (!value) return null;
  const [kind, id] = value.split(":");
  const n = Number(id);
  switch (kind) {
    case "addProject":
    case "addHost":
      return { kind };
    case "renameHost":
    case "removeHost":
      return id ? { kind, hostId: id } : null;
    case "newWorkspace":
    case "removeProject":
      return { kind, projectId: n };
    case "newAgent":
    case "newRun":
    case "renameWorkspace":
    case "removeWorkspace":
      return { kind, workspaceId: n };
    case "removeAgent":
      return { kind, agentId: n };
    case "cancelRun":
    case "limits":
      return { kind, runId: n };
    case "envVar": {
      const name = value.split(":")[2];
      return {
        kind,
        projectId: id === "all" ? null : n,
        ...(name ? { name, mode: "edit" as const } : {}),
      };
    }
    default:
      return null;
  }
}

const theme = params.get("theme");
if (theme === "dark" || theme === "light") document.documentElement.dataset.theme = theme;

async function main() {
  const tauri = inTauri();
  const bridge = await createBridge({
    running: params.get("daemon") !== "off",
    simulate: !params.has("static"),
    host: params.get("host") ?? undefined,
    firewall: params.has("firewall"),
    tailscaleOff: params.get("tailscale") === "off",
    staffPaused: params.has("paused"),
    chiefContext: params.has("context") ? Number(params.get("context")) : undefined,
  });
  const cos = new CosStore(bridge);
  const ui = createUiStore({
    page: parsePage(params.get("page")) ?? { kind: "waiting" },
    dialog: parseDialog(params.get("dialog")),
    chiefTerminal: params.has("terminal"),
  });
  followAttention(cos);
  void cos.start();
  // Handy in the browser console: drive the fake daemon by hand.
  if (!tauri) Object.assign(window, { cos, ui, bridge });

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App cos={cos} ui={ui} />
    </StrictMode>,
  );
}

void main();
