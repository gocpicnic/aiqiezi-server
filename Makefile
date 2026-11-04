SHELL := /bin/bash

CARGO ?= cargo
ENV_FILE ?= .env
ENV_PATH := $(abspath $(ENV_FILE))
BIN := target/release/aiqiezi-server
MUSL_TARGET := x86_64-unknown-linux-musl
BUILD_DATE := $(shell /bin/date -u +%Y%m%d%H%M)
RUNTIME_DIR ?= ./data/run

.DEFAULT_GOAL := help

# Declare all targets as PHONY.
.PHONY: help env check-env build build-static setup-musl run run-debug version \
	status healthcheck shutdown fmt fmt-check check test clippy ci clean

help:
	@printf 'Usage:\n'
	@printf '  make env          Create %s from .env.example if needed\n' "$(ENV_FILE)"
	@printf '  make build        Build the release server binary\n'
	@printf '  make build-static Build the static musl binary into dist/\n'
	@printf '  make run          Build and run the server with %s\n' "$(ENV_FILE)"
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

env:
	@if [ -f "$(ENV_FILE)" ]; then \
		echo "$(ENV_FILE) already exists"; \
	else \
		cp .env.example "$(ENV_FILE)"; \
		echo "created $(ENV_FILE); edit it before running the server"; \
	fi

check-env:
	@if [ ! -f "$(ENV_PATH)" ]; then \
		echo "error: $(ENV_FILE) is missing; run 'make env' first" >&2; \
		exit 1; \
	fi

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

run: check-env build
	@set -a; . "$(ENV_PATH)"; set +a; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" run

run-debug: check-env
	@set -a; . "$(ENV_PATH)"; set +a; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		BUILD_DATE="$(BUILD_DATE)" exec $(CARGO) run -p aiqiezi-server -- run

status: check-env build
	@set -a; . "$(ENV_PATH)"; set +a; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" status

healthcheck: check-env build
	@set -a; . "$(ENV_PATH)"; set +a; \
		export AIQIEZI_RUNTIME_DIR="$${AIQIEZI_RUNTIME_DIR:-$(RUNTIME_DIR)}"; \
		exec "./$(BIN)" healthcheck

shutdown: check-env build
	@set -a; . "$(ENV_PATH)"; set +a; \
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
