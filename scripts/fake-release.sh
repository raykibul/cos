#!/bin/sh
# Lays out dist/'s Linux binaries as GitHub would serve a release, for
# testing install.sh offline (`make install-test`):
#
#   <out>/latest/download/{cos-<target>.tar.gz,SHA256SUMS}
#   <out>/download/v<version>/…        the same files
#   <out>/download/v0.0.0-bad/…        with a wrong checksum
#
#   fake-release.sh <version> <out> <target>...
set -eu

version="$1"
out="$2"
shift 2
here="$(dirname "$0")"

rm -rf "$out"
latest="$out/latest/download"
mkdir -p "$latest"
for target in "$@"; do
    sh "$here/package-release.sh" "dist/cos-$version-$target" "$target" "$latest" >/dev/null
done
(cd "$latest" && sha256sum cos-*.tar.gz >SHA256SUMS)

mkdir -p "$out/download"
cp -R "$latest" "$out/download/v$version"
cp -R "$latest" "$out/download/v0.0.0-bad"
sed 's/^./x/' "$latest/SHA256SUMS" >"$out/download/v0.0.0-bad/SHA256SUMS"
find "$out" -type f | sort
