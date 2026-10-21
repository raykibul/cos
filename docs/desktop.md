# Desktop app (contract)

The Mac app is a **Tauri 2** app in `clients/desktop`. It replaces the SwiftUI app. `clients/shared/CosKit` stays for a native iPhone app later.

Two pieces are built in parallel against this document:

1. **Shell:** the Rust side (`clients/desktop/src-tauri`), the window, the tray icon, and the bridge to the daemon.
2. **UI:** the web side (`clients/desktop/src`, React and TypeScript), every screen, and the Ghostty terminal.

Change this document only together with both sides.

## Layout

```
clients/desktop/
  package.json, pnpm-lock.yaml, vite.config.ts, tsconfig.json, index.html   (UI)
  src/                                                                        (UI)
  src-tauri/                                                                  (shell)
    Cargo.toml          its own Cargo workspace root ([workspace] in it), not a member of the repo workspace
    tauri.conf.json, capabilities/, icons/, src/
crates/cos-bridge/      (shell) the daemon bridge without Tauri, a member of the repo workspace, tested in Docker
```

- **`crates/cos-bridge`:** everything that talks to the daemon (HTTP over the Unix socket, the event stream with reconnects, terminal streams). It depends on no Tauri crate, so `make test` and `make lint` cover it in Docker.
- **`src-tauri`:** a thin layer that exposes `cos-bridge` as Tauri commands and channels. It is built natively (`cargo tauri build`), like the Swift app was; the root `Cargo.toml` must not pick it up.

## Tooling

- **Node in Docker:** all `pnpm` commands run in Docker (the repo rule), from a `node:22` image with pnpm through corepack (`docker/node.Dockerfile`). `node_modules` lives in a Docker volume mounted over `clients/desktop/node_modules`, so Linux binaries never land on the Mac.
  - `make ui-install`, `make ui-test` (vitest), `make ui-lint` (eslint and prettier --check), `make ui-typecheck` (tsc --noEmit), `make ui-build` (writes `clients/desktop/dist`, which is plain static files).
  - `make ui-dev`: Vite's dev server in Docker, published on `localhost:5173`.
- **Tauri natively:** `cargo tauri dev` and `cargo tauri build` run on the Mac (`cargo install tauri-cli --version ^2`). `tauri.conf.json` has no `beforeDevCommand` or `beforeBuildCommand`. `devUrl` is `http://localhost:5173` (start `make ui-dev` first), and `frontendDist` is `../dist` (run `make ui-build` first).
  - `make desktop`: runs `make ui-build`, then `cargo tauri build --bundles app`. It prints the `.app` path.
- **Browser mode:** the UI also runs in a plain browser (`make ui-dev`, then open `localhost:5173`). With no Tauri present, it uses the fake bridge (below) with canned data, which is how the UI is developed and tested.

## The bridge (Tauri commands)

The UI never talks to the socket. It calls these commands with `invoke` from `@tauri-apps/api/core`, and streams arrive on a `Channel`. Field names are camelCase on the TypeScript side; the daemon's JSON bodies pass through unchanged, in snake_case.

| Command | Arguments | Result |
|---|---|---|
| `daemon_info` | – | `{socket: string, home: string}`: where the bridge looks for the daemon (`$COS_HOME/cos.sock`, else `~/.cos/cos.sock`) |
| `daemon_request` | `{method: "GET"\|"POST"\|"PUT"\|"DELETE", path: string, body?: any}` | `{status: number, body: any \| null}`. `path` must start with `/v1/`, else it is refused. `body` is the parsed JSON, or null for an empty response. A non-JSON response comes back as a string. Any HTTP status is a normal result; only failing to reach the daemon is an error. |
| `follow_events` | `{after: number \| null, onMessage: Channel<EventMessage>}` | `number`: a stream id |
| `attach_terminal` | `{agentId: number, onMessage: Channel<TerminalMessage>}` | `number`: a stream id |
| `close_stream` | `{id: number}` | – |

**Errors:** a rejected `invoke` carries `{kind: "not_running" | "io" | "bad_request", message: string}`. `not_running` means the socket is missing or refuses connections.

**Keys and resizes** go through `daemon_request`: `POST /v1/agents/{id}/keys {data: base64}` and `POST /v1/agents/{id}/resize {rows, cols}`. Keystrokes must stay in order, so the UI sends them one request at a time and batches what was typed meanwhile.

### EventMessage

```ts
type EventMessage =
  | { type: "connected"; status: DaemonStatus }        // GET /v1/status, sent on each (re)connect
  | { type: "event"; event: LoggedEvent }              // one log event, in order
  | { type: "log_reset" }                              // the daemon's log restarted below `after`; the UI drops its events
  | { type: "disconnected"; reason: string; notRunning: boolean };
```

- **Reconnects:** the bridge keeps the stream open until `close_stream`. On a drop it sends `disconnected`, retries with backoff (0.5 s doubling to 5 s), and resumes after the last `seq` it delivered, sending `connected` again first.
- **Log reset:** if `status.last_seq` is lower than the last `seq` delivered, it sends `log_reset` and restarts from 0.

