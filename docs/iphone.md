# iPhone app

The iPhone app is SwiftUI in `clients/iphone`, built on `clients/shared/CosKit`. It reaches a cos daemon on a Mac or a server over Tailscale, the same way `cos --host` and the desktop app's hosts do. How to build it and put it on your phone is in [its README](../clients/iphone/README.md).

Version 1 has **no push notifications**: it is current while it is open, and catches up when it comes back to the foreground. The plan for push is at the end, along with what the daemon needs for it.

## Hosts

- **Adding:** by Tailscale name (`my-mac`, `my-mac.tail1234.ts.net`) or address (`100.101.102.103`, `[fd7a:…]`), with an optional `:port` (default 7700). The address is read as the CLI's `--host` reads it (`CosEndpoint.parse`, mirroring `cos-bridge`'s `parse_address`), so a URL, a path or a space is refused with the same message. Once one host is saved, the add screen also lists the rest of the tailnet (below).
- **First run:** with no host saved, the add screen says "Add your first machine by its Tailscale name (for example `my-mac`) or `100.x` address. After that, cos lists the rest of your tailnet here." above the address field.
- **Saved:** names and addresses in `UserDefaults`, plus which one is current. Nothing secret: Tailscale identifies the phone, and the host approves it.
- **Hosts tab:** switch by tapping, rename or remove by swiping, reorder with Edit. "Found on your tailnet" lists machines running cos that aren't saved yet, each with Connect. It shows the current host's address, status, cos version and event count, and the setup steps.
- **Switching** stops the old host's session and starts a new one, so nothing from the previous host stays on screen. A host is followed only while the app is in the foreground (`scenePhase`).

## Your tailnet

An iPhone app can't ask Tailscale who is on the tailnet: there's no CLI and no shared API. A host can, so the app asks every saved host that answers (reachable and approved) for `GET /v1/tailnet/peers`, all at once with a 5 s limit each, and merges the lists by Tailscale node ID (`TailnetListing.merge` in `Cos/Model/Tailnet.swift`). `running` from any host wins, as does online, and the latest "last seen".

The add screen shows **On your tailnet** above the manual name/address field, with "via build-box and studio" under the title, and pull to refresh. Each row has an OS icon, the name, `os · 100.x`, and a chip:

| Chip | When | Action |
|---|---|---|
| **cos running** (green) | Online, cos answers on its port, not saved. | **Connect** saves it by its `address` under its Tailscale name and switches to it. The host there then asks you to approve the phone as usual. |
| **Online** | Online, no cos. | |
| **Offline · seen 3h ago** | Offline (the time when Tailscale knows it). | |
| **Added** | Already saved, matched by address (its IPs, MagicDNS name or `address`) or by name. | |

The phone itself (`is_you`) is never listed. Machines of other Tailscale users and tagged ones (`mine: false`) can't be approved, so they sit behind "Show N other devices". The hosts tab's **Found on your tailnet** shows just the unsaved machines running cos.

A host with an older cos answers 403 `host_only` for the peer list over the tailnet: it adds nothing, and the section says "Update cos on <host> to list your tailnet here." A host whose Tailscale isn't usable says why. Hosts that can't be reached or refuse the phone are skipped silently; their own screens say why.

## Connection states

Each state fills the screen in place of the tab's content. Once the lists have loaded, a lost connection shows a banner over what was loaded instead.

| State | Shown as |
|---|---|
| `403 device_pending` | "Approve this iPhone on <host>", the command from the daemon's message (`cos devices approve <n>`) with Copy, the daemon's own words, and "Checking again every few seconds…". The event follower retries with backoff (0.5 s doubling to 5 s), so approving it on the host lets the app in by itself. |
| `403 device_denied` | "<host> denied this iPhone", with the daemon's message on how to start over. |
| `403 wrong_user`, `unknown_peer`, `tagged_device`, `host_only` | What the code means and what to do, with the daemon's message. |
| `503 tailscale_unavailable` | Tailscale isn't usable on the host; retried. |
| Can't reach | The name doesn't resolve, the port is closed, or there's no route: the error's detail, and three checks (Tailscale on both devices, `cos status`, `cos remote status`). Retried. |

Any other error code is shown with its daemon message: `RefusalCode` is an open type.

## Screens

Two tabs: **Chiefs** (the home tab, with a badge of what waits on you) and **Hosts**. The host chip at the top left switches hosts from Chiefs. The layout follows a messaging app: one chat per project's Chief of Staff, and a project's workspaces and sessions one menu away.

