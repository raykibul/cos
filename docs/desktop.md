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
| `daemon_info` | – | `{socket: string, home: string, host: Host}`: where the bridge looks for this Mac's daemon (`$COS_HOME/cos.sock`, else `~/.cos/cos.sock`), and the host in use |
| `daemon_request` | `{method: "GET"\|"POST"\|"PUT"\|"PATCH"\|"DELETE", path: string, body?: any}` (`PATCH` renames workspaces) | `{status: number, body: any \| null}`. `path` must start with `/v1/`, else it is refused. `body` is the parsed JSON, or null for an empty response. A non-JSON response comes back as a string. Any HTTP status is a normal result; only failing to reach the daemon is an error. |
| `daemon_upload` | `{path: string, contentType: string, data: string}` | `{status, body}`, as `daemon_request`. Sends `data` (base64) as the request body with that `Content-Type`, for attachment uploads (`POST …/attachments?name=`, see [agents-api.md](agents-api.md#attachments)). `path` must start with `/v1/`. |
| `daemon_fetch_bytes` | `{path: string}` | `{status: number, contentType: string, data: string}`: a `GET` whose body comes back as base64, for attachments to show. |
| `follow_events` | `{after: number \| null, onMessage: Channel<EventMessage>}` | `number`: a stream id |
| `attach_terminal` | `{agentId: number, onMessage: Channel<TerminalMessage>}` | `number`: a stream id |
| `close_stream` | `{id: number}` | – |
| `hosts_list` | – | `HostList`: `{hosts: Host[], current: string}`, where `Host` is `{id: string, name: string, address: string \| null}`. This Mac comes first, with id `"local"` and no address. |
| `host_add` | `{name: string, address: string}` | `Host`. It saves the host but doesn't switch to it. The address is `name`, `name:port`, `100.x.y.z:port` or an IPv6 address, read as `cos --host` reads it; the port defaults to 7700. A malformed address, a name over 60 characters, a name another host has (case aside) or an address already saved is `bad_request`. |
| `host_rename` | `{id: string, name: string}` | `HostList`. The name is trimmed and must be non-empty, at most 60 characters and unlike every other host's, This Mac's included (case aside), else `bad_request` (`"Studio" is already the name of another machine`). This Mac can be renamed; its id stays `"local"`. |
| `host_remove` | `{id: string}` | `HostList`. Removing the host in use switches to This Mac, as `host_select` does. This Mac can't be removed. |
| `host_select` | `{id: string}` | `HostList`. It closes every stream and points every later command at that host. The UI then reloads its state and follows the new host's events. |
| `tailnet_peers` | `{port?: number}` | The body of `GET /v1/tailnet/peers[?port=N]` (`{peers, self_name, reason}`, see "Remote access" in [daemon.md](daemon.md)), unchanged. It always asks **This Mac's** socket, whichever host is in use, because the route answers on the socket only. `not_running` when This Mac's daemon is down; a non-2xx answer is `io` with the daemon's message (and its `code`), and a 404 says the daemon is too old. |

**Errors:** a rejected `invoke` carries `{kind: "not_running" | "io" | "bad_request", message: string, code?: string}`. `not_running` means the socket is missing or refuses connections or, for another machine, that nothing answers at its address (the connect fails or takes over 5 s); the message names the address.

**Hosts:** saved as `hosts.json` in the app's config folder (`~/Library/Application Support/local.cos.desktop/`), with the host picked last, which the app starts on. Another machine is reached over Tailscale, as plain HTTP on TCP. Its daemon may refuse this Mac with a 403 and a `code` (see "Remote access" in [daemon.md](daemon.md)). For `daemon_request` that is a normal response; a stream turns it into `disconnected` with that `code`.

**Keys and resizes** go through `daemon_request`: `POST /v1/agents/{id}/keys {data: base64}` and `POST /v1/agents/{id}/resize {rows, cols}`. Keystrokes must stay in order, so the UI sends them one request at a time and batches what was typed meanwhile.

### EventMessage

```ts
type EventMessage =
  | { type: "connected"; status: DaemonStatus }        // GET /v1/status, sent on each (re)connect
  | { type: "event"; event: LoggedEvent }              // one log event, in order
  | { type: "log_reset" }                              // the daemon's log restarted below `after`; the UI drops its events
  | { type: "disconnected"; reason: string; notRunning: boolean; code: string | null };
```

- **Reconnects:** the bridge keeps the stream open until `close_stream`. On a drop it sends `disconnected`, retries with backoff (0.5 s doubling to 5 s), and resumes after the last `seq` it delivered, sending `connected` again first.
- **Refusals:** when the daemon refuses this Mac, `code` says why (`device_pending`, `device_denied`, `wrong_user`, …) and the bridge waits the longest delay, 5 s, before asking again. An approval on the other Mac lets the app in within seconds.
- **Log reset:** if `status.last_seq` is lower than the last `seq` delivered, it sends `log_reset` and restarts from 0.

### TerminalMessage

The daemon's terminal SSE (see [agents-api.md](agents-api.md)) passed through:

```ts
type TerminalMessage =
  | { type: "snapshot"; rows: number; cols: number; data: string }   // base64
  | { type: "output"; data: string }                                 // base64
  | { type: "resize"; rows: number; cols: number }
  | { type: "exit"; exitCode: number | null }
  | { type: "disconnected"; reason: string; code: string | null };
```

- **Reconnects:** after `disconnected` (not after `exit`), the bridge reconnects by itself with the same backoff, waiting 5 s after a refusal. The next message is a fresh `snapshot`, so the UI resets the terminal and plays it.

## Shell: window and tray

- **Window:** one main window, 1200×780 by default, minimum 900×560. The title bar is hidden with the traffic lights inset over the sidebar (`titleBarStyle: "Overlay"`, `hiddenTitle: true`), as in Claude's and Codex's desktop apps. The UI leaves a 28 px drag region (`data-tauri-drag-region`) at the top: the sidebar's top and each page's 52 px header bar are drag regions, so the window drags from anywhere up there except its buttons.
- **Tray icon:** a menu bar icon whose menu has Open cos, a "N waiting on you" line, and Quit. The UI calls `set_waiting_count({count})` to update it. Closing the window hides it; the app keeps running in the tray, and clicking the Dock icon or Open cos shows it again.
- **Notifications:** `notify({title, body})` posts a macOS notification (tauri-plugin-notification). The UI calls it for a new approval while the window isn't focused.
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
- **Layout:** a project rail on the left, then the sidebar, then the content: a row of tabs on top, the page, and a status bar at the bottom.
  - **Project rail** (72 px, `nav "Projects"`): a tile per project, in the daemon's order, with its first letter, a tint, and ⌘1 to ⌘9 under the first nine. A tile opens that project's chief; a ring marks the project the sidebar shows, a bar on its left marks its chief's page, and a badge counts the agents waiting on you there or a dot says something is working. Its tooltip has the root and the env hint. Below: Add project, then "Waiting on you" with its count, and Settings.
  - **Sidebar** (248 px): the host picker on top (the host's avatar letter and name, and a menu with This Mac, the saved machines, the cos machines on the tailnet, "Rename <host in use>…", "Add a machine…" and Settings; a host is a "machine" everywhere in the copy, This Mac keeps its name). Then the project the rail picked: its name in the serif, its root, a ⋯ menu (New workspace, All workspaces, Environment, the folder actions, Remove), a "★ Chief of Staff" row with the chief's state and how many of its agents work, and its workspaces, newest first, each with your agents under it. The sidebar follows the page: a workspace or agent page shows its project; the waiting page, Settings and the event log keep the last one. The footer is the host in use with its daemon's status, which opens Settings.
  - **Tabs:** every page opens in a tab, right of the one you're on; a page that already has one selects it. A tab has the chief's ★, a state dot for agents, or the page's icon, and a title (the project for a chief, the agent's title). × or a middle click closes it, and the neighbour on the right takes over (else the left one). Closing the last opens Waiting on you. At most 12: past that, the oldest you aren't on goes. Removing a project, workspace or agent closes its tabs.
  - **⌘K switcher:** a dialog with one box. Before you type it lists the chiefs (with their shortcut and what waits), up to five things waiting on you, your live agents and "Go to" (Waiting on you, Event log, Settings); typing matches every word against titles, details and sections, adds workspaces, and puts titles that start with the query first. ↑↓ move, Return opens. On a chief, Tab turns the box into "Message the chief of <project>": Return sends it (`POST /v1/projects/{id}/staff/message`) and closes, without leaving the page; Escape goes back, a second one closes.
  - **Focus:** opening a chief or a workspace (a click, a tab, ⌘K, ⌘1–9) puts the cursor in its box: the chief's message box, or the workspace's New agent prompt (without scrolling to it). Opening the page that's already showing does it again. An agent's page focuses its terminal.
  - **Status bar:** how many agents work, "N waiting on you" (opens it), and the shortcuts.
  - **Shortcuts** (capture phase, so they work while the terminal has focus; not while a dialog is open): ⌘K the switcher, ⌘1–9 a project's chief (by key code, so any layout), ⌘W close the tab, ⌃Tab and ⌃⇧Tab or ⌘⇧] and ⌘⇧[ the next and previous tab. ⌃W stays the terminal's.
  - **The event log** isn't in the nav: it opens from Settings ("Open the event log") and the switcher.
- **Screens.** The Swift app (removed in this change; see git history before the removal for `clients/desktop/Cos/*.swift`) is the reference for behaviour:
  - **Chief of Staff page:** the chief's conversation (`/v1/projects/{id}/staff` and its transcript) with a composer, the "An agent is waiting on you" strip, and a header with the chief's state, a context meter, the model menu, the terminal toggle and a ⋯ menu.
    - **Context meter:** a small ring and "42% context" from the transcript's `context` (`used_tokens` of `window_tokens`); its tooltip has the exact numbers ("84,123 of 200,000 tokens in context"). Without a window it reads "84K context" with no ring. The ring turns amber from 70% and accent from 90%. Nothing shows before Claude has answered. When the chief's column is narrow, only the ring stays.
    - **Pause coordination:** the ⋯ menu has Pause coordination or Resume coordination (`PUT /v1/projects/{id}/staff {paused}`). While paused, a strip under the waiting strip says "Coordination paused · 3 updates waiting · your messages still go through" (`updates_waiting`), with Resume. `staff.changed` and `staff.inbox` refetch the project's Staff, so the count stays current.
    - **Terminal split:** with the terminal open, a divider (`role="separator"`) between the conversation and the terminal resizes it by dragging (pointer capture) or with the arrow keys, keeping each side at least 360 px. The width is saved in `localStorage` (`cos.chief.terminal-width`, ignored when storage throws) and a double-click goes back to the default, 46% up to 760 px. A drag writes the width straight to the DOM, so React doesn't re-render on each move.
  - **Attachments:** the chief's composer and a workspace's New agent composer take files: the paperclip opens the file picker, and files can be dropped on the composer or pasted (a pasted screenshot is `pasted-1.png`). Each uploads at once (`daemon_upload` into the chief's worktree or the workspace) and shows as a chip above the text, with a thumbnail for a picture, "Uploading…", its size, or why it failed, and × to take it off. Send waits until every file is up, and a message may be files only. At most 20 files, 32 MB each, refused before uploading. In the conversation, your message shows its files above the bubble: pictures fetched with `daemon_fetch_bytes` (cached as `data:` URLs), other files as chips; a message not in the transcript yet shows the local previews.
  - **Agent page:** the Ghostty terminal filling the page, with a header (title, state, model, Stop, Resume, Remove). A Trust banner shows for `needs_trust`, and an approval banner with Allow and Deny (and the policy `reason`) for a pending approval.
  - **Workspace page:** its agents, plus a "New agent" composer (a prompt box like Claude's chat input, with model and permission-mode pickers).
  - **Waiting on you:** pending approvals, trust dialogs, Claude logins, and, on This Mac, devices asking to reach it.
  - **Event log:** a live list of events, opened from Settings or ⌘K.
  - **Project page:** its workspaces, with New Workspace, Rename and Remove.
  - **Another machine refusing or unreachable:** a full page in place of the content. A refusal explains its `code`, shows the command from the message (`cos devices approve 4`), and keeps retrying where waiting may help (`device_pending`, `device_denied`, `tailscale_unavailable`). "Can't reach <host>" shows the address and what to check. Both offer Try now and Switch to This Mac.
  - **Environment:** the variables cos sets for agents (`/v1/env` and `/v1/projects/{id}/env`, see "Environment" in [daemon.md](daemon.md)). A project's page opens from its ⋯ menu in the sidebar and on the chief's page. It lists the project's own variables, with mono names, values masked until Show, Edit, and × with Undo, then a muted "Inherited from all projects" list of the global ones it doesn't override, each with Override. Add and Edit open a small sheet: the name is checked here (`^[A-Za-z_][A-Za-z0-9_]*$`, at most 128, not starting with `COS_`), fixed when editing; the value is a password field with a show toggle, at most 32 KiB. Another name the daemon keeps (`TERM`, `CLAUDECODE`, …) shows its reason in the sheet. `PUT` and `DELETE` answer with the scope's lists, which the store takes as they come. With nothing set, a chip offers `CLAUDE_CONFIG_DIR`, the Claude account. Changes apply to agents launched or resumed after; running ones keep theirs. A value is never in the page's markup until Show: hidden ones are drawn as dots, and the sheet's value field is uncontrolled, so React doesn't mirror it into a `value` attribute.
  - **Where env shows:** the chief's header has `env: CLAUDE_CONFIG_DIR, +2` (the project's own names; it opens the page) and, when `CLAUDE_CONFIG_DIR` is set for the project or all projects, its last path segment (`.claude-personal`) as a small chip. The project's rail tile has the same hint in its tooltip. An agent's tab and page show its `env_names` (what it launched with), names only.
  - **Claude login (`needs_login`):** an agent whose `CLAUDE_CONFIG_DIR` nobody has logged into sits on Claude's login screen. It is labelled "Needs Claude login", counts in Waiting on you (under "Claude logins") and in the chief's waiting strip, and shows "Open its terminal and log in, or run `CLAUDE_CONFIG_DIR=<dir> claude` once" with Open terminal; on the agent's page and tab the terminal is right below the banner.
  - **Settings:** the machines, with Add, Switch, Rename and Remove. Rename (also in the host picker) is a small sheet with the name selected: at most 60 characters, checked as you type, and the shell's refusal of a taken name shown in the sheet. Environment, "All projects": the same list for the global variables. On This Mac, Remote access: on or off and the port (`GET`/`PUT /v1/tailnet`), the addresses, the `reason` when it can't listen and, for the macOS firewall, the command that allows cos; then the devices from `/v1/devices` with Approve, Deny and Remove. On another machine it says that's managed there. Judgment (jev): where the daemon found jev's key (or why jev is off, with Check again), per-project switches, and how jev's would-haves compare with your answers, with the disagreements (`GET /v1/judge`, `PUT /v1/projects/{id}/judge`, `POST /v1/judge/reload`; hidden on a daemon without them; see [judge.md](judge.md)). Event log: a button that opens it.
- **Terminal:** **ghostty-web** (Ghostty's VT engine in WebAssembly, with an xterm.js-compatible API), wrapped in one `Terminal` component so swapping to `@xterm/xterm` is a one-file change.
  - Feed it `snapshot`, then `output`. Size it to the page and send `resize` (debounced 150 ms).
  - Send keystrokes, pastes (bracketed paste when the terminal asks for it) and mouse reports through `keys`.
  - Copy, paste and select all work with Cmd-C, Cmd-V and Cmd-A.
- **Fake bridge:** `src/bridge/fake.ts` implements the same interface as `src/bridge/tauri.ts` with canned projects, agents in every state, approvals, and terminal streams replaying a Claude Code screen. It also has two other machines: Studio refuses with `device_pending` until it is approved (by itself 12 s after the first try, unless `?static`), and Travel laptop can't be reached. Adding and renaming check names as the shell does. The chiefs' transcripts report 42% of a 200,000-token context (`?context=92` sets another share), each reply adds to it, and `?paused` starts them paused with 3 updates waiting. This Mac has a tailnet listener (`?firewall` blocks it) and three devices. Environment: `DISABLE_TELEMETRY=1` for all projects, and the cos project sets `CLAUDE_CONFIG_DIR=~/.claude-personal` and `ANTHROPIC_BASE_URL`; opal-app sets nothing. It refuses the names the daemon keeps (`COS_*`, `TERM`, `COLORTERM`, Claude's session markers) with the daemon's reasons, answers `PUT` and `DELETE` with the lists, logs `env.changed`, and stamps agents' `env_names` at launch. Agent 11, in porto, sits on Claude's login screen (`needs_login`); Return in its terminal logs it in. Uploads are kept in memory with the daemon's naming, and the cos chief's first ask carries a screenshot and a log. The app picks it when `window.__TAURI_INTERNALS__` is missing.
- **Data:** a `DaemonClient` in TypeScript wraps `daemon_request` with typed calls for every route in [daemon.md](daemon.md) and [agents-api.md](agents-api.md). A store keeps projects, agents and approvals current from `follow_events`, refetching what each event kind changes (coalesced so a burst costs one request). It loads the global environment and every project's on connect; `env.changed` refetches that project's list, or, with a null `project_id`, the global list and every project's merge. Values stay in the store, never in the event log, which carries names only.
