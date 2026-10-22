# Toolchain for `make linux`: static musl builds of `cos` for x86_64 and
# aarch64 Linux, cross-compiled from either arch with cargo-zigbuild. Zig is
# the C compiler and linker, so the bundled SQLite builds for both targets.
FROM rust:1.99

ARG ZIG_VERSION=0.15.2
ARG ZIG_SHA256_AARCH64=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f
ARG ZIG_SHA256_X86_64=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
ARG CARGO_ZIGBUILD_VERSION=0.23.4

RUN apt-get update \
    && apt-get install -y --no-install-recommends file xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN set -eu; \
    arch="$(uname -m)"; \
    case "$arch" in \
      aarch64) sha="$ZIG_SHA256_AARCH64" ;; \
      x86_64) sha="$ZIG_SHA256_X86_64" ;; \
      *) echo "no Zig checksum for $arch" >&2; exit 1 ;; \
    esac; \
    curl -sSfL -o /tmp/zig.tar.xz "https://ziglang.org/download/${ZIG_VERSION}/zig-${arch}-linux-${ZIG_VERSION}.tar.xz"; \
    echo "$sha  /tmp/zig.tar.xz" | sha256sum -c -; \
    mkdir -p /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    rm /tmp/zig.tar.xz; \
    ln -s /opt/zig/zig /usr/local/bin/zig

RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
    && cargo install --locked cargo-zigbuild --version "$CARGO_ZIGBUILD_VERSION" \
    && rm -rf /usr/local/cargo/registry
