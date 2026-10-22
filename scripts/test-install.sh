#!/bin/sh
# Tests install.sh on a stock distro image against a fake release folder.
# `make install-test` runs it with the repo at /src and the fake release,
# made from dist/ by scripts/fake-release.sh, at /release.
#
#   test-install.sh <version>
set -eu

version="$1"
say() { printf '  [%s] %s\n' "$(uname -m)" "$*"; }
fail() {
    printf '  [%s] FAIL: %s\n' "$(uname -m)" "$*" >&2
    exit 1
}

# shellcheck source=/dev/null
say "$(. /etc/os-release && echo "$PRETTY_NAME")"
export HOME=/tmp/home COS_DOWNLOAD_BASE=file:///release
mkdir -p "$HOME"
cos="$HOME/.local/bin/cos"

# The amazonlinux image has no tar (EC2's AMI does): a clear error first.
if ! command -v tar >/dev/null 2>&1; then
    if sh /src/install.sh >/tmp/out 2>&1; then
        fail "installed without tar"
    fi
    grep -q "needs tar" /tmp/out || { cat /tmp/out; fail "no tar message"; }
    say "without tar: refused clearly"
    dnf install -y -q tar gzip >/dev/null
fi
# The debian image has no curl.
if ! command -v curl >/dev/null 2>&1; then
    apt-get update -qq >/dev/null
    apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null
fi

# The one-liner: the script on stdin, latest release, default folder.
sh </src/install.sh >/tmp/out 2>&1 || { cat /tmp/out; fail "install from stdin"; }
"$cos" --version | grep -qx "cos $version" || fail "cos --version after install"
grep -q "is not on your PATH" /tmp/out || fail "no PATH hint"
say "installed: $("$cos" --version)"

# Again, with the folder on PATH and a service installed: an upgrade in place.
mkdir -p "$HOME/.config/systemd/user"
printf 'ExecStart="%s" daemon\n' "$cos" >"$HOME/.config/systemd/user/cos.service"
PATH="$HOME/.local/bin:$PATH" sh /src/install.sh >/tmp/out 2>&1 || { cat /tmp/out; fail "reinstall"; }
grep -q "is not on your PATH" /tmp/out && fail "PATH hint with the folder on PATH"
grep -q "systemctl --user restart cos.service" /tmp/out || { cat /tmp/out; fail "no restart hint"; }
say "reinstall: ok, with the service restart hint"

# A service running another binary gets told how to switch.
printf 'ExecStart="/elsewhere/cos" daemon\n' >"$HOME/.config/systemd/user/cos.service"
sh /src/install.sh >/tmp/out 2>&1 || fail "install with another service"
grep -q "runs another cos binary" /tmp/out || { cat /tmp/out; fail "no switch hint"; }
rm "$HOME/.config/systemd/user/cos.service"

# A pinned version, into another folder.
COS_VERSION="v$version" COS_INSTALL_DIR=/tmp/other sh /src/install.sh >/tmp/out 2>&1 ||
    { cat /tmp/out; fail "pinned install"; }
/tmp/other/cos --version | grep -qx "cos $version" || fail "pinned version"
say "COS_VERSION=v$version COS_INSTALL_DIR=/tmp/other: ok"

# A tampered release is refused and leaves the installed binary alone.
before="$(sha256sum "$cos")"
if COS_VERSION=v0.0.0-bad sh /src/install.sh >/tmp/out 2>&1; then
    fail "a checksum mismatch was accepted"
fi
grep -q "checksum mismatch" /tmp/out || { cat /tmp/out; fail "no checksum message"; }
[ "$(sha256sum "$cos")" = "$before" ] || fail "a failed install replaced cos"
say "tampered release: refused"

# The app is for macOS only, and says so before downloading anything.
if sh /src/install.sh --app >/tmp/out 2>&1; then
    fail "--app installed on Linux"
fi
grep -q "the cos app is for macOS" /tmp/out || { cat /tmp/out; fail "no macOS message"; }
grep -q "downloading" /tmp/out && fail "--app downloaded on Linux"
say "--app on Linux: refused"

# A release that doesn't exist.
if COS_VERSION=v9.9.9 sh /src/install.sh >/tmp/out 2>&1; then
    fail "a missing release was accepted"
fi
grep -q "could not download" /tmp/out || { cat /tmp/out; fail "no download message"; }
say "missing release: refused"
say "ok"
