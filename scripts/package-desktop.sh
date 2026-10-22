#!/bin/sh
# Packs a built cos.app as the release asset
# cos-desktop-universal-apple-darwin.zip, after checking that it's the
# version being released, has both arm64 and x86_64 code, and has a valid
# (ad-hoc) signature. macOS only (lipo, codesign, ditto).
#
#   scripts/package-desktop.sh <cos.app> <version> <out dir>
set -eu

app="$1"
version="$2"
out="$3"
asset=cos-desktop-universal-apple-darwin.zip

fail() {
    printf 'package-desktop: error: %s\n' "$*" >&2
    exit 1
}

[ -d "$app/Contents" ] || fail "$app is not an app bundle"
[ "$(basename "$app")" = cos.app ] || fail "$app should be named cos.app"

plist="$app/Contents/Info.plist"
bundled="$(plutil -extract CFBundleShortVersionString raw -o - "$plist")"
[ "$bundled" = "$version" ] || fail "$app is version $bundled, not $version"
id="$(plutil -extract CFBundleIdentifier raw -o - "$plist")"
[ "$id" = local.cos.desktop ] || fail "$app has bundle id $id, not local.cos.desktop"

binary="$app/Contents/MacOS/cos-desktop"
archs="$(lipo -archs "$binary")"
for arch in x86_64 arm64; do
    case " $archs " in
        *" $arch "*) ;;
        *) fail "$binary has $archs; the release needs both x86_64 and arm64" ;;
    esac
done
codesign --verify --deep --strict "$app" || fail "$app's signature doesn't verify"

mkdir -p "$out"
rm -f "$out/$asset"
# --keepParent puts cos.app itself at the top of the zip. ditto keeps the
# bundle's symlinks, permissions and signature intact.
ditto -c -k --keepParent "$app" "$out/$asset"
echo "$out/$asset ($version, $archs)"
