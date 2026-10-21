# Tests, lints and type checks run in Docker. Only `build` and `try` run on the
# Mac, because cos has to drive the local `claude` binary.

DEV_IMAGE := cos-dev:1.99
DOCKER := docker run --rm -t \
	-v "$(CURDIR)":/work -w /work \
	-v cos-cargo-registry:/usr/local/cargo/registry \
	-e CARGO_TARGET_DIR=/work/target/docker \
	$(DEV_IMAGE)

.PHONY: image check test lint fmt build try

image:
	docker build -q -t $(DEV_IMAGE) -f docker/dev.Dockerfile docker >/dev/null

check: lint test

test: image
	$(DOCKER) cargo test --workspace

lint: image
	$(DOCKER) sh -c 'cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings'

fmt: image
	$(DOCKER) cargo fmt --all

build:
	cargo build --release

# Example: make try PROMPT="list the files here" ARGS="--model haiku"
try: build
	./target/release/cos try $(ARGS) "$(PROMPT)"
