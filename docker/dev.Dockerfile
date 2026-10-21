# Toolchain for `make test` and `make lint`. The official image ships without
# rustfmt and clippy.
FROM rust:1.99
RUN rustup component add rustfmt clippy
