# cos for Mac

A menu bar app that shows the cos daemon's status and its live event log. It only reads from the daemon; it doesn't start or control it yet.

- `Cos/`: the SwiftUI app (macOS 14+). A menu bar item and one window, no Dock icon (`LSUIElement`).
- `../shared/CosKit/`: the Swift package the app is built on, shared with the iPhone app later. Models, an HTTP/1.1 client for the daemon's Unix socket, an SSE parser, and an event stream that reconnects and resumes after the last event it saw. No AppKit in it.
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

The live test against the real daemon is skipped unless you give it a `cos` binary and a scratch home. It starts the daemon, follows the stream, restarts the daemon with SIGTERM and checks nothing is missed or repeated, then stops it:

```sh
cargo build --release   # from the repo root
cd clients/shared/CosKit
COS_E2E_BINARY=$PWD/../../../target/release/cos COS_E2E_HOME=$PWD/../../../.sandbox/app-home swift test
```

Swift doesn't build in the Docker dev image, so these tests run natively.