- **Chiefs:** every project's Chief of Staff as a chat row, the most recently active first; chiefs that never started go last. Each row has the project's avatar (its initial on a colour picked from its name), the time of its last activity, and a preview: "2 sessions working…" while the agents it started work (as a chat app shows "typing…"), else what it last said as one plain line, or "Not started". An orange count is what waits on you in that project; an orange dot means it did something since you last opened its chat on this phone (`ReadMarks`, kept per host in `UserDefaults`; a chat never opened here isn't marked).
  - Above the rows: chips for All, Needs you and Working, search by name or preview, and a pinned **Waiting on you** row with the count and a summary ("1 approval · 1 trust dialog · 2 logins").
  - Swipe a row (or long-press it) for its workspaces and sessions, or the chief's terminal.
- **Waiting on you** (the pinned row): pending approvals with the command, the policy's reason, Deny and Allow; agents at Claude's trust dialog (Trust folder); agents on Claude's login screen (`needs_login`, with `cos agent attach <id>` to log in on the host). Each card opens its agent.
- **Chief of Staff chat:** read from its transcript (`GET /v1/agents/{id}/transcript?after=N`) over a dotted wallpaper. Your messages are bubbles on the right, its replies are markdown bubbles on the left, each with its time; tool calls are slim rows (the `cos` tools in words: "Started an agent"); a `[cos · N updates]` wakeup is an "N updates from sessions" chip that expands. A session the chief started (`mcp__cos__agent_spawn`) is a **Session started** card with its title, branch, model and live state, and Terminal and Session buttons; it finds the agent by the `agent_id` the daemon adds to the tool row, else by title. Claude's `<task-notification>` (a subagent or background command the chief ran inside its own session finished) is an **Agent finished** card with the time it took and the first lines of its result, never a raw bubble; Read the report opens the whole result as markdown in a sheet, with Copy and Reply to the chief about it. Agents it started that wait on you come last, as messages with their buttons in them: Allow and Deny for an approval, Trust folder for the trust dialog, and Open its terminal. The composer posts to `/staff/message`, which starts the chief if needed; a failed message offers Retry and Discard. It reads again on each `agent.transcript` event for its agent. An unstarted chief shows three suggestions.
  - **Ticks** on your messages, as a messaging app has them: a clock while the phone sends it; ✓ once cos has it (while the chief is mid-task it waits in cos's inbox and goes in when the chief pauses); ✓✓ once it is in the transcript (Claude has it); blue ✓✓ once Claude has started on it (its next transcript entry is Claude's own, or the chief is working right after your message). Long-press a message for Info (when each happened; the Sent time is known only for what this phone sent while the app ran) and Copy.
  - **Files:** the paperclip left of the message box offers Photos (the system picker, up to 20; HEIC and other kinds go as JPEG, PNG, JPEG, GIF and WebP as they are) and Files (iCloud Drive, On My iPhone, any file provider). Each file uploads into the chief's worktree as soon as you pick it (`POST /v1/projects/{id}/staff/attachments`) and sits above the box as a chip with a thumbnail, "Uploading…", its size or why it failed (over 32 MB, more than 20), and × to take it off. Send waits for the uploads, and a message may be files only; a failed message keeps its files for Retry. In your bubble, pictures show above the text (tap one for the whole picture) and other files as a row with their size. Pictures you picked show at once; others come from `GET /v1/agents/{id}/attachments/{name}`, fetched once per chat.
  - **Typing:** while the chief works (`working` or `starting`), the title says "typing…" and the chat ends with moving dots and its latest tool call with how long ago it started ("cargo test · 12s"), unless it has written since. Tapping it opens the chief's terminal. The chiefs list shows "typing…" too.
  - The title (avatar, name, "2 sessions working…" or its state) opens its info page. The terminal button opens the chief's terminal.
  - The **⋯ menu:** Workspaces and Sessions (each with a count, opening the info page on that tab), Identity & memory, the chief's terminal, New session (once it has started) and Stop Chief of Staff (with a confirmation; its sessions keep running and your next message starts it again).
  - **New session** (`POST /v1/projects/{id}/staff/new-session`, after a confirmation that also offers Identity & memory) starts the chief in a fresh Claude session with its identity, brief and notes; the chat starts over, since the transcript's `next` drops to 0. See "New session" in [chief-of-staff.md](chief-of-staff.md#new-session).
  - Before it starts, without a brief or identity, the empty chat has a **Give it a brief first** card that opens Identity & memory.
- **Identity & memory** (`-screen chief-memory`): from the chat's ⋯ menu, the info page's Memory button and the empty chat's card.
  - **Identity**, marked "Only you can change this" (`PUT /v1/projects/{id}/staff {identity}`): who the chief is and how it works, in an editor counting words against about 500 and bytes against 8 KiB, with Save and Revert once you change it. The chief has no tool for it.
  - **Brief** (`PUT …/staff {brief}`, at most 4 KiB, counted as you type): what it's working on now. It keeps the brief itself too, so it may rewrite yours.
  - **Memory notes** (`GET …/staff/memory` while the page shows, again on `staff.changed`): each note's name, description and when it changed, unfolding to its body. + and Add note open a sheet that checks the name (lowercase letters, digits and dashes), the one-line description (200 characters) and the body (8 KiB) as the daemon does, and says when a name replaces a note; swipe or long-press a note to Edit or Delete it (`PUT`/`DELETE …/staff/memory/{name}`), and Undo puts a deleted note back for a few seconds. At 50 notes, adding is off. A daemon older than notes says so; one older than editable briefs or identities keeps the old text, which the page reports.
  - Changes reach the chief from its next start, so once it has started the page ends with New session, which goes back to the chat.
- **Chief info:** the project's avatar, name, root and the chief's state and model; Chat, Terminal and Memory; then a segmented control:
  - **Workspaces:** each workspace, newest first, as a section: the workspace (name, branch, base, what waits on you) with its sessions under it.
  - **Sessions:** the project's agents, what needs you first. A row opens its terminal; swipe it for its details.
- **Workspace:** the branch and base, and its agents (state, role, last message).
- **Agents** (sessions, from any list): state, title, project and workspace, model, "Started by the Chief of Staff"; the approval it waits on (Allow, Deny), the trust dialog or the login hint; the last message as markdown; Open its terminal; Screen, a plain-text snapshot of its terminal (`GET /v1/agents/{id}/screen`) loaded on request; Stop; and a message box that types into the agent, resuming it first when it has exited.

- **Terminal** (full screen, for the chief and every session): the agent's live Claude Code terminal, drawn by Ghostty (libghostty, through [libghostty-spm](https://github.com/Lakr233/libghostty-spm)'s `GhosttyTerminal`, pinned to an exact version since libghostty's C API isn't stable) from `GET /v1/agents/{id}/terminal`. The stream goes into an in-memory session (`InMemoryTerminalSession`): it starts from the daemon's snapshot, follows the stream, and reconnects to a fresh snapshot. Ghostty keeps the scrollback; a drag scrolls it, with momentum. Typing goes to `POST …/keys`. Ghostty's grid always fits the view, and the PTY is resized to it (`POST …/resize`), so it reflows on rotation; the app turns to landscape for it. A resize from another client isn't mirrored; the phone takes the size back when you type.
  - Chips along the top switch between the project's chief (★) and its live sessions without leaving the screen.
  - When this agent waits on you, a bar shows the approval (Deny, Allow), the trust dialog (Trust) or the login hint; when another of the project's sessions does, it says so with Show.
  - With the keyboard down, a key bar has esc, ^C, tab, ⇧⇥, the arrows, ⏎, paste (bracketed) and a keyboard key; with it up, Ghostty's own key bar does the same.
  - The ⋯ menu: Larger and Smaller text (kept on this phone), Chat with the chief, Session details, and Stop session (with a confirmation). An exited agent offers Resume.

