# cos for Mac

A menu bar app for the cos daemon. The menu bar panel shows its status and latest events. The window lists your projects (registered git repos) and their workspaces (worktrees cos owns) in a sidebar, where you can add and remove projects and create, rename and remove workspaces. The event log is the sidebar's first item. The app doesn't start the daemon itself yet.

- `Cos/`: the SwiftUI app (macOS 14+). A menu bar item and one window, no Dock icon (`LSUIElement`).
- `../shared/CosKit/`: the Swift package the app is built on, shared with the iPhone app later. Models, an HTTP/1.1 client for the daemon's Unix socket, an SSE parser, an event stream that reconnects and resumes after the last event it saw, and `DaemonMonitor`, which keeps status, events and projects current for the UI. No AppKit in it.
- `project.yml`: the XcodeGen spec. `Cos.xcodeproj` is generated from it and not checked in.

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

Swift doesn't build in the Docker dev image, so these tests run natively.
