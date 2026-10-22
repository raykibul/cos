#!/bin/sh
# Packs one built `cos` binary as a release asset, `cos-<target>.tar.gz`,
# with `cos` at the top and README.md and LICENSE next to it when the repo
# has them. Asset names carry no version, so
# https://github.com/raykibul/cos/releases/latest/download/cos-<target>.tar.gz
# always names the newest build.
#
#   scripts/package-release.sh <binary> <target> <out dir>
set -eu

binary="$1"
target="$2"
out="$3"

root="$(cd "$(dirname "$0")/.." && pwd)"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

cp "$binary" "$staging/cos"
chmod 755 "$staging/cos"
files=cos
for extra in README.md LICENSE; do
    if [ -f "$root/$extra" ]; then
        cp "$root/$extra" "$staging/"
        files="$files $extra"
    fi
done

mkdir -p "$out"
# macOS tar would otherwise add ._* entries for extended attributes.
export COPYFILE_DISABLE=1
# $files is a list of plain names, split on purpose.
# shellcheck disable=SC2086
tar -czf "$out/cos-$target.tar.gz" -C "$staging" $files
echo "$out/cos-$target.tar.gz"