There's no environment editing on the phone. Markdown is rendered by blocks (headings, paragraphs, lists, quotes, code, rules) with inline styles from `AttributedString`.

The look follows the desktop app: warm neutral pages, Claude orange (#d97757) only for actions and what waits on you, SF Symbols, light and dark. Chats add a warm wallpaper (`Wall`, `WallDot`) and a light orange for your bubbles (`Outgoing`).

## Contract

The app uses CosKit's `CosService` protocol, which `CosClient` implements over HTTP/1.1, on a Unix socket or TCP. `FakeCos` implements it in memory for `-fakeData` and the tests.

| Route | Used for |
|---|---|
| `GET /v1/status`, `GET /v1/events` (SSE, `Last-Event-ID`) | Connecting, and following. Resumes after the last `seq`; `log_reset` reloads. |
| `GET /v1/projects` | Projects and workspaces. |
| `GET /v1/agents`, `GET /v1/agents/{id}`, `POST …/input`, `…/trust`, `…/stop`, `…/resume`, `GET …/screen` | Agents. |
| `GET /v1/agents/{id}/terminal` (SSE), `POST …/keys`, `POST …/resize` | The live terminal. |
| `GET /v1/agents/{id}/transcript?after=N` | The Chief of Staff chat. |
| `GET /v1/approvals?state=pending`, `POST /v1/approvals/{id}` | Approvals. |
| `GET /v1/projects/{id}/staff`, `POST …/staff/start`, `POST …/staff/message`, `GET …/staff/inbox` | The Chief of Staff. |
| `PUT /v1/projects/{id}/staff {brief?, identity?}`, `POST …/staff/new-session`, `GET …/staff/memory`, `PUT`/`DELETE …/staff/memory/{name}` | Identity & memory, and New session. |
| `POST /v1/projects/{id}/staff/attachments?name=`, `GET /v1/agents/{id}/attachments/{name}` | Files you send the chief, and pictures to show ([agents-api.md](agents-api.md#attachments)). |
| `GET /v1/tailnet/peers` | The tailnet, from every saved host (`is_you`, `self`). |

Each event refreshes only what it touches, and refreshes of one kind fold together while one is under way:

| Events | Reloads |
|---|---|
| `project.*`, `workspace.*` | projects, chiefs |
| `agent.*`, `approval.*` | agents, approvals |
| `staff.*`, agent state changes | chiefs |
| `staff.changed` | the memory notes of that project, if its page is open |
| `agent.transcript` | the chat of that agent, if open |

Decoding is lenient: unknown fields are ignored, a bad row in a list is dropped rather than failing the list, numbers sent as strings are read, and titles may be `null` (workspaces then show their city).

## Push, later

Everything that would raise a notification already goes through one seam, `Notifications` (`clients/iphone/Cos/Model/Notifications.swift`): `start()`, `notify(_:)` and `setBadge(_:)`. Version 1 uses `NoNotifications`. `HostSession` calls it when something new waits on you, never for what already waited at launch, and sets the badge to the waiting count.

**Note:** push needs a paid Apple Developer account. A free Personal Team can't sign the Push Notifications capability.

### On the phone

1. Add the Push Notifications capability and the `aps-environment` entitlement (development for Xcode builds, production for TestFlight).
2. An `APNsNotifications` in place of `NoNotifications`: `start()` asks for permission and calls `registerForRemoteNotifications()`; the app delegate gets the device token and sends it to every saved host (`PUT /v1/push/token`) whenever it changes or a host is added.
3. Categories with actions: Allow and Deny on approvals. Actions post to the same routes the cards use, through Tailscale's VPN on demand.
4. A notification's `thread-id` is the host, and tapping one opens its card (`Navigator.open`).

### On the daemon (changes wanted)

- **Device token route:** `PUT /v1/push/token` with `{"token": "<hex>", "environment": "development" | "production", "bundle_id": "…"}`, tailnet only, saved against the approved device that sent it; `DELETE` to stop. Removing or denying a device drops its token. APNs answering `410 Unregistered` drops it too.
- **APNs key:** a `.p8` auth key with its Key ID and Team ID, set on the host (for example `cos push setup --key AuthKey_ABC123.p8 --key-id ABC123 --team-id XYZ987`), kept in `~/.cos` with mode 0600 and never sent over the API. The daemon signs an ES256 JWT, refreshes it within the hour, and posts over HTTP/2 to `api.push.apple.com` or `api.sandbox.push.apple.com` by the token's environment.
- **What notifies:**
  - `approval.requested`;
  - `agent.state_changed` to `needs_trust` or `needs_login`;
  - optionally, the Chief of Staff ending a turn that answered your message (`staff.changed` to idle after `staff/message`).
- **Payload:** `alert` (title and body as the cards word them), `badge` (the count of what waits on you), `thread-id` (the host), `category` (approval, trust, login), and the ids (`approval_id`, `agent_id`) as custom keys. `apns-collapse-id` per item, so a decided approval can be replaced or withdrawn.
- **Not for every event:** at most one push per item, and none while the phone is following the event stream (the daemon knows which devices have a stream open).