### TerminalMessage

The daemon's terminal SSE (see [agents-api.md](agents-api.md)) passed through:

```ts
type TerminalMessage =
  | { type: "snapshot"; rows: number; cols: number; data: string }   // base64
  | { type: "output"; data: string }                                 // base64
  | { type: "resize"; rows: number; cols: number }
  | { type: "exit"; exitCode: number | null }
  | { type: "disconnected"; reason: string };
```

- **Reconnects:** after `disconnected` (not after `exit`), the bridge reconnects by itself with the same backoff. The next message is a fresh `snapshot`, so the UI resets the terminal and plays it.

## Shell: window and tray

- **Window:** one main window, 1200×780 by default, minimum 900×560. The title bar is hidden with the traffic lights inset over the sidebar (`titleBarStyle: "Overlay"`, `hiddenTitle: true`), as in Claude's and Codex's desktop apps. The UI leaves a 28 px drag region (`data-tauri-drag-region`) at the top.
- **Tray icon:** a menu bar icon whose menu has Open cos, a "N waiting on you" line, and Quit. The UI calls `set_waiting_count({count})` to update it. Closing the window hides it; the app keeps running in the tray, and clicking the Dock icon or Open cos shows it again.
- **Notifications:** `notify({title, body})` posts a macOS notification (tauri-plugin-notification). The UI calls it for a new approval or question while the window isn't focused.
- **Identity:** product name `cos`, identifier `local.cos.desktop`, version from `package.json`, ad-hoc signed. `make desktop` needs no Apple account.

| Extra command | Arguments | Result |
|---|---|---|
| `set_waiting_count` | `{count: number}` | – |
| `notify` | `{title: string, body: string}` | – |
| `reveal_path` | `{path: string}` | – Shows it in Finder. |
| `open_in_terminal` | `{path: string}` | – Opens Terminal.app (or Ghostty.app if installed) there. |

## UI

React 19, TypeScript (strict), Vite, Tailwind CSS 4. State through a small store (Zustand or React context); no Redux. Tests with Vitest and Testing Library.

- **Look:** like the Claude and Codex desktop apps. A quiet sidebar, a large calm content area, few borders, generous spacing, rounded 8–12 px cards, and light and dark themes from the system. Use a sans for the UI (Inter, bundled through `@fontsource`), a serif for big headings (Source Serif 4), and a monospace for paths and code (JetBrains Mono). Claude's warm neutrals: an off-white `#FAF9F5`-ish light background, a near-black `#1F1E1D`-ish dark one, and one accent (Claude orange `#D97757`) used sparingly. Icons from `lucide-react`.
- **Sidebar:**
  - "Waiting on you" with a count, and the event log.
  - Projects, each with its workspaces, and under each workspace its agents (yours) and runs.
  - A footer with the daemon's status and Add Project.
- **Screens.** The Swift app (removed in this change; see git history before the removal for `clients/desktop/Cos/*.swift`) is the reference for behaviour:
  - **Agent page:** the Ghostty terminal filling the page, with a header (title, state, model, Stop, Resume, Remove). A Trust banner shows for `needs_trust`, and an approval banner with Allow and Deny (and the policy `reason`) for a pending approval. A chief-routed approval shows "The chief is deciding", with Allow and Deny still offered.
  - **Workspace page:** its agents and runs, plus a "New agent" composer (a prompt box like Claude's chat input, with model and permission-mode pickers) and New Run.
  - **Run page:** the goal and state, the plan with Approve Plan, the board (tasks with state, checks, review, last failure, and Retry/Fail/Cancel), each worker's and the chief's terminal on click, open questions with answer buttons, "Message the chief", limits, the activity (the run's approvals), and the report rendered as markdown.
  - **Waiting on you:** pending approvals not routed to the chief, trust dialogs, open questions, and plans awaiting approval.
  - **Event log:** a live list of events.
  - **Project page:** its workspaces, with New Workspace, Rename and Remove.
- **Terminal:** **ghostty-web** (Ghostty's VT engine in WebAssembly, with an xterm.js-compatible API), wrapped in one `Terminal` component so swapping to `@xterm/xterm` is a one-file change.
  - Feed it `snapshot`, then `output`. Size it to the page and send `resize` (debounced 150 ms).
  - Send keystrokes, pastes (bracketed paste when the terminal asks for it) and mouse reports through `keys`.
  - Copy, paste and select all work with Cmd-C, Cmd-V and Cmd-A.
- **Fake bridge:** `src/bridge/fake.ts` implements the same interface as `src/bridge/tauri.ts` with canned projects, agents in every state, approvals, a run in each state, questions, a report, and terminal streams replaying a Claude Code screen. The app picks it when `window.__TAURI_INTERNALS__` is missing.
- **Data:** a `DaemonClient` in TypeScript wraps `daemon_request` with typed calls for every route in [daemon.md](daemon.md), [agents-api.md](agents-api.md) and [orchestration.md](orchestration.md). A store keeps projects, agents, approvals, runs and questions current from `follow_events`, refetching what each event kind changes (coalesced so a burst costs one request).
