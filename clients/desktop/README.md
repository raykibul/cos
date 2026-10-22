# cos for Mac

The Mac app for the cos daemon: a Tauri 2 app with a web UI. It has one window, a Dock icon and a menu bar icon. The contract between the UI and the shell is [docs/desktop.md](../../docs/desktop.md).

- `src/`: the web UI (React and TypeScript). Built and tested in Docker with `make ui-*`.
- `src-tauri/`: the shell (Rust). It has its own Cargo workspace, so `make test` and Docker never build it. It builds natively on the Mac.
- `../../crates/cos-bridge/`: everything that talks to the daemon (requests, the event stream, terminal streams). It has no Tauri code and is tested in Docker by `make test`.

## Install

Each release has the app as one universal (Apple Silicon and Intel) zip, ad-hoc signed. To install or upgrade it in `/Applications` (or `~/Applications`):

```sh
curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh -s -- --app
npx cos-agent@latest app    # the same, through npm
```

The installer quits a running copy first and clears the quarantine flag, since the app isn't notarized; a copy downloaded with a browser would be blocked by Gatekeeper. Add `--with-cli` for the CLI too. See "The Mac app" in [docs/releasing.md](../../docs/releasing.md#the-mac-app). The release workflow builds it with `tauri build --target universal-apple-darwin`, and `make release-local` does the same here.

## Build

You need Rust and the Tauri CLI on the Mac, and Docker for the UI:

```sh
cargo install tauri-cli --version "^2" --locked
make desktop
```

`make desktop` builds the UI into `dist/` (`make ui-build`), then bundles the app. It prints the path: `clients/desktop/src-tauri/target/release/bundle/macos/cos.app`. The app is ad-hoc signed, so it needs no Apple account.

## Run

```sh
open clients/desktop/src-tauri/target/release/bundle/macos/cos.app
```

The app looks for the daemon at `~/.cos/cos.sock`, or `$COS_HOME/cos.sock` when `COS_HOME` is set. Apps opened from Finder don't see your shell's environment. To point the app at a test daemon, run the binary inside the bundle with the variable set:

```sh
COS_HOME=$PWD/.sandbox/app-home ./target/release/cos daemon &
COS_HOME=$PWD/.sandbox/app-home clients/desktop/src-tauri/target/release/bundle/macos/cos.app/Contents/MacOS/cos-desktop
```

Keep the test home's path short. A Unix socket path can't be longer than 103 bytes, and the daemon also makes `hooks.sock` there, so a home deep inside a worktree fails.

Closing the window hides it. The app keeps running in the menu bar. Click the Dock icon or Open cos in the menu to bring it back. Quit from the menu bar icon or with Cmd-Q.

## Develop

```sh
make ui-dev        # Vite on localhost:5173, in Docker; leave it running
make desktop-dev   # in another terminal: the app, loading the dev server
```

`tauri.conf.json` has no `beforeDevCommand` or `beforeBuildCommand`, so start the UI yourself. In a plain browser, the UI on `localhost:5173` uses a fake bridge with canned data.

Checks for the shell run natively in `src-tauri`:

```sh
cd clients/desktop/src-tauri
cargo clippy --all-targets -- -D warnings
cargo test
```

## The shell

- **Commands:** `daemon_info`, `daemon_request`, `follow_events`, `attach_terminal`, `close_stream`, `hosts_list`, `host_add`, `host_remove`, `host_select`, `tailnet_peers` (always this Mac's daemon), `set_waiting_count`, `notify`, `reveal_path` and `open_in_terminal` (Ghostty if it's in an Applications folder, else Terminal). Streams live until `close_stream`, `host_select`, or a page reload.
- **Hosts:** the app talks to this Mac's daemon through its socket, or to another Mac's over Tailscale (TCP, port 7700 unless you pick another). Saved Macs, and the one in use, are in `~/Library/Application Support/local.cos.desktop/hosts.json`. Add one from the picker at the top of the sidebar. On the other Mac, run `cos remote on`, then approve this Mac there (`cos devices approve <id>`, or in its app under Waiting on you) the first time it connects. Settings > Remote access does the same for this Mac.
- **Permissions:** `capabilities/default.json` allows the cos commands, dragging the window and double-click to zoom, nothing else. `build.rs` lists the commands; a new command needs adding to both.
- **CSP:** scripts from the app only, plus `wasm-unsafe-eval` for ghostty-web's WASM. Styles may be inline. No CSP in dev, so Vite's hot reload works.
- **Window:** an overlay title bar, with the traffic lights centered in the UI's 52 px header (`trafficLightPosition` in `tauri.conf.json`).
- **Icons:** `icons/icon.svg` (the app) and `icons/tray.svg` (the menu bar template image) are the sources. Regenerate them with `cargo tauri icon icons/icon.svg -o <dir>` and copy `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns` and `icon.png` back. For the tray, run `cargo tauri icon icons/tray.svg -o <dir> --png 44` and copy the result to `icons/tray.png`.
