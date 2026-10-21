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

# The desktop UI (clients/desktop). Every Node command runs in Docker:
# node_modules lives in a volume (one per checkout) mounted over
# clients/desktop/node_modules, so Linux binaries never land on the Mac.
NODE_IMAGE := cos-node:22
UI_DIR := clients/desktop
UI_MODULES := cos-ui-modules-$(shell printf '%s' "$(CURDIR)" | cksum | cut -d' ' -f1)
NODE_RUN = docker run --rm --init \
	-v "$(CURDIR)":/work -w /work/$(UI_DIR) \
	-v $(UI_MODULES):/work/$(UI_DIR)/node_modules \
	-v cos-pnpm-store:/pnpm-store
NODE := $(NODE_RUN) -t $(NODE_IMAGE)
PLAYWRIGHT_IMAGE := mcr.microsoft.com/playwright:v1.64.1-noble

.PHONY: node-image ui-install ui-test ui-lint ui-format ui-typecheck ui-build ui-dev ui-shots

node-image:
	docker build -q -t $(NODE_IMAGE) -f docker/node.Dockerfile docker >/dev/null

ui-install: node-image
	$(NODE) pnpm install --no-frozen-lockfile

ui-test: ui-install
	$(NODE) pnpm test

ui-lint: ui-install
	$(NODE) pnpm lint

ui-format: ui-install
	$(NODE) pnpm format

ui-typecheck: ui-install
	$(NODE) pnpm typecheck

ui-build: ui-install
	$(NODE) pnpm build

# Vite's dev server on localhost:5173. In a plain browser the UI uses the
# fake bridge.
ui-dev: ui-install
	$(NODE_RUN) -it -p 5173:5173 $(NODE_IMAGE) pnpm dev

# Screenshots of every screen against the fake bridge, light and dark, into
# .sandbox/desktop-shots (headless Chromium in Playwright's image).
ui-shots: ui-build
	$(NODE_RUN) -t -e SHOTS_DIR=/work/.sandbox/desktop-shots $(PLAYWRIGHT_IMAGE) node scripts/shots.mjs
