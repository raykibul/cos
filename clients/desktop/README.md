# cos for Mac

A menu bar app for the cos daemon. The menu bar panel shows its status, what's waiting on you (permission requests and trust dialogs, answerable right there) and the latest events. The window's sidebar has the event log, "Waiting on you", your projects (registered git repos) with their workspaces (worktrees cos owns) and each workspace's agents. You can add and remove projects and create, rename and remove workspaces. The app doesn't start the daemon itself yet.

- **Agents:** a workspace's page lists its agents (state, role, model, last message) and starts new ones from a prompt, with an optional title and model. Selecting an agent shows its live terminal: Claude's own UI, which the daemon runs in a PTY, drawn with [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm). You can type into it and copy and paste, and it resizes the agent to fit the window. The header has Stop, Resume and Remove. A banner offers Trust when Claude asks to trust the folder, and Allow or Deny when it asks permission for a tool; answering in the terminal works too.

The agents API is in [docs/agents-api.md](../../docs/agents-api.md). Against a daemon without it, the workspace page says so and everything else works as before.

- `Cos/`: the SwiftUI app (macOS 14+). A menu bar item and one window, no Dock icon (`LSUIElement`).
- `../shared/CosKit/`: the Swift package the app is built on, shared with the iPhone app later. Models, an HTTP/1.1 client for the daemon's Unix socket (plain POSIX sockets), an SSE parser, an event stream that reconnects and resumes after the last event it saw, `TerminalStream` and `TerminalSession` for agents' terminals (base64 decoding, reconnecting with a fresh snapshot, ordered and batched keystrokes, debounced resizes), and `DaemonMonitor`, which keeps status, events, projects, agents and pending approvals current for the UI. No AppKit in it.
- `Snapshots/`: `CosSnapshots`, a debug tool that renders the app's views to PNGs against an in-process fake daemon (see below).
- `project.yml`: the XcodeGen spec. `Cos.xcodeproj` is generated from it and not checked in. SwiftTerm is pinned to exactly 1.11.2, the last release before its Metal renderer, whose shaders need Xcode's separately downloaded Metal toolchain.

## Build

```sh
cd clients/desktop
xcodegen generate
xcodebuild -project Cos.xcodeproj -scheme Cos -derivedDataPath build build
```

The app is at `build/Build/Products/Debug/cos.app`.

## Run

```sh
open build/Build/Products/Debug/cos.app
```

It looks for the daemon at `~/.cos/cos.sock`, or `$COS_HOME/cos.sock` when `COS_HOME` is set. Apps opened from Finder don't see your shell's environment, so pass it explicitly to point the app at a test daemon:

```sh
COS_HOME=$PWD/../../.sandbox/app-home ../../target/release/cos daemon &
open --env COS_HOME=$PWD/../../.sandbox/app-home build/Build/Products/Debug/cos.app
```

If the daemon isn't running, the app says so and shows how to start it (`cos daemon install`). It checks again every few seconds and picks up where it left off once the daemon is back.

## Test

```sh
cd clients/shared/CosKit
swift test
```

Or through Xcode, from `clients/desktop`:

```sh
xcodebuild -project Cos.xcodeproj -scheme Cos -derivedDataPath build test
```

The live tests against the real daemon are skipped unless you give them a `cos` binary and a scratch home. One starts the daemon, follows the stream, restarts the daemon with SIGTERM and checks nothing is missed or repeated. The other makes throwaway git repos next to the scratch home and goes through every project and workspace route, including the 400, 404 and 409 errors. Each stops its own daemon by pid and removes what it created:

```sh
cargo build --release   # from the repo root
cd clients/shared/CosKit
COS_E2E_BINARY=$PWD/../../../target/release/cos COS_E2E_HOME=$PWD/../../../.sandbox/app-home swift test
```

Keep the scratch home's path short. A Unix socket path can't be longer than 104 bytes, so a home deep inside a worktree fails with "socket path is too long".

Swift doesn't build in the Docker dev image, so these tests run natively.

## Check the layout without a daemon

`CosSnapshots` compiles the app's views (everything in `Cos/` except `CosApp.swift`) with a fake daemon. The fake serves canned projects, agents in every state, a pending approval, and terminal streams that replay a Claude Code screen. The tool draws each page offscreen in dark and light, and prints the keystroke and resize requests the terminals sent:

```sh
xcodebuild -project Cos.xcodeproj -scheme CosSnapshots -derivedDataPath build build
build/Build/Products/Debug/CosSnapshots ../../.sandbox/snapshots
```

Offscreen drawing leaves the sidebar's list blank, so the tool also draws the sidebar's table on its own (`*-sidebar.png`). Colours in its unselected rows, vibrancy and window chrome still don't show.
