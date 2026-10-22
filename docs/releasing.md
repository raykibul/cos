# Releasing cos

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed. Asset names carry no version, so `https://github.com/raykibul/cos/releases/latest/download/cos-<target>.tar.gz` always points at the newest build, and `.../cos-desktop-universal-apple-darwin.zip` at the newest Mac app. `install.sh`, `cos upgrade` and the `cos-agent` npm package all depend on that.

## Steps

1. **Bump the version.** Set `version` under `[workspace.package]` in `Cargo.toml`, run `make test` so `Cargo.lock` picks it up, and commit. You don't need to change `npm/package.json`, `clients/desktop/package.json` or `clients/desktop/src-tauri/Cargo.toml`. The workflow sets the npm package's and the app's versions from the tag.
2. **Check it builds** (optional): `make release-local` builds every asset into `dist/release`, as the workflow does. See [Checking before a release](#checking-before-a-release).
3. **Tag and push.**

   ```sh
   git tag -a v0.2.0 -m "cos v0.2.0"
   git push origin main v0.2.0
   ```

4. **Watch the run** with `gh run watch`, or in the repo's Actions tab.

To rebuild a release for an existing tag, after fixing the workflow for example, run it by hand: `gh workflow run release.yml -f tag=v0.2.0`. The assets are replaced (`--clobber`).

## What the workflow does

1. **version:** fails unless the tag is `v` plus the workspace version in `Cargo.toml`, so `cos --version` always matches the tag.
2. **linux:** `make linux`, the same Docker and cargo-zigbuild build as on a dev machine, builds static musl binaries for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, and checks that each is statically linked. The job then runs the x86_64 build's `--version` and packs both with `scripts/package-release.sh`.
3. **macos:** on `macos-latest` (Apple Silicon), `cargo build --release` for `aarch64-apple-darwin` and, by cross-compiling, `x86_64-apple-darwin`. The minimum macOS is 11. The binaries aren't signed or notarized, except for the ad-hoc signature the linker adds on arm64. Downloads with `curl` or Node get no quarantine flag, so they run without a Gatekeeper prompt.
4. **desktop:** on `macos-latest`, installs Node 22 and pnpm (through corepack, at the version `clients/desktop/package.json` names), builds the UI with `pnpm install --frozen-lockfile` and `pnpm build`, then bundles one universal `cos.app` with the Tauri CLI from npm: `tauri build --target universal-apple-darwin --bundles app`. Tauri builds both targets and joins them with `lipo`. The version comes from the tag through `--config '{"version":"…"}'`, which sets the bundle's `CFBundleShortVersionString`, so no file in the repo needs bumping. The app is ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`) and needs macOS 13. `scripts/package-desktop.sh` checks the version, the bundle id, both CPUs and the signature, then zips the app with `ditto -c -k --keepParent cos.app`.
5. **release:** checks that all five assets are there and writes `SHA256SUMS` over all of them at once. Each build job uploads only its own files, so there are no partial sums to merge. It then creates the GitHub Release with generated notes, or uploads over an existing one. It needs only `GITHUB_TOKEN` with `contents: write`. It waits for every build job, so a failed app build holds back the CLI too, and a release is never half there.
6. **npm:** publishes `npm/` as `cos-agent@<version>`, but only if the repo has an `NPM_TOKEN` secret, and skips a version that's already on npm.

Each release has these assets:

| Asset | |
|---|---|
| `cos-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64, static |
| `cos-aarch64-unknown-linux-musl.tar.gz` | Linux arm64 (Graviton), static |
| `cos-aarch64-apple-darwin.tar.gz` | macOS, Apple Silicon |
| `cos-x86_64-apple-darwin.tar.gz` | macOS, Intel |
| `cos-desktop-universal-apple-darwin.zip` | the Mac app, `cos.app`, for Apple Silicon and Intel |
| `SHA256SUMS` | `sha256sum` output for the five files above |

Each tarball holds `cos`, plus `README.md` and `LICENSE` when the repo has them. The zip holds `cos.app`.

## The Mac app

`install.sh --app` installs the app, and `npx cos-agent app` does the same in Node:

```sh
curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh -s -- --app
curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh -s -- --app --with-cli
```

It's a flag on `install.sh` rather than a second script because the two share everything but the last step: picking the release, downloading, checking `SHA256SUMS`, and the CLI install that `--with-cli` reuses. There is one URL to remember. The steps:

1. **macOS only**, 13 or later. On Linux it stops before downloading anything.
2. **Download** `cos-desktop-universal-apple-darwin.zip` from the latest release, or `COS_VERSION`'s, check it against `SHA256SUMS`, and unpack it with `ditto` into a temporary folder. If the checksum doesn't match, nothing is touched.
3. **Pick the folder:** `/Applications/cos.app` if it can be written, else `~/Applications/cos.app`. A `cos.app` that exists and belongs to you counts as writable even when `/Applications` itself isn't (a standard user's Mac), since only its insides are replaced. A copy already in `~/Applications`, with none in `/Applications`, is upgraded where it is. When the installer falls back to `~/Applications`, it says so and prints the `sudo` command for `/Applications`. Under `sudo`, the app is handed back to you afterwards, so later upgrades need no `sudo`.
4. **Quit a running copy.** It finds the app's process with `pgrep -x -U <you> cos-desktop`, sends that exact pid `SIGTERM` (`kill <pid>`), and waits up to 10 seconds for it to exit. The app is only a window on the daemon, so agents keep running. It never uses `pkill -f` or `killall`.
5. **Replace the contents:** `rm -rf cos.app/Contents`, then `ditto` the new app into place. The `cos.app` folder itself stays.
6. **Clear the quarantine flag:** `xattr -dr com.apple.quarantine cos.app`.

Settings: `COS_VERSION` and `COS_DOWNLOAD_BASE`, as for the CLI. `COS_APP_DIR` replaces `/Applications` (still falling back to `~/Applications`). `COS_APP_NO_QUIT=1` skips step 4. The tests use the last two to stay away from `/Applications` and the running app.

### Gatekeeper and the quarantine flag

The app is signed ad hoc, which says nothing about who made it, and Apple hasn't notarized it. macOS checks an app with Gatekeeper when it carries the `com.apple.quarantine` extended attribute. Browsers, Mail, AirDrop and the like add it to what they download, and it carries over to files unpacked from such an archive. If you download the zip with a browser and double-click the app, Gatekeeper refuses to open it ("cos is damaged and can't be opened", or "Apple could not verify…"). You would then have to clear the flag yourself, or allow the app under System Settings > Privacy & Security.

`curl` and Node don't add the flag, and the installer clears it anyway, in case the zip came from a browser or a mirror. Clearing it means trusting this download without Apple's check. What you rely on instead is HTTPS from GitHub and the `SHA256SUMS` check, both from this repo's releases. Clearing the flag changes nothing else about how the app runs.

### Signing and notarization, later

With a Developer ID signature and notarization, Gatekeeper would accept a browser download as it is, and clearing the flag would no longer be needed (it would stay harmless). It would take:

- **An Apple Developer account** (paid, yearly) and a **Developer ID Application** certificate, exported as a `.p12` with a password.
- **Repo secrets:** `APPLE_CERTIFICATE` (the `.p12`, base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (`Developer ID Application: Name (TEAMID)`), `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password for that Apple ID) and `APPLE_TEAM_ID`. An App Store Connect API key (`APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH`) can replace the Apple ID and password.
- **In the desktop job:** import the certificate into a temporary keychain, then run the Tauri build with those variables in its environment and `signingIdentity` set to the identity, through `--config` so local builds stay ad hoc. Tauri then signs with the hardened runtime, submits the app to `notarytool`, waits, and staples the ticket. It already looks for the variables: today's build logs "skipping app notarization". The hardened runtime may need an entitlements file if it blocks something the app does; that needs trying.
- **The bundle id** `local.cos.desktop` should become a reverse-DNS name you own before the first signed release. It also names the app's settings folder, so the hosts list would need moving.
- The CLI binaries could be signed and notarized the same way (`codesign --options runtime`, then `notarytool` on a zip), though downloads with `curl` don't need it.

