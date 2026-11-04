SHELL := /bin/bash

CARGO ?= cargo
ENV_FILE ?= .env
ENV_PATH := $(abspath $(ENV_FILE))
BIN := target/release/aiqiezi-server

.DEFAULT_GOAL := help

.PHONY: help env check-env build run run-debug fmt fmt-check check test clippy ci clean

help:
	@printf 'Usage:\n'
	@printf '  make env       Create %s from .env.example if needed\n' "$(ENV_FILE)"
	@printf '  make build     Build the release server binary\n'
	@printf '  make run       Build and run the server with %s\n' "$(ENV_FILE)"
	@printf '  make run-debug Run the server through cargo without release optimization\n'
	@printf '  make fmt       Format all Rust code\n'
	@printf '  make fmt-check Check Rust formatting\n'
	@printf '  make check     Run cargo check for the workspace\n'
	@printf '  make test      Run all workspace tests\n'
	@printf '  make clippy    Run clippy with warnings denied\n'
	@printf '  make ci        Run all checks used by CI\n'
	@printf '  make clean     Remove build artifacts\n'

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
	$(CARGO) build --release --locked -p aiqiezi-server

run: check-env build
	@set -a; . "$(ENV_PATH)"; set +a; exec "./$(BIN)" run

run-debug: check-env
	@set -a; . "$(ENV_PATH)"; set +a; \
		exec $(CARGO) run -p aiqiezi-server -- run

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
