#!/bin/sh
# Installs or upgrades cos from its GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh -s -- --app
#
# Without options it installs the `cos` CLI: the build for this OS and CPU
# (Linux or macOS, x86_64 or arm64), checked against the release's
# SHA256SUMS, in ~/.local/bin. Running it again upgrades. Options:
#
#   --app        install the Mac app instead (macOS 13 or later): cos.app in
#                /Applications, or ~/Applications when /Applications isn't
#                writable. A running copy is quit first.
#   --with-cli   with --app, install the CLI as well
#
# Settings, all optional:
#
#   COS_VERSION=v0.2.0          a release tag instead of the latest
#   COS_INSTALL_DIR=/some/bin   where to put `cos` (default ~/.local/bin)
#   COS_APP_DIR=/some/dir       where to put cos.app (default /Applications,
#                               falling back to ~/Applications)
#   COS_APP_NO_QUIT=1           don't quit a running app (tests set it)
#   COS_NO_SERVICE_HINT=1       don't print how to restart the daemon's
#                               service (`cos upgrade` sets it, and restarts
#                               the service itself)
#   COS_DOWNLOAD_BASE=URL       where releases live (default
#                               https://github.com/raykibul/cos/releases);
#                               tests point it at a local folder (file://)
#
# It never restarts a running daemon: if `cos daemon install` set one up, it
# prints the command to restart it on the new version. Once cos is
# installed, `cos upgrade` runs this script for you and restarts the daemon.
set -eu

base="${COS_DOWNLOAD_BASE:-https://github.com/raykibul/cos/releases}"
version="${COS_VERSION:-latest}"
install_dir="${COS_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"
# The app is one universal (arm64 and x86_64) build, so its name has no CPU.
app_asset=cos-desktop-universal-apple-darwin.zip
app_system_dir="${COS_APP_DIR:-/Applications}"
app_user_dir="$HOME/Applications"
# The app's executable, as pgrep sees it. Tests point it at a fake process.
app_process="${COS_APP_PROCESS:-cos-desktop}"
one_liner="curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh"

