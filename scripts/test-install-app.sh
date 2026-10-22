#!/bin/sh
# Tests `install.sh --app` on this Mac against a fake release made here: a
# tiny cos.app (no real binary) zipped as the release zips it, and a fake
# `cos` CLI. `make app-install-test` runs it. Everything happens under
# <scratch> (default target/app-install-test): HOME, the "/Applications"
# (COS_APP_DIR) and the release. It never quits the real app: the quit step
# is skipped (COS_APP_NO_QUIT=1), except in one test where it looks for a
# fake process with another name.
#
#   scripts/test-install-app.sh [scratch dir]
set -eu

[ "$(uname -s)" = Darwin ] || {
    echo "test-install-app.sh runs on macOS" >&2
    exit 1
}

repo="$(cd "$(dirname "$0")/.." && pwd)"
root="${1:-$repo/target/app-install-test}"
mkdir -p "$root"
root="$(cd "$root" && pwd)"
case "$root" in
    /Applications* | "$HOME" | /) echo "refusing to use $root as scratch" >&2 && exit 1 ;;
esac
asset=cos-desktop-universal-apple-darwin.zip

say() { printf '  %s\n' "$*"; }
fail() {
    printf '  FAIL: %s\n' "$*" >&2
    [ ! -f "$root/out" ] || sed 's/^/    | /' "$root/out" >&2
    exit 1
}

# Undo a read-only folder from a failed run before removing it.
[ ! -d "$root" ] || chmod -R u+w "$root"
rm -rf "$root"
mkdir -p "$root/home" "$root/Applications" "$root/release" "$root/build"

# A fake cos.app at <version>. Version 0.1.0 has an extra file, so an
# upgrade shows that old contents go. The quarantine flag inside the zip
# stands in for one a browser would add.
fake_app() {
    app="$root/build/$1/cos.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>cos-desktop</string>
  <key>CFBundleIdentifier</key><string>local.cos.desktop</string>
  <key>CFBundleShortVersionString</key><string>$1</string>
</dict>
</plist>
EOF
    printf '#!/bin/sh\necho "cos-desktop %s"\n' "$1" >"$app/Contents/MacOS/cos-desktop"
    chmod 755 "$app/Contents/MacOS/cos-desktop"
    [ "$1" != 0.1.0 ] || echo old >"$app/Contents/Resources/only-in-0.1.0"
    xattr -w com.apple.quarantine "0081;00000000;Safari;" "$app/Contents/MacOS/cos-desktop"
    echo "$app"
}

# release <dir> <version>: the app zip, a fake CLI for both Mac CPUs and
# SHA256SUMS, as the release workflow lays them out.
release() {
    dir="$root/release/$1"
    mkdir -p "$dir"
    ditto -c -k --keepParent "$(fake_app "$2")" "$dir/$asset"
    cli="$root/build/$2/cli"
    mkdir -p "$cli"
    printf '#!/bin/sh\necho "cos %s"\n' "$2" >"$cli/cos"
    chmod 755 "$cli/cos"
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        COPYFILE_DISABLE=1 tar -czf "$dir/cos-$target.tar.gz" -C "$cli" cos
    done
    (cd "$dir" && shasum -a 256 cos-*.tar.gz "$asset" >SHA256SUMS)
}
release latest/download 0.2.0
release download/v0.1.0 0.1.0
release download/v0.0.0-bad 0.1.0
sed 's/^./0/' "$root/release/download/v0.0.0-bad/SHA256SUMS" >"$root/sums" &&
    mv "$root/sums" "$root/release/download/v0.0.0-bad/SHA256SUMS"

export HOME="$root/home" COS_APP_DIR="$root/Applications" COS_APP_NO_QUIT=1 \
    COS_DOWNLOAD_BASE="file://$root/release" COS_APP_PROCESS=cos-test-none
unset COS_VERSION COS_INSTALL_DIR
system_app="$root/Applications/cos.app"
user_app="$HOME/Applications/cos.app"

# install [NAME=value...] [option...]: runs install.sh with those settings.
# (A `NAME=value install` prefix would outlive the call in sh.)
install() {
    (
        while [ $# -gt 0 ]; do
            case "$1" in
                *=*) export "${1?}" && shift ;;
                *) break ;;
            esac
        done
        exec sh "$repo/install.sh" "$@"
    ) >"$root/out" 2>&1
}
version_of() { plutil -extract CFBundleShortVersionString raw -o - "$1/Contents/Info.plist"; }
installed_ok() { # <app> <version>
    [ "$(version_of "$1")" = "$2" ] || fail "$1 is $(version_of "$1"), expected $2"
    "$1/Contents/MacOS/cos-desktop" | grep -qx "cos-desktop $2" || fail "$1 doesn't run"
    if xattr -r "$1" | grep -q com.apple.quarantine; then fail "$1 is still quarantined"; fi
}

# Install, pinned to 0.1.0, through the one-liner's `sh -s -- --app`.
COS_VERSION=v0.1.0 sh -s -- --app <"$repo/install.sh" >"$root/out" 2>&1 || fail "install from stdin"
installed_ok "$system_app" 0.1.0
[ -f "$system_app/Contents/Resources/only-in-0.1.0" ] || fail "0.1.0's file is missing"
grep -q "installed the cos app 0.1.0 at $system_app" "$root/out" || fail "no installed message"
grep -q "add --with-cli" "$root/out" || fail "no hint about the CLI"
[ ! -e "$HOME/.local/bin/cos" ] || fail "--app installed the CLI"
say "install 0.1.0 into COS_APP_DIR: ok, quarantine cleared"

