#!/bin/sh
# Installs or upgrades cos from its GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh
#
# Picks the build for this OS and CPU (Linux or macOS, x86_64 or arm64),
# checks it against the release's SHA256SUMS and puts `cos` in
# ~/.local/bin. Running it again upgrades. Settings, all optional:
#
#   COS_VERSION=v0.2.0          a release tag instead of the latest
#   COS_INSTALL_DIR=/some/bin   where to put `cos` (default ~/.local/bin)
#   COS_DOWNLOAD_BASE=URL       where releases live (default
#                               https://github.com/raykibul/cos/releases);
#                               tests point it at a local folder (file://)
#
# It never restarts a running daemon: if `cos daemon install` set one up, it
# prints the command to restart it on the new version.
set -eu

base="${COS_DOWNLOAD_BASE:-https://github.com/raykibul/cos/releases}"
version="${COS_VERSION:-latest}"
install_dir="${COS_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"

say() { printf 'cos install: %s\n' "$*"; }
die() {
    printf 'cos install: error: %s\n' "$*" >&2
    exit 1
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

# Prints how to restart the daemon's service on the new binary, if
# `cos daemon install` set one up.
service_hint() {
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

main() {
    target="$(detect_target)"
    asset="cos-$target.tar.gz"
    case "$version" in
        latest) url="$base/latest/download" ;;
        v*) url="$base/download/$version" ;;
        *) url="$base/download/v$version" ;;
    esac

    command -v tar >/dev/null 2>&1 || die "needs tar to unpack the download"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "downloading $asset ($version)"
    download "$url/$asset" "$tmp/$asset"
    download "$url/SHA256SUMS" "$tmp/SHA256SUMS"

    expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name { print $1 }' "$tmp/SHA256SUMS")"
    [ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"
    actual="$(sha256 "$tmp/$asset")"
    [ "$actual" = "$expected" ] || die "checksum mismatch for $asset: expected $expected, got $actual"

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

main "$@"
