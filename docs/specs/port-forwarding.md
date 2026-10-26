# Port forwarding to another machine's localhost (spec)

Status: built, with the recommended answer to each open decision. The contract now lives in [daemon.md](../daemon.md#port-forwarding) and [desktop.md](../desktop.md#ports); this file stays as the record of why.

## 1. Context & problem

The Mac app can drive a daemon on another machine, usually a Linux box on EC2 reached over Tailscale ([linux.md](../linux.md)). Agents there run dev servers on that machine's loopback: `localhost:3343`, Vite on `localhost:5173`. Today you can't see them from the Mac without SSH.

You want to pick the machine in the app, add port 3343, open `http://localhost:3343` on the Mac and get that dev server, as if it ran on the Mac.

What exists to build on:

- **The tailnet listener and its guard.** Every request on the daemon's tailnet port (7700) goes through `admit` (`crates/cos/src/daemon/remote_api.rs:94-111`): same Tailscale user, then an approved device (`Guard::admit`, `crates/cos/src/daemon/tailnet.rs:558-612`). Refusals are `Refusal {status, code, message}` (`tailnet.rs:484-498`, rendered at `remote_api.rs:141-149`).
- **Terminals on other machines (commit `0f02036`).** That commit didn't add a transport: it took `/v1/shells` out of `HOST_ONLY` (`remote_api.rs:40`), so approved devices open shells over the tailnet. Terminals stream as server-sent events (`GET /v1/shells/{id}/terminal`) and keys go up as `POST …/keys` (base64 JSON). It matters here in two ways: the same guarded listener and bridge connection code carry the new route, and an approved device can already run any command as the daemon's user, `curl localhost:3343` included. Forwarding gives a device no reach it doesn't already have.
- **The bridge** (`crates/cos-bridge`) opens one HTTP/1.1 connection per request over the socket or TCP (`http.rs:135-204`), with a 5 s connect timeout (`lib.rs:45-47`). The Tauri shell keeps one bridge to the host in use and closes every stream when you switch hosts (`clients/desktop/src-tauri/src/commands.rs:89-94`).

SSE plus POST can't carry a browser's TCP traffic: it is byte-oriented and two-way, and websockets need both directions at once with low latency. So this needs one new piece of transport: an HTTP/1.1 `Upgrade` to a raw byte stream, on the same listener and behind the same guard. axum 0.8.9's `serve`, which both listeners use (`tailnet.rs:462-477`, `daemon/mod.rs:161-171`), already serves connections with upgrades (`serve_connection_with_upgrades`), so no server change is needed.

## 2. Goals / non-goals

**Goals**

- From the Mac app, with another machine selected, forward one or more of its loopback ports to the same port on the Mac's loopback.
- Anything that speaks TCP works: plain HTTP, websockets (HMR), HTTPS dev servers, gRPC, a database port.
- The daemon only ever connects to its own loopback, only for approved devices, through the existing guard.
- You can see each forward's state (listening, nothing running there, refused, open connections) and open it in a click.
- Forwards stop when you switch machines or quit the app.

**Non-goals**

- Forwarding the Mac's ports to the remote machine (reverse).
- Forwarding to any host other than the daemon's own loopback.
- UDP.
- iPhone support, port discovery and agent-initiated "open this URL" (see section 10, Later).
- A `cos forward` CLI command. The route makes it easy later, but the CLI doesn't use cos-bridge, so it's separate work.

## 3. Approach

A TCP tunnel through the existing cos connection. One browser connection is one tunnel, which is one upgraded HTTP request to the daemon.

```
Browser on the Mac -> http://localhost:3343
  -> cos desktop app (cos-bridge, in the Tauri process)
       listens on 127.0.0.1:3343 and [::1]:3343 on the Mac
  -> per accepted TCP connection: a new TCP connection to the daemon,
       GET /v1/forward/3343
       Connection: upgrade
       Upgrade: cos-forward
     over Tailscale to the daemon's tailnet port (7700)
  -> daemon: the usual guard (same Tailscale user, approved device),
       then connect to 127.0.0.1:3343 (else [::1]:3343) on its own machine,
       then 101 Switching Protocols
  -> both sides copy bytes until each direction closes
  -> dev server
```

- **Daemon** (`crates/cos/src/daemon/forward.rs`, new): one route, `GET /v1/forward/{port}`. With the upgrade headers it connects to loopback first and only then answers 101, so every failure is a normal JSON error with a `code`. Without them it is a probe, saying whether anything listens there. After 101 it runs `tokio::io::copy_bidirectional` between the upgraded connection and the loopback socket. A small registry counts tunnels per device, so a device can be cut off and a cap enforced.
- **Bridge** (`crates/cos-bridge/src/forward.rs`, new): `Bridge::open_tunnel(port)` does the upgrade handshake on a connection from the existing `connect()`. A `Forwarder` owns a local listener and spawns one tunnel per accepted connection. It reports its state to a `Sink<ForwardMessage>`, like the other streams.
- **Tauri shell:** commands to add, remove, list, follow and open forwards. They live in `AppState` beside the streams and are dropped on `switch_to`.
- **UI:** a Ports section for the machine in use.

## 4. Alternatives considered

| Option | Verdict |
|---|---|
| **TCP tunnel to a real localhost port on the Mac** (chosen) | The browser sees a real `localhost:3343`, so absolute paths, cookies, websockets and HMR, Vite's host check, secure-context APIs (`crypto.subtle`, service workers, clipboard) and OAuth redirects to `localhost` all work unchanged. It reuses the guard, device approval and the one tailnet port. Cost: one new transport (HTTP upgrade) and a listener in the app. |
| Path-based HTTP proxy, `/v1/proxy/3343/...` | Rejected. The page lives under a prefix, so absolute asset paths (`/assets/app.js`, `/@vite/client`) miss, redirects and cookies scoped to `/` break, and websockets need special handling through an HTTP proxy. It also only covers HTTP. |
| Browse the Tailscale address directly, `http://ec2-box:3343` | Rejected. The dev server must bind `0.0.0.0`, so every device on the tailnet (and anything else on the box's networks) reaches it, past cos's device approval. Vite rejects the unknown `Host`. The page isn't `localhost`, so secure-context APIs are off and OAuth redirects to `localhost` fail. |
| `ssh -L 3343:localhost:3343 ec2-box` | Rejected. It needs SSH keys set up between the Mac and the box and a separate process to manage, and it bypasses cos's device approval, so the app can't show or control it. |
| WebSocket instead of a bare `Upgrade` | Not needed. WebSocket framing adds masking and per-message overhead and a dependency, for no gain: both ends are ours, and hyper exposes the upgraded connection directly. A custom `Upgrade: cos-forward` token is plain HTTP/1.1 (RFC 9110 §7.8). |
| One multiplexed connection per forward (yamux, HTTP/2 CONNECT) | Not now. It saves a TCP connection and a guard check per browser connection, but browsers keep about six connections per host open and reuse them, so the saving is small, and it costs a dependency and flow-control bugs. It can come later behind the same commands. |

## 5. Affected components

| File | New / modified | Notes |
|---|---|---|
| `crates/cos/src/daemon/forward.rs` | new | The route, loopback connect (v4 then v6), probe, upgrade, byte copy, tunnel registry (per-device count, cap, close by device). |
| `crates/cos/src/daemon/api.rs` | modified | `AppState` gets `forwards: super::forward::Forwards` (`api.rs:89-106`); `router` merges `forward::routes()` (`api.rs:108-131`). |
| `crates/cos/src/daemon/remote_api.rs` | modified | `RemoteClient` (`:46-50`) becomes `pub(crate)` so the route knows the device's node id. `deny_device` and `remove_device` (`:165-180`) close that device's tunnels. |
| `crates/cos/src/daemon/tailnet.rs` | modified | `serve` (`:462-477`) turns on TCP keepalive for accepted connections, with axum's `ListenerExt::tap_io` and `socket2::SockRef` (socket2 is already in `Cargo.lock`). |
| `crates/cos/src/daemon/mod.rs` | modified | Builds `Forwards` into `AppState`. |
| `crates/cos/src/daemon/tests.rs` | modified | Integration tests (section 8). |
| `crates/cos/Cargo.toml`, `Cargo.toml` | modified | `socket2` as a direct dependency. |
| `crates/cos-bridge/src/forward.rs` | new | `Bridge::open_tunnel`, `Forwarder` (local listener, fallback port, per-connection tunnel, 502 page, state). |
| `crates/cos-bridge/src/http.rs` | modified | `connect()` (`:170-204`) becomes `pub(crate)` and sets keepalive on TCP. The handshake for tunnels uses `.with_upgrades()`. |
| `crates/cos-bridge/src/messages.rs` | modified | `Forward`, `ForwardMessage`. |
| `crates/cos-bridge/src/lib.rs` | modified | `mod forward`, re-exports. |
| `crates/cos-bridge/Cargo.toml` | modified | `socket2`. |
| `crates/cos-bridge/tests/bridge.rs` | modified | The fake daemon gets an upgrade route; tunnel and forwarder tests. |
| `clients/desktop/src-tauri/src/commands.rs` | modified | `AppState` holds forwards; `switch_to` (`:89-94`) stops them; new commands. |
| `clients/desktop/src-tauri/src/hosts.rs` | modified | Only if forwards persist (decision 2): `Host` (`:20-28`) gets `forwards: Vec<u16>`, defaulted so old `hosts.json` files load. |
| `clients/desktop/src-tauri/src/main.rs` | modified | Registers the commands (`:23-43`). |
| `clients/desktop/src-tauri/build.rs`, `capabilities/default.json` | modified | The new commands' `allow-…` permissions. |
| `clients/desktop/src/bridge/types.ts`, `tauri.ts` | modified | `Forward`, `ForwardMessage`, the `Bridge` methods. |
| `clients/desktop/src/bridge/fake.ts`, `fake/forwards.ts` | modified, new | Canned forwards (section 6.3). |
| `clients/desktop/src/screens/sidebar/PortsSection.tsx` | new | The Ports section. |
| `clients/desktop/src/screens/Sidebar.tsx` | modified | Shows it for another machine. |
| `clients/desktop/src/app/ports.ts`, `ports.test.tsx` | new | Store slice and tests. |
| `docs/daemon.md`, `docs/desktop.md`, `docs/proposal.md` | modified (last step) | Contract and Progress. |

## 6. API / contract changes

### 6.1 Daemon route

In the style of [daemon.md](../daemon.md). Served on the Unix socket and the tailnet listener. Over the tailnet, the guard runs first as for every route, so an unapproved client gets the usual 403 codes and never learns the route exists.

| Route | Headers | Returns |
|---|---|---|
| `GET /v1/forward/{port}` | none | Probe: 200 `{"port": 3343, "listening": true, "address": "127.0.0.1:3343"}`, or `"listening": false, "address": null`. Opens a connection to loopback and closes it without sending anything, like the peer probe. |
| `GET /v1/forward/{port}` | `Connection: upgrade`, `Upgrade: cos-forward` | 101 `Switching Protocols` with `Upgrade: cos-forward`, then raw bytes both ways. The daemon connects to loopback before answering, so 101 means the dev server accepted. |

**Where it connects.** `127.0.0.1:{port}`, and if that is refused, `[::1]:{port}`, each with a 3 s timeout. There is no host parameter and no name lookup: the address is built from the two fixed loopback IPs and the port, so the route can't be pointed at the server's network or the cloud metadata service (`169.254.169.254`). Node 17+ and Vite often listen on `::1` only, which is why v6 is tried too.

**Errors** are `{"error": "…", "code": "…"}` on both the socket and the tailnet (the route uses `Refusal`, not `ApiError`, so the bridge can tell them apart):

| Status | `code` | When |
|---|---|---|
| 400 | `bad_port` | `{port}` isn't a number from 1 to 65535. |
| 502 | `nothing_listening` | Both loopback addresses refused: nothing runs on that port there. The message names the machine and port. |
| 504 | `connect_timeout` | Neither answered within 3 s. |
| 503 | `too_many_forwards` | This device already has 256 open tunnels, or the daemon 1024. |
| 403 | the guard's codes | `device_pending`, `wrong_user`, … as in "Refusals". |
| 404 | (none) | A daemon older than this route. The app says the machine's cos is too old. |

A request to the route with `Upgrade` set to anything other than `cos-forward` is 400 `bad_upgrade`.

**Socket: allowed.** The socket's user can already open any loopback port on that machine, so the route grants nothing there. Serving it keeps the rule that every route works on both listeners unless it's host-only, and it lets bridge and daemon tests run without a tailnet. The app doesn't offer forwarding for This Mac (6.3).

**Ports: none refused besides 0.** An approved device already has a shell as the daemon's user (`0f02036`), so a deny-list (22, the daemon's own 7700) would add no safety, and it would block legitimate uses such as a database port. The daemon's tailnet port isn't bound on loopback (`tailnet.rs:369-409` binds only tailnet addresses), so a forward to 7700 just gets `nothing_listening`.

**Tunnel lifetime.**

- Bytes are copied with `copy_bidirectional`, which passes each side's EOF on as a write shutdown, so half-closed connections behave as on a direct connection.
- No idle timeout: a websocket can sit quiet for hours.
- Each tunnel ends when the daemon stops or the tailnet listener closes. After 101, hyper no longer tracks the connection and graceful shutdown won't wait for it, so the tunnel task watches `state.stopping`, which is the listener's `closing` on the tailnet (`tailnet.rs:298-301`).
- Denying or removing a device closes its tunnels at once. The guard reads approval per request (`tailnet.rs:558-612`), but a tunnel is one request that may last hours. The registry is keyed by the `RemoteClient` node id, and the deny and remove routes call `forwards.close_device(node_id)`.
- Accepted tailnet sockets get TCP keepalive (60 s idle, 10 s interval, 3 probes), so a Mac that vanished (slept, changed network) has its tunnels reaped within about 90 s.

**Events: none.** One page load opens several connections and an HMR reload opens more. The event log is permanent, numbered, and replayed to every app and the iPhone, so tunnel events would drown everything else and grow `cos.db` with noise. Approving the device is already the logged, auditable act. The daemon writes a `tracing` debug line per tunnel and a warning per refusal to its log instead. If an audit trail is wanted later, add `forward.started` once per device and port per hour, not per connection.

### 6.2 Bridge (Tauri commands)

In the style of the command table in [desktop.md](../desktop.md). Forwards belong to the host in use. They live in the Rust process, not the page, so a page reload doesn't stop them (`on_page_load` only closes streams, `main.rs:60-66`).

| Command | Arguments | Result |
|---|---|---|
| `forwards_list` | – | `ForwardList`: `{host: string, forwards: Forward[]}` for the host in use. |
| `forward_add` | `{remotePort: number, localPort?: number}` | `Forward`. Binds the Mac's port (the same as `remotePort` unless `localPort` is given), then probes the remote. `bad_request` for a port outside 1–65535, a `remotePort` already forwarded, or This Mac in use ("This Mac's ports are already here"). A 404 probe is `io` with code `too_old` and the message "<machine>'s cos is too old to forward ports. Run `cos upgrade` there."; the forward isn't kept. Guard refusals come back with their `code`, and the forward isn't kept. |
| `forward_remove` | `{remotePort: number}` | `ForwardList`. Closes the listener and every open tunnel of that forward. |
| `follow_forwards` | `{onMessage: Channel<ForwardMessage>}` | `number`: a stream id, closed by `close_stream` like the others. |
| `forward_open` | `{remotePort: number}` | –. Opens `http://localhost:{localPort}/` in the default browser (`/usr/bin/open`, as `reveal_path` does) or in an app window (decision 1). The URL is built in Rust from a forward that exists, so the UI can't open arbitrary URLs. |

```ts
type Forward = {
  remotePort: number;
  localPort: number;              // differs from remotePort when the Mac's port was busy
  state: "listening" | "error";   // the Mac's listener; "error" only if no local port could be bound
  remote: "unknown" | "listening" | "nothing_listening" | "refused" | "unreachable";
                                  // from the probe, then from each tunnel's outcome
  code: string | null;            // the daemon's code for "refused" (device_pending, …)
  message: string | null;         // the latest problem, in words, for the panel
  connections: number;            // tunnels open now
};

type ForwardMessage = { type: "forwards"; host: string; forwards: Forward[] };
```

- `follow_forwards` sends the whole list first and then on every change. Connection counts change fast, so changes are coalesced to at most four messages a second.
- **Local port busy.** The bridge binds `127.0.0.1:P` and `[::1]:P` (the browser may resolve `localhost` to either). If either is taken on the Mac, it tries `P+1` to `P+10`, as Vite does, then lets the OS pick. `[::1]` failing with "address not available" (IPv6 off) is fine: it carries on with v4 only. The panel shows the local port whenever it differs. OAuth redirect URIs registered for the original port, and links that name it, break then. The panel says so in the row's tooltip.
- **A tunnel that fails** before 101 (nothing listening, refused, unreachable) can't be passed on as such, because the browser's connection is already accepted. The bridge peeks at the first bytes. If they start an HTTP request, it answers `502 Bad Gateway` with a one-line HTML page ("Nothing is listening on localhost:3343 on ec2-box"). Otherwise, for TLS (first byte `0x16`) and other protocols, it just closes. The failure also updates `remote` and `message`.
- **Host switch.** `host_select` and `host_remove` (through `switch_to`) drop every forward of the old host. If forwards persist (decision 2), the new host's saved ports are added again.
- **Quit.** The process exits and the OS closes every socket. Closing the window only hides it (`main.rs`), so forwards keep running while the app sits in the menu bar.
- **Errors** use the existing `{kind, message, code?}`.

### 6.3 UI

- **Where.** A "Ports" section at the bottom of the sidebar, above the footer that names the machine in use. It belongs to the machine, not to the project the sidebar shows, so it stays put when you change projects. It is hidden when This Mac is in use: its ports are already on the Mac, so `forward_add` refuses there too.
- **Rows.** One row per forward: `:3343`, then `→ localhost:3344` when the local port differs, a state dot (green when listening with something running there, amber for `nothing_listening` or `unknown`, red for `refused`, `unreachable` or `error`), the open connection count when above zero, Open, and × to remove. The tooltip carries `message`, and the OAuth note when the port moved.
- **Add.** `+` turns into a small inline field for a port number, checked as you type (1–65535, not already forwarded). Return adds it. A "Local port" field is behind "More" for when you want another one.
- **Too old.** A `too_old` error replaces the field with "<machine>'s cos is too old to forward ports. Run `cos upgrade` there." The section stays, so you can try again after upgrading.
- **Refusals.** A refused machine already shows the full "Another machine refusing" page in place of the content, so the Ports section only needs its red dot.
- **⌘K.** Each forward appears in the switcher as "Open localhost:3343".
- **Fake bridge** (`src/bridge/fake/forwards.ts`). Studio, once approved, starts with two forwards: 5173 listening with 2 connections, and 3343 forwarded to local 3344 (busy) with `nothing_listening`. `forward_add` of 8080 succeeds. `?portsTooOld` makes Studio answer `too_old`. This Mac refuses `forward_add`. `forward_open` records the URL so tests can check it.

### 6.4 Backward compatibility

- **New app, older daemon.** The probe in `forward_add` gets a 404 with no `code` (axum's empty 404 through `remote_router`'s fallback), which the bridge turns into `too_old`. Nothing else changes for that machine.
- **Older app, new daemon.** The route is unused.
- **`hosts.json`.** If forwards persist, `forwards` is optional with a default, so files from older apps load, and older apps ignore the field. Older apps rewrite the file without it, so downgrading forgets saved ports, which is harmless.
- No database migration and no event kinds.

## 7. Implementation plan

Each step is one commit of under about 150 lines, plus tests, and passes `make test` and `make lint` (and `make ui-test ui-lint ui-typecheck` for UI steps) by itself.

1. **Daemon: the route.** `daemon/forward.rs` with the probe, the loopback connect (v4 then v6, 3 s), the upgrade and `copy_bidirectional`, `bad_port`, `bad_upgrade`, `nothing_listening` and `connect_timeout`, merged into `api::router`. Tests over the socket: echo round trip, probe, nothing listening, bad port, a v6-only echo server.
2. **Daemon: the registry.** `Forwards` in `AppState` (per-device and total caps, `too_many_forwards`), tunnels closing on `stopping`, and `deny_device` and `remove_device` closing a device's tunnels. Tests through `RemoteDaemon` with `FakeTailscale`: approved device tunnels, pending device gets `device_pending` before any loopback connect, deny mid-stream closes it, turning remote off closes it.
3. **Daemon: keepalive** on accepted tailnet sockets (`tap_io` and socket2), with a unit test that the option is set.
4. **Bridge: `open_tunnel`.** The upgrade handshake on `connect()`, keepalive on TCP, errors mapped (404 → `too_old`, codes passed through). Tests against the fake daemon in `tests/bridge.rs`, which gets an upgrade echo route, over both socket and TCP.
5. **Bridge: `Forwarder`.** Local listener on v4 and v6, port fallback, one tunnel per connection, the 502 page, state through a `Sink<ForwardMessage>` with coalescing. Tests: bytes round trip through a real local port, a busy port moves to `P+1`, 502 for HTTP when nothing listens, close without a page for TLS, half-close, 50 parallel connections.
6. **Tauri commands.** `forwards_list`, `forward_add`, `forward_remove`, `follow_forwards`, `forward_open` in `commands.rs`, registered in `main.rs`, `build.rs` and the capability, and forwards dropped in `switch_to`. A unit test beside `selecting_a_host_closes_every_stream_and_retargets` (`commands.rs:435`) checks that selecting a host stops forwards. `acl.test.ts` covers the permissions. Built natively; not covered in Docker.
7. **Persistence** (only if decision 2 is yes). `Host.forwards` in `hosts.rs`, saved on add and remove, restarted on select. `hosts.rs` tests for old files and round trips.
8. **UI: bridge types and fake.** `types.ts`, `tauri.ts`, `fake/forwards.ts`, the store slice in `app/ports.ts`. Store tests against the fake.
9. **UI: the Ports section.** `PortsSection.tsx` in the sidebar, the add field, rows, Open, ⌘K entries. Testing Library tests against the fake: hidden on This Mac, add, busy port shown, `too_old` message, remove, Open calls `forward_open`.
10. **Docs.** Move the contract into `daemon.md` (route, codes, "no events") and `desktop.md` (commands, Ports section, fake bridge), and add a Progress entry to `proposal.md`.

## 8. Test plan

All in Docker (`make test`, `make lint`, `make ui-test`, `make ui-lint`, `make ui-typecheck`), except the Tauri shell, which is built natively with `make desktop` and tried by hand.

**Daemon** (`crates/cos/src/daemon/tests.rs`, with `RemoteDaemon` and `FakeTailscale`, whose machine is on 127.0.0.1):

- A tunnel to a loopback echo server carries bytes both ways, including 1 MB each way at once.
- The probe says `listening: true` with the server up and `false` with it down.
- An echo server on `[::1]` only is reached (skipped where the test container has no IPv6 loopback).
- Nothing listening gives 502 `nothing_listening`, with no 101.
- Ports `0`, `65536` and `abc` give 400 `bad_port`.
- A wrong `Upgrade` gives 400 `bad_upgrade`.
- A pending, denied, wrong-user or tagged device gets the guard's 403 and the echo server sees no connection. This is the point of running the guard first.
- Denying the device while a tunnel is open closes it within a second. So do removing the device, `PUT /v1/tailnet {enabled: false}`, and stopping the daemon.
- A half-close works: the client shuts its write side, the echo server sees EOF and replies, and the client still reads the reply.
- The 257th tunnel from one device gets 503 `too_many_forwards`.
- Non-loopback is refused by construction: the route has no host input, and the test asserts that the connect address for any port is `127.0.0.1` or `::1` (a unit test on the address helper).

**Bridge** (`crates/cos-bridge/tests/bridge.rs`, with the fake axum daemon, which gains an upgrade echo route and a switch to answer 404 or a refusal):

- `open_tunnel` works over the socket and TCP. A 404 maps to `too_old`, a 403 keeps its `code`, and an unreachable host is `not_running`.
- `Forwarder` serves a real `127.0.0.1` port end to end. A port held by the test moves to `P+1`, and the message says so.
- HTTP to a forward whose remote has nothing listening gets a 502 page. A TLS ClientHello just gets a close.
- 50 parallel connections each echo their own bytes, and `connections` goes up to 50 and back to 0.
- Dropping the forwarder closes its listener and its tunnels.

**UI** (Vitest and Testing Library against the fake bridge): the section is hidden on This Mac and shown on Studio after approval; adding, removing, the busy-port arrow, the `too_old` text, and Open calling `forward_open` with the right port; ⌘K lists forwards.

**By hand, once:** the app against a real EC2 box running Vite, checking HMR over the websocket, a page reload, the Mac sleeping and waking, and denying the Mac from the box mid-session.

## 9. Edge cases

| Case | What happens |
|---|---|
| Dev server not running | The probe and each tunnel get `nothing_listening`. The row turns amber with "Nothing is listening on 3343 on ec2-box", and an HTTP request in the browser gets the 502 page. Starting the server fixes it with no action: the next connection tunnels. |
| Daemon restarts | Open tunnels drop, and the browser sees closed connections (HMR shows "server connection lost" and retries). The Mac's listener stays. While the daemon is down, tunnels fail as `unreachable` (502 page), and once it is back the next connection works. |
| Host switch | Every forward of the old host stops: listeners close and tunnels drop. The new host's saved forwards start if decision 2 says so. |
| Many parallel connections | One tunnel per browser connection. Browsers keep about six per host and reuse them, so even an unbundled Vite app with hundreds of modules uses a handful. Each tunnel costs one guard check (cached whois, one SQLite read). Caps: 256 per device, 1024 per daemon. |
| Half-closed sockets | `copy_bidirectional` on both sides passes EOF on as a write shutdown and keeps the other direction open until it ends too. Tested on both sides. |
| IPv6 localhost | The Mac listens on `127.0.0.1` and `[::1]`, and treats either being taken as busy, so `localhost` can't land on another local server. The daemon tries `127.0.0.1`, then `[::1]`. A Mac with IPv6 off uses v4 only. |
| Websocket upgrade | Transparent: the tunnel carries raw bytes, so the browser's `Upgrade: websocket` is just data to it. |
| Long-lived connections | No idle timeout. TCP keepalive on both ends of the tailnet connection reaps dead ones. |
| Device revoked mid-stream | `deny` and `remove` close that device's tunnels at once. New tunnels get `device_denied`, the row turns red, and the app shows its refusal page as for any request. |
| Mac sleep | The tailnet connections die silently. After wake, the browser opens new connections and new tunnels. Old tunnels are reaped by keepalive on the daemon (about 90 s) and on the Mac. The Mac's listener is local and survives sleep. |
| Port already forwarded | `forward_add` refuses it. Remove it first to change its local port. |
| Two Macs forwarding the same remote port | Fine: each has its own tunnels. The dev server sees two clients from loopback. |
| HTTPS dev server | Bytes pass through, and the browser checks the certificate against `localhost`, as it would locally. On failure the bridge closes instead of writing an HTTP 502 into a TLS stream. |
| Window closed, app in the menu bar | Forwards keep running. Quitting stops them. |
| Page reload in the app | Forwards keep running (they live in Rust), and the UI follows them again. |

## 10. Rollout

- It ships in one release with the app. The machine's daemon must run that release too: run `cos upgrade` on the EC2 box. Until then the panel says that machine's cos is too old. Nothing changes for anyone who doesn't add a forward.
- No flag: the feature is opt-in per port, and the route is behind the existing guard.
- **Later, out of scope:**
  - The daemon lists ports listening on its loopback (`/proc/net/tcp*` on Linux, `lsof` on macOS), so the panel can suggest "5173 · node" with one-click add.
  - iPhone support. iOS can't run a local listener for Safari in the background, so it would be an in-app web view through the same route.
  - An agent asks for a URL to be opened on your Mac ("open http://localhost:3343"), which the app turns into a forward and a browser tab after you confirm.
  - A `cos forward [--host H] PORT` CLI.
  - Multiplexing tunnels over one connection.

## 11. Decisions needed from the user

All taken as recommended when it was built (26 Oct 2026).

- [x] **Open where:** the default browser (recommended: cookies, devtools and extensions as usual), an in-app Tauri window, or both. An in-app window would get its own label, which `capabilities/default.json` doesn't list (`"windows": ["main"]`), so the forwarded page can't call cos commands.
- [x] **Persist forwards per host in `hosts.json`** and restart them when you select that host (recommended: yes, ports only; dev servers keep their ports), or start empty every time.
- [x] **Allow the route on the Unix socket** (recommended: yes; it grants nothing the socket's user lacks, and it keeps tests simple).
- [x] **No port deny-list** beyond 1–65535 (recommended: none, since an approved device already has a shell), or refuse some, such as 22 or the daemon's own port.
- [x] **Busy local port:** try `P+1` to `P+10`, then let the OS pick (recommended, as Vite does), or let the OS pick straight away.
- [x] **No event-log entries for tunnels** (recommended), or a rate-limited `forward.started` per device and port.
- [x] **Where the section sits:** the bottom of the sidebar (recommended), or a page under Settings for the machine.
- [x] **Caps:** 256 tunnels per device and 1024 per daemon.

## 12. Open risks

- **Other local users on the Mac.** The forward listens on the Mac's loopback, so any process there, other OS users' included, can reach the remote dev server through this device's approval. `daemon.md` refuses to auto-approve a Mac's own tailnet address for the same reason. This is the same exposure as `ssh -L`, and forwards are explicit, listed and stopped on quit. Fine on a single-user Mac. A shared Mac would need a per-forward token, which browsers can't send.
- **DNS rebinding.** A hostile site in the Mac's browser can try to reach `localhost:3343` through DNS rebinding, as it can any local dev server. Vite's host check helps. This is not new, but the forward makes a remote server look local.
- **Streams already open when a device is denied.** The guard runs per request, so today's SSE streams (events, terminals) also outlive a deny. This spec closes tunnels on deny, but not those streams. That gap exists already and is worth its own fix.
- **Upgrades through the middleware.** hyper's `OnUpgrade` travels in the request's extensions through `admit` and `refuse_host_only` (`remote_api.rs:86-91`). That should work with axum's `from_fn`, but step 1's tests must cover it over the tailnet listener, not only the socket.
- **Slow paths.** When Tailscale has no direct path and relays through DERP, page loads with many requests will be slow. The panel can't tell. Tailscale's `tailscale ping` shows it.
- **Untested in the Tauri shell in Docker.** Steps 6 and 7 build only natively, as the other shell commands do, so they need a manual run with `make desktop`.