# Upgrade to the latest: the same folder, only the new contents.
inode="$(stat -f %i "$system_app")"
install --app || fail "upgrade"
installed_ok "$system_app" 0.2.0
[ ! -e "$system_app/Contents/Resources/only-in-0.1.0" ] || fail "the upgrade left 0.1.0's file"
[ "$(stat -f %i "$system_app")" = "$inode" ] || fail "the upgrade replaced the app folder"
say "upgrade to latest (0.2.0): ok, same folder, old contents gone"

# A user-owned cos.app inside an Applications folder the user can't write
# to (a standard user's /Applications) upgrades in place, without sudo.
chmod 555 "$root/Applications"
install COS_VERSION=0.1.0 --app || { chmod 755 "$root/Applications"; fail "in-place upgrade in a read-only folder"; }
chmod 755 "$root/Applications"
installed_ok "$system_app" 0.1.0
[ ! -e "$user_app" ] || fail "installed in ~/Applications although cos.app was writable"
if grep -q sudo "$root/out"; then fail "asked for sudo needlessly"; fi
say "user-owned cos.app in a read-only Applications: upgraded in place"

# The same cos.app owned by someone else (read-only to us): ~/Applications,
# a warning that the old copy stays, and the sudo command.
chmod 555 "$system_app"
install --app || { chmod 755 "$system_app"; fail "fallback with a read-only cos.app"; }
chmod 755 "$system_app"
installed_ok "$user_app" 0.2.0
[ "$(version_of "$system_app")" = 0.1.0 ] || fail "the read-only copy changed"
grep -q "the old copy in $root/Applications stays" "$root/out" || fail "no warning about the old copy"
grep -q "| sudo sh -s -- --app" "$root/out" || fail "no sudo hint"
say "read-only cos.app: installed in ~/Applications, with the sudo hint"

# No cos.app and a read-only Applications: ~/Applications, with the hint.
rm -rf "$system_app" "$user_app"
chmod 555 "$root/Applications"
install --app || { chmod 755 "$root/Applications"; fail "fallback"; }
chmod 755 "$root/Applications"
installed_ok "$user_app" 0.2.0
[ ! -e "$system_app" ] || fail "installed in the read-only folder"
grep -q "isn't writable by you, so installing in $HOME/Applications" "$root/out" || fail "no fallback message"
grep -q "| sudo sh -s -- --app" "$root/out" || fail "no sudo hint"
say "read-only Applications: installed in ~/Applications, with the sudo hint"

# Next time, with Applications writable again, the copy in ~/Applications
# is upgraded rather than a second one made.
install COS_VERSION=v0.1.0 --app || fail "upgrade in ~/Applications"
installed_ok "$user_app" 0.1.0
[ ! -e "$system_app" ] || fail "made a second copy in Applications"
rm -rf "$user_app"
say "a copy in ~/Applications is upgraded where it is"

# A tampered release is refused and leaves the app alone.
install --app || fail "install before the checksum test"
before="$(shasum -a 256 "$system_app/Contents/Info.plist")"
if install COS_VERSION=v0.0.0-bad --app; then fail "a checksum mismatch was accepted"; fi
grep -q "checksum mismatch for $asset" "$root/out" || fail "no checksum message"
[ "$(shasum -a 256 "$system_app/Contents/Info.plist")" = "$before" ] || fail "a failed install changed the app"
say "tampered release: refused, app untouched"

# A running app is quit by its pid first. A process with another name
# stands in for it, so the real app is never touched. (A renamed copy of
# /bin/sleep is killed by code signing, so it's built here.)
printf '#include <unistd.h>\nint main(void) { sleep(60); return 0; }\n' |
    cc -x c -o "$root/build/cosfakeapp" - || fail "cc couldn't build the fake app process"
# Started from another shell, so this one doesn't report its end.
pid="$(sh -c '"$1" >/dev/null 2>&1 & echo $!' sh "$root/build/cosfakeapp")"
sleep 0.5
kill -0 "$pid" 2>/dev/null || fail "the fake app process didn't start"
install COS_APP_NO_QUIT='' COS_APP_PROCESS=cosfakeapp --app || fail "install with the app running"
if kill -0 "$pid" 2>/dev/null; then
    kill "$pid"
    fail "the running app wasn't quit"
fi
grep -q "quitting the running cos app (pid $pid)" "$root/out" || fail "no quit message"
grep -q "it was running; open it again" "$root/out" || fail "no reopen hint"
say "running app: quit by pid ($pid), then replaced"

# --with-cli installs the CLI as well.
install --app --with-cli || fail "--with-cli"
"$HOME/.local/bin/cos" --version | grep -qx "cos 0.2.0" || fail "--with-cli didn't install cos"
grep -q "installed cos 0.2.0 at $HOME/.local/bin/cos" "$root/out" || fail "no CLI message"
say "--app --with-cli: app and CLI"

# Without options, still the CLI only.
rm -rf "$HOME/.local/bin"
install COS_VERSION=v0.1.0 || fail "CLI install"
"$HOME/.local/bin/cos" --version | grep -qx "cos 0.1.0" || fail "CLI version"
[ "$(version_of "$system_app")" = 0.2.0 ] || fail "a CLI install touched the app"
say "no options: the CLI only"

# Bad options.
if install --with-cli; then fail "--with-cli without --app was accepted"; fi
grep -q "goes with --app" "$root/out" || fail "no --with-cli message"
if install --bogus; then fail "an unknown option was accepted"; fi
grep -q "unknown option --bogus" "$root/out" || fail "no unknown-option message"
install --help || fail "--help"
grep -q -- "--with-cli" "$root/out" || fail "--help doesn't mention --with-cli"
say "bad options refused, --help works"
say "ok"