## Upgrading with `cos upgrade`

`cos upgrade` upgrades an installed `cos` to the latest release, along with the Mac app and the daemon's service. It reuses `install.sh` rather than downloading releases itself, so the checksum check lives in one place and `cos` needs no TLS stack. The steps:

1. **The latest release:** the tag at the end of the `Location` header of `curl -fsSI https://github.com/raykibul/cos/releases/latest` (`wget` if there's no `curl`), compared with this binary's version as semver. Without `--version` it only goes forward: a build newer than the latest release is left alone. `--version v0.1.5` goes to that release, older ones included. `--check` prints the installed, latest, app and daemon versions and stops.
2. **Where:** this binary's real path (`current_exe`, symlinks resolved). A binary inside a Cargo `target/` folder, or inside a git checkout of a Cargo project, is refused ("built from source at …; update it with `git pull && make build`"); `--force` overrides. A folder you can't write to is refused, with `sudo cos upgrade` or `COS_INSTALL_DIR` as the way out. Under `sudo`, only the CLI is upgraded.
3. **The CLI:** it downloads `install.sh` from `https://raw.githubusercontent.com/raykibul/cos/<tag>/install.sh`, so the script matches its release, and runs it with `sh -s` on stdin, with `COS_VERSION=<tag>`, `COS_INSTALL_DIR=<this binary's folder>` and `COS_NO_SERVICE_HINT=1`. The installer's output goes straight to the terminal. Afterwards the binary must report the new version.
4. **The Mac app** (macOS, unless `--no-app`): if `cos.app` is in `/Applications` (or `COS_APP_DIR`) or `~/Applications`, and its `CFBundleShortVersionString` isn't the new version, it runs `install.sh --app` with `COS_APP_DIR` set to that folder. It first notes whether `cos-desktop` is running for you (`pgrep -x -U`), since the installer quits it, and reopens it with `open` afterwards. An app you can't write to is left alone, and you get the `sudo` command.
5. **The daemon** (unless `--no-restart`): if it answers on the socket with an older version and the installed plist or unit runs this binary, it is restarted with `launchctl kickstart -k gui/<uid>/local.cos.daemon` or `systemctl --user restart cos.service`, and `cos upgrade` waits up to 20 s for `/v1/status` to report the new version. A restart interrupts agents that aren't `idle`, `exited` or `interrupted`, so it lists them and asks first. `--yes` skips the question; with no terminal, it prints the command instead. A service that runs another binary, or a daemon that isn't a service, is left running, with a note. The same check runs when the CLI is already current, so `cos upgrade` after a manual `install.sh` restarts the daemon too.

`cos upgrade --host` is refused: run it on that machine.

For tests, `COS_DOWNLOAD_BASE` points at a release folder, as for `install.sh`. A `file://` folder has no redirect, so the latest tag is read from `<base>/latest/tag`, which `scripts/fake-release.sh` writes. `COS_INSTALL_SCRIPT` replaces the tag's `install.sh` with a path or URL. The Rust tests (`crates/cos/tests/upgrade.rs`, in `make test`) upgrade a copy of `cos` against such a folder, and `make install-test` runs `cos upgrade` from the released Linux builds. Only a real release shows that GitHub's redirect and the tag's `install.sh` are where `cos upgrade` expects them.

## npm

The `cos-agent` package (`npm/`) is a small installer with no dependencies. `npx cos-agent@latest` installs or upgrades `cos` in `~/.local/bin`, `npx cos-agent app` installs the Mac app as above, and `npx cos-agent <args>` runs `cos <args>`, installing it first if needed. It never runs cos from npx's cache, because `cos daemon install` records the binary's path.

To publish it, set it up once:

1. Create an npm account, and an automation access token that can publish.
2. Add the token as the repo secret `NPM_TOKEN`: `gh secret set NPM_TOKEN`.

The next tagged release then publishes it. The package's tests run in CI and with `make npm-test`.

## Checking before a release

```sh
make lint test          # what CI runs
make linux linux-smoke  # static binaries, run on Amazon Linux 2023 and Debian, both arches
make install-test       # install.sh against a fake release made from dist/
make app-install-test   # install.sh --app on this Mac, against a fake release
make npm-test shellcheck actionlint
make release-local      # every release asset, into dist/release
```

`make test` includes `cos upgrade` against a fake release. `make install-test` also runs `cos upgrade` from the Linux builds: up to date, `--check`, and a tampered release.

`make app-install-test` runs `scripts/test-install-app.sh` natively, since it needs `ditto`, `xattr` and `pgrep`. It builds a tiny fake `cos.app` and CLI, lays them out as a release, and points `HOME`, `COS_APP_DIR` and `COS_DOWNLOAD_BASE` into `target/app-install-test`. It covers an install, an upgrade (same folder, old files gone, quarantine cleared), a user-owned app in a read-only folder, the `~/Applications` fallbacks, a checksum mismatch, `--with-cli`, and quitting a running app. A process with another name plays the running app, so the real one is never touched.

`make release-local` needs Docker, the Tauri CLI (`cargo install tauri-cli --version "^2" --locked`) and both Mac targets (`rustup target add aarch64-apple-darwin x86_64-apple-darwin`). It writes the five assets and `SHA256SUMS` to `dist/release`, giving the app the workspace version without changing a file in the repo. To try the real app zip with the installer without touching `/Applications` or the running app:

```sh
mkdir -p target/local-release/latest target/local-apps && ln -sfn ../../../dist/release target/local-release/latest/download
COS_DOWNLOAD_BASE=file://$PWD/target/local-release COS_APP_DIR=$PWD/target/local-apps COS_APP_NO_QUIT=1 \
  sh install.sh --app
```

Only a real tagged release shows that the `desktop` job works on GitHub's runner (corepack, the Tauri CLI from npm, the build time), that `releases/latest/download/cos-desktop-universal-apple-darwin.zip` redirects to the new asset, and that the released app opens on a Mac that never built it, Intel included.
