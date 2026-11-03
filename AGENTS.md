<!--
  AGENTS.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# AGENTS.md

## AI assistant rules

- Always communicate in English.
- Explain code, errors, and design decisions in English.
- Ask clarification questions in English.
- Read the relevant repository documentation before changing code.
- Never commit secrets, keys, database dumps, media, or runtime data.
- Keep changes small and consistent with the existing architecture.
- Prefer an incremental migration over a destructive rewrite.

## Project context

`aiqiezi-server` is the single backend for a long-term, self-hosted photo and
video archive. See [README.md](README.md) for the project goal, deployment
model, architecture, and non-goals.

Mandatory constraints:

- no HTTP server or HTTP client;
- PostgreSQL stores metadata only;
- media bytes live in content-addressed storage;
- iroh JSON is the control/metadata transport;
- iroh-blobs is the media transport;
- clients are authorized by an iroh EndpointId whitelist;
- there is no login, password, or session-token flow;
- the backend is the single source of truth;
- the GUI is deferred until the CLI/server are functional.

## Repository layout

```text
aiqiezi-server/
├── Cargo.toml                 # workspace
├── crates/
│   ├── aiqiezi-proto/         # shared protocol types
│   └── aiqiezi-server/        # server binary and libraries
├── docs/
├── scripts/
└── AGENTS.md
```

`aiqiezi-proto` owns request/response types and protocol constants. The server
depends on it. CLI and GUI consume it through the shared client library.

## Stack

- Rust 2021.
- Tokio async runtime.
- iroh for endpoint, relay, and NAT traversal.
- iroh JSON ALPN for control and metadata.
- iroh-blobs for media upload/download.
- PostgreSQL through `sqlx`.
- `serde` / `serde_json` for protocol messages.
- `tracing` for structured logs.
- `anyhow` at application boundaries; `thiserror` for typed library errors.
- `image` and `ffmpeg`/`ffprobe` for media processing later.
- Docker Compose for deployment.

## Server responsibilities

- endpoint identity, relay/DNS configuration, and EndpointId whitelist
  authorization;
- JSON protocol handlers;
- PostgreSQL schema, migrations, and queries;
- content-addressed media storage;
- upload/download coordination;
- thumbnail and media metadata generation later;
- pagination, cursors, and search;
- backups, integrity checks, and maintenance;
- health and operational status.

## PostgreSQL rules

- Metadata only; never store original photo/video bytes.
- Every schema change must have a migration.
- Prefer additive migrations and nullable rollout fields.
- Use transactions for multi-table updates.
- Use indexes for every normal query path.
- Use cursor pagination for large result sets; avoid deep OFFSET scans.
- Keep transfer history prunable.
- Back up PostgreSQL and media storage together.

Core entities:

```text
clients                  # EndpointId whitelist
assets
folders / VFS nodes
meta entries
tags
albums
upload sessions          # resumable uploads only
transfer history
media jobs
```

Clients are authorized by EndpointId:

```sql
create table clients (
    endpoint_id   bytea primary key,
    label         text,
    enabled       boolean not null default true,
    created_at    timestamptz not null default now(),
    last_seen_at  timestamptz
);
```

## Media storage rules

- Store media in a content-addressed tree, for example:
  `data/media/data/<hash>.data`.
- Verify every uploaded file against its content hash.
- Preserve originals; generate thumbnails and previews separately.
- Deduplicate identical content by hash.
- Never modify an original in place.
- Keep storage paths configurable; do not hardcode production paths.
- Protect current data from garbage collection.
- Test restore, not just backup.

## Protocol rules

- ALPN_AIQIEZI_JSON is the control and metadata transport.
- iroh-blobs is the media transport.
- No HTTP server or HTTP client for normal operation.
- Protocol messages are versioned JSON.
- Paginate large responses.
- Include a revision/timestamp for cache coherence.
- Authorize clients by iroh EndpointId whitelist, not passwords or session
  tokens.
- Enforce the whitelist at the iroh handshake/firewall layer so JSON and blob
  ALPNs are protected equally.
- Keep the backend secret key persistent; EndpointId stability depends on it.

## Security rules

- Never commit keys, relay tokens, database dumps, or media.
- Keep authorization separate from transport encryption.
- Keep PostgreSQL off the public network.
- Run the server as a dedicated non-root user.
- Use minimum filesystem permissions.
- Use least-privilege database roles.
- Keep relay tokens and database credentials in environment or secret files.
- Do not expose Docker's socket to clients.
- Do not weaken the whitelist for local development.
- A stolen client key grants that client's access until its EndpointId is
  disabled or removed.

## Client administration

The whitelist is stored in PostgreSQL and enforced by the running server.

```text
aiqiezi-server client-add <endpoint-id> [--label NAME]
aiqiezi-server client-list
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-enable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

Clients generate and persist their own iroh secret key. Enrollment is done by
adding the resulting EndpointId on the server. There is no login command.

## Commands

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
sqlx migrate run
cargo run -p aiqiezi-server
```

## Source file rules

Every new `.rs`, `.sql`, or `.md` source file must start with the project
license header, using the file path relative to the repository root:

```rust
/*
  relative/path.rs
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
*/
```

For Markdown use an HTML comment with the same content.

## Testing rules

- Unit-test protocol encoding, pagination, validation, and storage helpers.
- Integration-test PostgreSQL migrations.
- Test media hash verification.
- Test whitelist enforcement for allowed, unknown, disabled, and removed
  clients.
- Test that JSON and blob connections are rejected for unauthorized peers.
- Test backup/restore procedures for metadata.
- Never require production data or production secrets in tests.

## Agent workflow

1. Read this file and the relevant protocol/schema code.
2. Explain the intended change before large edits.
3. Keep changes scoped.
4. Run formatting, tests, and checks.
5. Report migration requirements, operational impact, and rollback steps.
6. Update this file when a lasting architectural rule changes.