say() { printf 'cos install: %s\n' "$*"; }
die() {
    printf 'cos install: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Installs or upgrades cos from its GitHub releases.

  $one_liner | sh
  $one_liner | sh -s -- --app [--with-cli]

  (no options)  the cos CLI, in ~/.local/bin
  --app         the Mac app, in /Applications (or ~/Applications)
  --with-cli    with --app, the CLI as well

Settings: COS_VERSION, COS_INSTALL_DIR, COS_APP_DIR, COS_APP_NO_QUIT,
COS_NO_SERVICE_HINT, COS_DOWNLOAD_BASE. See the top of install.sh.
Once cos is installed, \`cos upgrade\` upgrades it (and the app) for you.
EOF
}

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "no cos build for the $arch CPU; build it from source (see the README)" ;;
    esac
    case "$os" in
        Linux) echo "$arch-unknown-linux-musl" ;;
        Darwin)
            # A shell running under Rosetta reports x86_64 on an Apple
            # Silicon Mac; the native build is the better one there.
            if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
                arch=aarch64
            fi
            echo "$arch-apple-darwin"
            ;;
        *) die "no cos build for $os; cos runs on Linux and macOS" ;;
    esac
}

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 3 -o "$2" "$1" || die "could not download $1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1" || die "could not download $1"
    else
        die "needs curl or wget"
    fi
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "needs sha256sum or shasum to check the download"
    fi
}

# fetch <asset>: downloads the release's <asset> into $tmp and checks it
# against the release's SHA256SUMS.
fetch() {
    say "downloading $1 ($version)"
    download "$url/$1" "$tmp/$1"
    [ -f "$tmp/SHA256SUMS" ] || download "$url/SHA256SUMS" "$tmp/SHA256SUMS"

    expected="$(awk -v name="$1" '$2 == name || $2 == "*" name { print $1 }' "$tmp/SHA256SUMS")"
    [ -n "$expected" ] || die "SHA256SUMS has no entry for $1"
    actual="$(sha256 "$tmp/$1")"
    [ "$actual" = "$expected" ] || die "checksum mismatch for $1: expected $expected, got $actual"
}

# Prints how to restart the daemon's service on the new binary, if
# `cos daemon install` set one up.
service_hint() {
    [ "${COS_NO_SERVICE_HINT:-}" != 1 ] || return 0
    case "$(uname -s)" in
        Linux)
            unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/cos.service"
            restart="systemctl --user restart cos.service"
            ;;
        Darwin)
            unit="$HOME/Library/LaunchAgents/local.cos.daemon.plist"
            restart="launchctl kickstart -k gui/$(id -u)/local.cos.daemon"
            ;;
        *) return 0 ;;
    esac
    [ -f "$unit" ] || return 0
    if grep -qF "$1" "$unit"; then
        say "the cos daemon service still runs the old version until it restarts. Restart it (this interrupts running agents; they can be resumed):"
        say "  $restart"
    else
        say "a cos daemon service is installed ($unit) but runs another cos binary."
        say "  to run this one instead: $1 daemon install"
    fi
}

install_cli() {
    target="$(detect_target)"
    asset="cos-$target.tar.gz"
    command -v tar >/dev/null 2>&1 || die "needs tar to unpack the download"

    fetch "$asset"
    tar -xzf "$tmp/$asset" -C "$tmp" cos || die "could not unpack $asset"
    mkdir -p "$install_dir"
    # Copy next to the target, then rename over it: a running `cos` keeps
    # its old file, and nobody sees a half-written one.
    staged="$install_dir/.cos.new.$$"
    cp "$tmp/cos" "$staged"
    chmod 755 "$staged"
    mv -f "$staged" "$install_dir/cos"

    installed="$("$install_dir/cos" --version 2>/dev/null)" || die "$install_dir/cos doesn't run on this machine"
    say "installed $installed at $install_dir/cos"

    case ":${PATH:-}:" in
        *":$install_dir:"*) ;;
        *)
            # Printed for the user to run, so $HOME stays unexpanded.
            case "${SHELL:-}" in
                */zsh) rc="\$HOME/.zshrc" ;;
                */bash) rc="\$HOME/.bashrc" ;;
                *) rc="\$HOME/.profile" ;;
            esac
            say "$install_dir is not on your PATH. Add it, for example:"
            say "  echo 'export PATH=\"$install_dir:\$PATH\"' >> $rc && . $rc"
            ;;
    esac
    service_hint "$install_dir/cos"
}

# app_writable <dir>: whether this user can put cos.app in <dir>. An existing
# cos.app is replaced from the inside (its Contents), so it only needs the
# app folder to be writable, not <dir>: that's how a user-owned
# /Applications/cos.app upgrades without sudo.
app_writable() {
    if [ -e "$1/cos.app" ]; then
        [ -w "$1/cos.app" ] && { [ ! -e "$1/cos.app/Contents" ] || [ -w "$1/cos.app/Contents" ]; }
    else
        [ -d "$1" ] && [ -w "$1" ]
    fi
}

# Sets $app_dir: /Applications (or $COS_APP_DIR) when it's writable, else
# ~/Applications. A copy already in ~/Applications, with none in
# /Applications, is upgraded where it is.
pick_app_dir() {
    if [ ! -e "$app_system_dir/cos.app" ] && [ -e "$app_user_dir/cos.app" ]; then
        app_dir="$app_user_dir"
    elif app_writable "$app_system_dir"; then
        app_dir="$app_system_dir"
    else
        app_dir="$app_user_dir"
        mkdir -p "$app_dir"
        if [ -e "$app_system_dir/cos.app" ]; then
            say "$app_system_dir/cos.app isn't writable by you, so installing in $app_dir instead."
            say "the old copy in $app_system_dir stays, and Spotlight may still open it. To replace that one instead, run:"
        else
            say "$app_system_dir isn't writable by you, so installing in $app_dir instead. To install in $app_system_dir, run:"
        fi
        say "  $one_liner | sudo sh -s -- --app"
    fi
}

# Quits a running copy of the app, by pid, and waits for it to exit.
quit_app() {
    was_running=
    if [ "${COS_APP_NO_QUIT:-}" = 1 ]; then
        return 0
    fi
    # Under sudo, the app runs as the user who ran sudo.
    pids="$(pgrep -x -U "${SUDO_UID:-$(id -u)}" "$app_process" || true)"
    [ -n "$pids" ] || return 0
    for pid in $pids; do
        say "quitting the running cos app (pid $pid)"
        kill "$pid" 2>/dev/null || true
    done
    for pid in $pids; do
        tries=0
        while kill -0 "$pid" 2>/dev/null; do
            tries=$((tries + 1))
            [ "$tries" -le 50 ] || die "the cos app (pid $pid) didn't quit; quit it from its menu bar icon and run this again"
            sleep 0.2
        done
    done
    was_running=1
}

install_app() {
    [ "$(uname -s)" = Darwin ] ||
        die "the cos app is for macOS. On $(uname -s), install the CLI (without --app) and use the app on a Mac, over Tailscale"
    major="$(sw_vers -productVersion 2>/dev/null | cut -d. -f1)"
    [ "${major:-0}" -ge 13 ] 2>/dev/null || die "the cos app needs macOS 13 or later"

    fetch "$app_asset"
    mkdir "$tmp/app"
    ditto -x -k "$tmp/$app_asset" "$tmp/app" || die "could not unpack $app_asset"
    [ -d "$tmp/app/cos.app/Contents" ] || die "$app_asset has no cos.app"

    pick_app_dir
    app="$app_dir/cos.app"
    quit_app
    # Replace what's inside the app folder rather than the folder itself:
    # the folder may be ours inside an /Applications we can't write to.
    if [ -e "$app/Contents" ]; then
        rm -rf "$app/Contents" || die "could not remove the old $app/Contents; run the installer with sudo"
    fi
    ditto "$tmp/app/cos.app" "$app" || die "could not copy cos.app to $app"
    # The app is ad-hoc signed, not notarized: with the quarantine flag,
    # which browsers (and archives they unpack) set, Gatekeeper refuses to
    # open it. See "The Mac app" in docs/releasing.md.
    xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
    # Under sudo, keep the app the user's, so later upgrades need no sudo.
    if [ "$(id -u)" = 0 ] && [ -n "${SUDO_UID:-}" ]; then
        chown -R "$SUDO_UID" "$app"
    fi
    touch "$app"

    app_version="$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist" 2>/dev/null || echo "?")"
    say "installed the cos app $app_version at $app"
    if [ -n "$was_running" ]; then
        say "it was running; open it again with: open '$app'"
    else
        say "open it with: open '$app'"
    fi
    if [ -z "$with_cli" ] && ! command -v cos >/dev/null 2>&1 && [ ! -x "$install_dir/cos" ]; then
        say "the app drives the cos daemon. To install the CLI too, add --with-cli, then run: cos daemon install"
    fi
}

main() {
    want_app=
    with_cli=
    for arg in "$@"; do
        case "$arg" in
            --app) want_app=1 ;;
            --with-cli) with_cli=1 ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) die "unknown option $arg (see --help)" ;;
        esac
    done
    [ -z "$with_cli" ] || [ -n "$want_app" ] || die "--with-cli goes with --app; without --app, the CLI is what's installed"

    case "$version" in
        latest) url="$base/latest/download" ;;
        v*) url="$base/download/$version" ;;
        *) url="$base/download/v$version" ;;
    esac
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    if [ -n "$want_app" ]; then
        install_app
        [ -z "$with_cli" ] || install_cli
    else
        install_cli
    fi
}

main "$@"
