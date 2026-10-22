#!/bin/sh
# Builds static `cos` binaries for Linux into dist/. Runs inside the
# cos-linux image (docker/linux.Dockerfile); `make linux` calls it.
#
#   dist/cos-<version>-<target>          the binary
#   dist/cos-<version>-<target>.tar.gz   the binary as `cos`
#   dist/SHA256SUMS
set -eu

version="$1"
shift
targets="${*:-x86_64-unknown-linux-musl aarch64-unknown-linux-musl}"
dist=dist

mkdir -p "$dist"
for target in $targets; do
    echo "==> cos $version for $target"
    CARGO_PROFILE_RELEASE_STRIP=symbols \
        cargo zigbuild --release --locked -p cos --bin cos --target "$target"
    built="$CARGO_TARGET_DIR/$target/release/cos"

    description="$(file -b "$built")"
    echo "    $description"
    case "$description" in
        *"statically linked"* | *"static-pie linked"*) ;;
        *) echo "error: $built is not statically linked" >&2; exit 1 ;;
    esac

    name="cos-$version-$target"
    cp "$built" "$dist/$name"
    staging="$(mktemp -d)"
    cp "$built" "$staging/cos"
    tar -czf "$dist/$name.tar.gz" -C "$staging" --owner=0 --group=0 cos
    rm -r "$staging"
done

cd "$dist"
sha256sum cos-"$version"-* > SHA256SUMS
cat SHA256SUMS
ls -lh cos-"$version"-*
