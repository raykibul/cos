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

# Static Linux binaries (musl) for x86_64 and aarch64, cross-compiled in
# Docker with cargo-zigbuild, into dist/ with tarballs and SHA256SUMS. See
# docs/linux.md for running the daemon on a Linux server.
LINUX_IMAGE := cos-linux:1.99
LINUX_TARGETS := x86_64-unknown-linux-musl aarch64-unknown-linux-musl
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
SMOKE_IMAGES := amazonlinux:2023 debian:bookworm-slim

.PHONY: linux-image linux linux-smoke

linux-image:
	docker build -q -t $(LINUX_IMAGE) -f docker/linux.Dockerfile docker >/dev/null

linux: linux-image
	docker run --rm -t \
		-v "$(CURDIR)":/work -w /work \
		-v cos-cargo-registry:/usr/local/cargo/registry \
		-v cos-zig-cache:/root/.cache/cargo-zigbuild \
		-e CARGO_TARGET_DIR=/work/target/linux \
		$(LINUX_IMAGE) sh docker/linux-release.sh $(VERSION) $(LINUX_TARGETS)

# Runs each binary in dist/ on stock distro images: the daemon, a project and
# a workspace. The x86_64 one runs under Docker's amd64 emulation.
linux-smoke:
	@for target in $(LINUX_TARGETS); do \
		case $$target in x86_64-*) platform=linux/amd64 ;; aarch64-*) platform=linux/arm64 ;; esac; \
		for image in $(SMOKE_IMAGES); do \
			echo "==> $$target on $$image ($$platform)"; \
			docker run --rm --platform $$platform \
				-v "$(CURDIR)/dist":/dist:ro -v "$(CURDIR)/docker":/smoke:ro \
				$$image sh /smoke/linux-smoke.sh /dist/cos-$(VERSION)-$$target || exit 1; \
		done; \
	done

# install.sh against a fake release made from dist/ (run `make linux` first),
# on the same distro images and both arches.
FAKE_RELEASE := target/fake-release

.PHONY: install-test npm-test shellcheck actionlint

install-test: linux-image
	docker run --rm -v "$(CURDIR)":/work -w /work $(LINUX_IMAGE) \
		sh scripts/fake-release.sh $(VERSION) $(FAKE_RELEASE) $(LINUX_TARGETS) >/dev/null
	@for platform in linux/arm64 linux/amd64; do \
		for image in $(SMOKE_IMAGES); do \
			echo "==> install.sh on $$image ($$platform)"; \
			docker run --rm --platform $$platform \
				-v "$(CURDIR)":/src:ro -v "$(CURDIR)/$(FAKE_RELEASE)":/release:ro \
				$$image sh /src/scripts/test-install.sh $(VERSION) || exit 1; \
		done; \
	done

# The npm installer's tests (npm/), in Node's image.
npm-test:
	docker run --rm -v "$(CURDIR)/npm":/pkg:ro -w /pkg node:22 node --test

shellcheck:
	docker run --rm -v "$(CURDIR)":/mnt:ro -w /mnt koalaman/shellcheck:v0.10.0 \
		install.sh scripts/*.sh docker/*.sh

actionlint:
	docker run --rm -v "$(CURDIR)":/repo:ro -w /repo rhysd/actionlint:1.7.12 -color

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

# The desktop app (Tauri), built natively on the Mac. `make desktop` builds
# the UI in Docker, then bundles cos.app. `make desktop-dev` runs the app
# against Vite's dev server, so start `make ui-dev` in another terminal first.
TAURI_DIR := clients/desktop/src-tauri

.PHONY: desktop desktop-dev

desktop: ui-build
	cd $(TAURI_DIR) && cargo tauri build --bundles app
	@echo "Built $(CURDIR)/$(TAURI_DIR)/target/release/bundle/macos/cos.app"

desktop-dev:
	@echo "desktop-dev loads http://localhost:5173: run 'make ui-dev' in another terminal first."
	cd $(TAURI_DIR) && cargo tauri dev
