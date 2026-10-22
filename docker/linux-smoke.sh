#!/bin/sh
# Smoke-tests a static `cos` binary on a stock distro image: the version, the
# daemon, a project and a workspace on a throwaway repo. `make linux-smoke`
# runs it in amazonlinux:2023 and debian:bookworm-slim for each arch, with
# dist/ mounted at /dist.
#
#   linux-smoke.sh /dist/cos-<version>-<target>
set -eu

binary="$1"
say() { printf '  [%s] %s\n' "$(uname -m)" "$*"; }

# Stock images come without git, which cos needs for projects.
if ! command -v git >/dev/null 2>&1; then
    if command -v dnf >/dev/null 2>&1; then
        dnf install -y -q git-core >/dev/null
    else
        apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends git >/dev/null
    fi
fi

mkdir -p /tmp/bin
cp "$binary" /tmp/bin/cos
export PATH="/tmp/bin:$PATH" COS_HOME=/tmp/h HOME=/tmp/home
mkdir -p "$HOME"

say "$(. /etc/os-release && echo "$PRETTY_NAME")"
say "$(cos --version)"

cos daemon >/tmp/daemon.log 2>&1 &
daemon=$!
stop() { kill "$daemon" 2>/dev/null || true; }
trap stop EXIT

i=0
until cos status >/dev/null 2>&1; do
    i=$((i + 1))
    if [ "$i" -ge 100 ] || ! kill -0 "$daemon" 2>/dev/null; then
        echo "the daemon didn't come up:" >&2
        cat /tmp/daemon.log >&2
        exit 1
    fi
    sleep 0.1
done
cos status

repo=/tmp/demo
git init -q -b main "$repo"
git -C "$repo" -c user.name=cos -c user.email=cos@example.com commit -q --allow-empty -m init
cos project add "$repo"
cos workspace new demo --from head
cos project list

# No Tailscale here: the listener reports why instead of failing.
cos remote on || true
cos sessions
# No systemd in a container: install explains instead of half-installing.
if cos daemon install 2>/tmp/install.err; then
    echo "cos daemon install should fail without systemd" >&2
    exit 1
fi
say "daemon install: $(cat /tmp/install.err)"

kill "$daemon"
wait "$daemon" || true
trap - EXIT
if grep -q "panicked" /tmp/daemon.log; then
    cat /tmp/daemon.log >&2
    exit 1
fi
say "ok"
