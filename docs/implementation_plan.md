<!--
  docs/implementation_plan.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Server Implementation Plan

Last updated: 2026-11-04

## Purpose

Build `aiqiezi-server` as the single backend for a long-term, self-hosted
photo/video archive. See [../README.md](../README.md) for the project goal,
architecture, deployment model, and non-goals.

## Fixed constraints

- PostgreSQL stores metadata only.
- Photo/video bytes live in content-addressed storage.
- Clients connect with iroh JSON and iroh-blobs.
- No HTTP server or HTTP client.
- Clients are authorized by an iroh EndpointId whitelist.
- There is no login, password, or session-token flow.
- The server is the single source of truth.
- The GUI is deferred until the server and CLI are functional.

## Initial technical decisions

```text
Rust edition:       2021
PostgreSQL:         17
Content hash:       BLAKE3
Asset ID:           UUID
Search:             PostgreSQL FTS + jieba-rs + pg_trgm
Media storage:      content-addressed filesystem
Deployment:         Docker Compose
Server network:     host network
PostgreSQL network: private bridge, 127.0.0.1:5432
Data directory:     /mnt/data/aiqiezi
```

## Working rules

Every step must end with:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

One step should normally be one focused commit. Do not begin the next step
before the current acceptance criteria pass.

## Phase 0 — Foundation

### Step 0.1 — Workspace scaffold

Create:

```text
Cargo.toml
crates/aiqiezi-proto/Cargo.toml
crates/aiqiezi-proto/src/lib.rs
crates/aiqiezi-server/Cargo.toml
crates/aiqiezi-server/src/main.rs
rust-toolchain.toml
rustfmt.toml
```

Acceptance:

- workspace builds;
- empty server binary runs;
- no database or iroh code yet.

### Step 0.2 — Repository metadata

Add or verify:

```text
LICENSE
.gitignore
.env.example
```

Acceptance:

- license is present;
- secrets and runtime data are ignored;
- example configuration contains no real secrets.

### Step 0.3 — Configuration module

Implement:

- `Config` struct;
- environment parsing;
- data/keys/media paths;
- PostgreSQL URL;
- relay/DNS settings;
- bind address;
- log level.

Tests:

- valid configuration;
- missing required values;
- invalid values;
- safe defaults.

### Step 0.4 — Error and logging skeleton

Implement:

- application error type;
- typed library errors with `thiserror`;
- tracing initialization;
- startup and shutdown logs.

Acceptance:

- expected configuration failures do not panic;
- server starts and exits cleanly.

## Phase 1 — iroh endpoint

### Step 1.1 — Persistent secret key

Implement:

- load or create `keys/secret_key.bin`;
- write the endpoint ticket;
- keep EndpointId stable across restarts.

Tests:

- same key produces the same EndpointId;
- malformed key is rejected;
- ticket is updated after endpoint bind.

### Step 1.2 — Endpoint lifecycle

Implement:

- bind the iroh endpoint;
- configure relay and DNS;
- optional bind address;
- router creation;
- SIGINT/SIGTERM handling;
- `router.shutdown()` and `endpoint.close()`.

Acceptance:

- endpoint starts;
- clean shutdown logs;
- no public HTTP listener exists.

### Step 1.3 — Protocol crate v0

Implement in `aiqiezi-proto`:

- request envelope;
- response envelope;
- error envelope;
- protocol version;
- serialization and deserialization tests.

### Step 1.4 — JSON handler skeleton

Implement:

- ALPN `/aiqiezi-json/1`;
- JSON Lines reading;
- `server-info` and `status` commands;
- malformed JSON errors;
- unknown-command errors.

Acceptance:

- a test client can read server information;
- errors use stable error codes;
- responses are machine-readable.

## Phase 2 — PostgreSQL foundation

### Step 2.1 — Database connection

Implement:

- `sqlx` PostgreSQL pool;
- configuration loading;
- startup connection check;
- health-check query.

Add a development PostgreSQL service under `compose/`.

### Step 2.2 — Migration workflow

Implement:

- sqlx migration directory;
- startup policy: auto-migrate or refuse when pending;
- test database reset.

Acceptance:

- empty database becomes ready;
- pending migrations are handled deterministically.

### Step 2.3 — Client whitelist schema

Create:

```text
clients
```

Schema:

```sql
create table clients (
    endpoint_id   bytea primary key,
    label         text,
    enabled       boolean not null default true,
    created_at    timestamptz not null default now(),
    last_seen_at  timestamptz
);

create index on clients(enabled);
```

Do not create users, auth sessions, login tokens, or password tables.

### Step 2.4 — Domain schema

Create migrations for:

```text
assets
vfs_nodes
meta_entries
asset_search
upload_sessions
media_jobs
```

Requirements:

- content hash uniqueness;
- parent/child folder relationships;
- soft deletion flags;
- indexes for normal query paths;
- cursor-friendly ordering;
- `pg_trgm` extension.

### Step 2.5 — Repository layer

Implement typed repository functions for:

