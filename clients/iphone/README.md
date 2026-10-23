# cos for iPhone

The iPhone app for the cos daemon, in SwiftUI. It reaches a Mac or server running cos over Tailscale and works like a messaging app: the home screen lists each project's Chief of Staff as a chat, and a chat's ⋯ menu leads to its workspaces and sessions. What waits on you is a pinned row on top. The screens, the routes it uses and the plan for push are in [docs/iphone.md](../../docs/iphone.md).

- `Cos/App`: the entry point and `AppModel` (hosts, the session for the current one, launch flags).
- `Cos/Model`: `HostStore` (saved hosts), `HostSession` (one host's lists, kept current from `/v1/events`), `Chiefs` (the chiefs list and `ReadMarks`), `TailnetFinder` and `TailnetListing` (the tailnet, as the saved hosts list it), `ChiefChat`, `AgentDetail` and the `Notifications` seam.
- `Cos/Views`: the screens.
- `Cos/Fake`: `FakeCos`, a daemon in memory with invented data, for `-fakeData`, screenshots and tests.
- `CosTests`: unit tests of the models against `FakeCos`.
- `../shared/CosKit`: the client library (HTTP over TCP or a Unix socket, models, the event follower), a local Swift package.
- [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) 1.10.1: draws the live terminal. Xcode fetches it on the first build.

## Build

You need Xcode 26 or later with the iOS 26 simulator, and [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`). The Xcode project is generated from `project.yml` and isn't committed.

```sh
cd clients/iphone
xcodegen generate
open Cos.xcodeproj
```

From the command line, with a simulator (create one first if `xcrun simctl list devices` shows none):

```sh
xcrun simctl create "iPhone 17" com.apple.CoreSimulator.SimDeviceType.iPhone-17
xcodebuild -project Cos.xcodeproj -scheme Cos -destination 'platform=iOS Simulator,name=iPhone 17' build
xcodebuild -project Cos.xcodeproj -scheme Cos -destination 'platform=iOS Simulator,name=iPhone 17' test
```

CosKit's own tests run with `swift test` in `clients/shared/CosKit`. Neither builds in Docker.

## Run with invented data

Launch arguments (Product > Scheme > Edit Scheme > Arguments, or after `simctl launch <device> local.cos.iphone`):

| Argument | Effect |
|---|---|
| `-fakeData` | Two invented hosts, both answered by `FakeCos`, on an invented tailnet (gpu-box runs cos and isn't saved, a Mac without cos, this iPhone, an offline iPad, a tagged CI server). Nothing touches the network or your saved hosts. |
| `-screen <name>` | Opens a screen: `chiefs` (the default), `waiting`, `chief`, `chief-new`, `chief-info`, `chief-terminal`, `terminal`, `workspace`, `agent`, `agent-approval`, `hosts`, `add-host`. |
| `-fakeRefusal <code>` | The fake refuses the device, e.g. `device_pending`, `device_denied`, `wrong_user`. |
| `-fakeUnreachable` | The fake can't be reached. |
| `-noHosts` | Starts with no hosts (the welcome screen). |
| `-fakeOldDaemon` | The fake studio is an older cos that refuses its tailnet list (`host_only`). |

## Run on your iPhone

A free Apple ID is enough; no paid developer account.

1. On the iPhone: install Tailscale and log in as the same Tailscale user as the host. Turn on Developer Mode (Settings > Privacy & Security > Developer Mode) when it asks, after the first install.
2. In Xcode: open `Cos.xcodeproj`, select the Cos target > Signing & Capabilities, keep "Automatically manage signing", and pick your Personal Team. If Xcode says the bundle id is taken, change `local.cos.iphone` to something of yours there (don't commit the team or the id; `project.yml` leaves `DEVELOPMENT_TEAM` empty).
3. Connect the iPhone by cable (or on the same Wi-Fi once paired), pick it as the run destination and press Run.
4. The first launch is refused until you trust the developer: Settings > General > VPN & Device Management > your Apple ID > Trust.
5. In the app, add the host by its Tailscale name (`my-mac`) or its `100.x` address. The port defaults to 7700. After the first one, the add screen lists the other machines on your tailnet, and Connect adds one running cos.
6. The host answers `device_pending`, and the app shows the command to run there: `cos devices approve <n>`. Run it on the host; the app gets in by itself within a few seconds.

On the host, remote access is on by default; `cos remote status` says where it listens, and `cos remote on` turns it back on. Only devices of the host's Tailscale user get in.

With a free Personal Team the app's signature lasts **7 days**. After that the app won't open; connect the iPhone and press Run again. A free team can have 3 apps installed this way at once.

The app talks plain HTTP inside the tailnet's WireGuard tunnel over its own sockets, so App Transport Security doesn't apply and needs no exception.
