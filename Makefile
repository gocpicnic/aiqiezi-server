SHELL := /bin/bash

CARGO ?= cargo
# Optional local-development overrides. Production uses compose.yaml only.
ENV_FILE ?= .env
ENV_PATH := $(abspath $(ENV_FILE))
BIN := target/release/aiqiezi-server
MUSL_TARGET := x86_64-unknown-linux-musl
CARGO_VERSION := $(shell $(CARGO) pkgid -p aiqiezi-server 2>/dev/null | sed 's/.*#//; s/[^A-Za-z0-9_.-]/-/g')
BUILD_DATE := $(shell /bin/date -u +%Y%m%d%H%M%S)
RUNTIME_DIR ?= ./data/run
IMAGE_REPO ?= ghcr.io/gocpicnic/aiqiezi-server
APP_VERSION ?= v$(CARGO_VERSION)
IMAGE_TAG ?= $(APP_VERSION)-$(BUILD_DATE)
LATEST_TAG ?= latest
DOCKER_PLATFORM ?= linux/amd64
IMAGE_TAR_DIR ?= imagefiles
IMAGE := $(IMAGE_REPO):$(IMAGE_TAG)
LATEST_IMAGE := $(IMAGE_REPO):$(LATEST_TAG)

.DEFAULT_GOAL := help

# Declare all targets as PHONY.
.PHONY: help build build-static setup-musl run run-debug version \
	status healthcheck shutdown docker-image docker-push docker-push-latest \
	docker-save check-image-version fmt fmt-check check test clippy ci clean

help:
	@printf 'Usage:\n'
	@printf '  make build        Build the release server binary\n'
	@printf '  make build-static Build the static musl binary into dist/\n'
	@printf '  make docker-image Build the dated and latest server images\n'
	@printf '  make docker-push  Build and push the dated and latest images\n'
	@printf '  make docker-push-latest  Build and push only the latest image\n'
	@printf '  make docker-save  Build and save both image tags into imagefiles/\n'
	@printf '  make run          Build and run the server\n'
	@printf '  make run-debug    Run the server through cargo without release optimization\n'
	@printf '  make status       Query the running daemon status\n'
	@printf '  make healthcheck  Check the running daemon health\n'
	@printf '  make shutdown     Shut down the running daemon\n'
	@printf '  make version      Print the built server version\n'
	@printf '  make fmt          Format all Rust code\n'
	@printf '  make fmt-check    Check Rust formatting\n'
	@printf '  make check        Run cargo check for the workspace\n'
	@printf '  make test         Run all workspace tests\n'
	@printf '  make clippy       Run clippy with warnings denied\n'
	@printf '  make ci           Run all checks used by CI\n'
	@printf '  make clean        Remove build artifacts\n'

build:
	BUILD_DATE="$(BUILD_DATE)" $(CARGO) build --release --locked -p aiqiezi-server

setup-musl:
	@rustup target list --installed | grep -qx "$(MUSL_TARGET)" || \
		rustup target add "$(MUSL_TARGET)"

build-static: setup-musl
	BUILD_DATE="$(BUILD_DATE)" $(CARGO) build --release --locked \
		--target "$(MUSL_TARGET)" -p aiqiezi-server
	mkdir -p dist
	cp "target/$(MUSL_TARGET)/release/aiqiezi-server" dist/aiqiezi-server

docker-image: check-image-version build-static
	@printf 'Building image: %s (latest: %s)\n' "$(IMAGE)" "$(LATEST_IMAGE)"
	docker build --platform "$(DOCKER_PLATFORM)" \
		-t "$(IMAGE)" \
		-t "$(LATEST_IMAGE)" \
		.

check-image-version:
	@test -n "$(APP_VERSION)" || { \
		echo "error: could not determine APP_VERSION from Cargo metadata" >&2; \
		echo "set APP_VERSION explicitly, for example APP_VERSION=v0.1.0" >&2; \
		exit 1; \
	}

docker-push: docker-image
	docker push "$(IMAGE)"
	docker push "$(LATEST_IMAGE)"

docker-push-latest: docker-image
	docker push "$(LATEST_IMAGE)"

docker-save: docker-image
	mkdir -p "$(IMAGE_TAR_DIR)"
	docker save -o "$(IMAGE_TAR_DIR)/aiqiezi-server-$(IMAGE_TAG).tar" \
		"$(IMAGE)" \
		"$(LATEST_IMAGE)"

run: build
	@set -a; if [ -f "$(ENV_PATH)" ]; then . "$(ENV_PATH)"; fi; set +a; \
		export AIQIEZI_DATABASE_URL="$${AIQIEZI_DATABASE_URL:-postgres://unused:unused@127.0.0.1/unused}"; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" start

run-debug:
	@set -a; if [ -f "$(ENV_PATH)" ]; then . "$(ENV_PATH)"; fi; set +a; \
		export AIQIEZI_DATABASE_URL="$${AIQIEZI_DATABASE_URL:-postgres://unused:unused@127.0.0.1/unused}"; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		BUILD_DATE="$(BUILD_DATE)" exec $(CARGO) run -p aiqiezi-server -- start

status: build
	@set -a; if [ -f "$(ENV_PATH)" ]; then . "$(ENV_PATH)"; fi; set +a; \
		export AIQIEZI_DATABASE_URL="$${AIQIEZI_DATABASE_URL:-postgres://unused:unused@127.0.0.1/unused}"; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" status

healthcheck: build
	@set -a; if [ -f "$(ENV_PATH)" ]; then . "$(ENV_PATH)"; fi; set +a; \
		export AIQIEZI_DATABASE_URL="$${AIQIEZI_DATABASE_URL:-postgres://unused:unused@127.0.0.1/unused}"; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" healthcheck

shutdown: build
	@set -a; if [ -f "$(ENV_PATH)" ]; then . "$(ENV_PATH)"; fi; set +a; \
		export AIQIEZI_DATABASE_URL="$${AIQIEZI_DATABASE_URL:-postgres://unused:unused@127.0.0.1/unused}"; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" shutdown

version: build
	@./$(BIN) --version

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

check:
	$(CARGO) check --workspace --locked

test:
	$(CARGO) test --workspace --locked

clippy:
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

ci: fmt-check check test clippy

clean:
	$(CARGO) clean