- clients;
- assets;
- folders/VFS nodes;
- metadata;
- pagination.

Tests use a disposable PostgreSQL database.

## Phase 3 — Whitelist authorization

### Step 3.1 — Client administration commands

Implement server-host commands:

```text
aiqiezi-server client-add <endpoint-id> [--label NAME]
aiqiezi-server client-list
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-enable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

Acceptance:

- commands update PostgreSQL;
- duplicate additions are handled;
- disabled/removed clients are visible in output.

### Step 3.2 — Connection-level enforcement

Implement:

- extract remote EndpointId from iroh;
- check the whitelist;
- reject unknown or disabled clients;
- update `last_seen_at`;
- cache with periodic reload.

Enforcement must protect both JSON and iroh-blobs ALPNs.

### Step 3.3 — Authorization tests

Test:

- allowed client connects;
- unknown client is rejected;
- disabled client is rejected;
- removed client is rejected;
- both JSON and blob connections are blocked;
- whitelist reload takes effect.

## Phase 4 — Read API

### Step 4.1 — Folder create/list

Implement:

- `folder-create`;
- `folder-list`;
- stable cursor pagination;
- parent validation.

### Step 4.2 — Folder tree

Implement:

- `folder-tree`;
- bounded depth;
- cycle and orphan handling;
- deterministic ordering.

### Step 4.3 — Asset metadata create/get

Implement metadata-only asset creation:

```json
{"command":"asset-create","content_hash":"...","size":123,"mime_type":"image/jpeg"}
```

No blob transfer yet.

### Step 4.4 — Asset list/get

Implement:

- `asset-list`;
- `asset-get`;
- filtering by MIME and date;
- cursor pagination;
- revision field.

## Phase 5 — Media storage and transfer

### Step 5.1 — Content-addressed storage

Implement:

- BLAKE3 hashing;
- temporary file handling;
- atomic move;
- content-addressed path layout;
- deduplication;
- permissions.

Tests:

- correct hash accepted;
- wrong hash rejected;
- duplicate content is deduplicated;
- failed writes leave no partial file.

### Step 5.2 — Upload finalization

Implement:

- confirm blob exists;
- verify hash and size;
- create asset, VFS entry, metadata, and search rows in one transaction;
- return asset ID and revision.

### Step 5.3 — Download authorization

Implement:

- authorized asset lookup;
- blob availability check;
- download metadata response;
- rejection for unknown/disabled clients.

## Phase 6 — Search

### Step 6.1 — Tokenization

Implement:

- combine name/path/labels/description;
- `jieba-rs` segmentation;
- stable token normalization;
- unit tests in Chinese and English.

### Step 6.2 — Search persistence

Implement:

- `search_text`;
- `search_tokens`;
- generated `search_vector`;
- `pg_trgm` index;
- backfill command.

### Step 6.3 — Search command

Implement:

- `asset-search`;
- query tokenization;
- PostgreSQL FTS ranking;
- `pg_trgm` fallback for substring matches;
- cursor pagination;
- filters for labels, date, size, and MIME.

Do not add Tantivy unless real usage proves PostgreSQL search is
insufficient.

## Phase 7 — Docker deployment

### Step 7.1 — Server image

Implement a multi-stage Dockerfile:

- Rust build;
- Debian slim runtime;
- non-root UID 1000;
- no HTTP port;
- optional ffmpeg/ffprobe later.

### Step 7.2 — Compose stack

Services:

```text
aiqiezi-server
postgres
```

Requirements:

- server uses host networking;
- PostgreSQL uses a private bridge network;
- PostgreSQL binds to `127.0.0.1:5432`;
- persistent data under `/mnt/data/aiqiezi`;
- PostgreSQL data at `/mnt/data/aiqiezi/postgres`;
- media at `/mnt/data/aiqiezi/media`;
- keys at `/mnt/data/aiqiezi/keys`;
- backups at `/mnt/data/aiqiezi/backups`.

### Step 7.3 — Backup and restore

Implement scripts for:

- `pg_dump`;
- media checksum manifest;
- keys/config backup;
- restore verification.

Acceptance:

- a full restore test succeeds;
- documentation exists for recovery.

## Phase 8 — CLI integration boundary

### Step 8.1 — Protocol documentation

Write:

```text
docs/protocol.md
```

Document:

- protocol version;
- request/response envelopes;
- error codes;
- every command;
- pagination;
- whitelist failures.

### Step 8.2 — Client library handoff

Provide:

- shared protocol types;
- EndpointId/ticket requirements;
- test client example;
- JSON examples.

### Step 8.3 — End-to-end test

Verify:

- client enrolls its EndpointId;
- client connects;
- server-info succeeds;
- folder/asset reads succeed;
- unauthorized client fails.

After this phase, begin the CLI repository work.

## Definition of done for every step

- Code compiles.
- Tests pass.
- Formatting and clippy pass.
- New behavior is documented.
- No secrets or production data are committed.
- Migration impact is stated.
- Operational and rollback impact is stated.

## Current status

Design phase. Step 0.1 has not started yet.
