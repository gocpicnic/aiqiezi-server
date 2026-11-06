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
- initial enrollment uses `AIQIEZI_BOOTSTRAP_PASSWORD`;
- the first login must change the bootstrap password;
- after the change, clients are authorized by an iroh EndpointId whitelist;
- there is no session-token flow;
- there is one daemon process with a public iroh listener and a local Unix
  admin socket;
- admin commands are local-only and never exposed through iroh or TCP;
- the backend is the single source of truth;
- the GUI is deferred until the CLI/server are functional.

## Platform support

- The server deployment target is a Linux host running Docker Engine with
  Compose.
- The container image targets `linux/amd64`; ARM64 is deferred.
- Docker Desktop on macOS or Windows is not a supported server host.
- Native server builds and deployments on Windows or macOS are unsupported.
- Do not add Windows- or macOS-specific server code paths.

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
- `argon2` / `password-hash` for bootstrap password storage.
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

## PostgreSQL rules (Phase 2)

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

## Container startup rules

- Server binary is compiled outside Docker for
  `x86_64-unknown-linux-musl`.
- Every image build tags both the dated tag and `latest`.
- Compose uses `latest` for simple NAS upgrades; keep dated tags for rollback.
- The Compose deployment is one self-contained `compose.yaml` with inline
  environment values. Do not add an `env_file` or require a `.env`.
- The deployed `compose.yaml` contains secrets and must be mode `0600`; never
  commit the filled-in copy.
- PostgreSQL is not part of the current Compose stack; add it back in Phase 2.
- Pushing a `v*` tag always publishes a `docker save` archive, `compose.yaml`,
  and `SHA256SUMS` as GitHub Release assets.
- The workflow also attempts to publish the GHCR version, dated, and `latest`
  tags. GHCR publishing is best-effort until package permissions and visibility
  are configured; it must not block archive publication.
- The release image is Linux/amd64 only; Windows and macOS server images are not
  published.
- Never include `.env` values, keys, auth state, database dumps, or media in
  release assets.
- Runtime image is Debian slim with `tini` and `gosu`; Rust is not built in
  the image.
- The entrypoint starts as root only for initialization.
- The entrypoint creates and chowns only app-owned directories.
- The entrypoint drops privileges to `AIQIEZI_UID:AIQIEZI_GID`.
- Default runtime UID/GID is `1000:1000`; operators can override it.
- The server process must never run as root.
- Never chown or touch PostgreSQL data from the server entrypoint.
- Do not require a setup-host script for normal Compose startup.

## Daemon process and admin control

- Run one daemon process with two listeners:
  - public iroh endpoint for clients;
  - local Unix admin socket for server commands.
- Admin socket path: `/run/aiqiezi/admin.sock`.
- Instance lock path: `/run/aiqiezi/server.lock`.
- Runtime directory defaults to `/run/aiqiezi` and is configurable with
  `AIQIEZI_RUNTIME_DIR`; the development Makefile uses `./data/run`.
- Admin socket permissions must be restrictive; never expose it over TCP or
  iroh.
- Admin requests use JSON Lines, one request per connection.
- The daemon owns database writes and updates the whitelist cache immediately.
- `set-password` is a daemon-owned admin write. The online path uses the admin
  socket; the `--offline` recovery path holds the instance lock.
- `migrate` and `migrate-status` are one-shot commands outside the daemon and
  must hold the same instance lock.
- Do not run two daemon processes against the same PostgreSQL database or media
  store.

## Protocol rules

- ALPN_AIQIEZI_JSON is the control and metadata transport.
- ALPN_AIQIEZI_AUTH is the bootstrap login and password-change transport.
- iroh-blobs is the media transport.
- No HTTP server or HTTP client for normal operation.
- Protocol messages are versioned JSON.
- Paginate large responses.
- Include a revision/timestamp for cache coherence.
- The auth ALPN is accepted before whitelist enforcement; the JSON and blob
  ALPNs are accepted only for whitelisted EndpointIds.
- Authorize enrolled clients by iroh EndpointId whitelist, not session tokens.
- Enforce the whitelist at the iroh handshake/firewall layer so JSON and blob
  ALPNs are protected equally.
- Do not add a local-IP bypass to the firewall.
- Initial blob access uses the trusted-whitelist model: any whitelisted client
  may fetch any blob if it knows the content hash.
- Per-asset blob ACLs are deferred until multi-user access is required.
- Keep the backend secret key persistent; EndpointId stability depends on it.

## Security rules

- Never commit keys, relay tokens, database dumps, or media.
- Keep the bootstrap password in `AIQIEZI_BOOTSTRAP_PASSWORD` in the deployed
  `compose.yaml`, not in the repository. Never log it or return it in a
  response.
- Store only the Argon2id password hash. Use `spawn_blocking` for hashing and
  verification.
- The admin `set-password` path must verify the current password and must never
  log or return password request parameters.
- Rate-limit failed login attempts per EndpointId.
- Auth state files live under the keys directory with `0600`; the keys
  directory uses `0700`.
- Keep authorization separate from transport encryption.
- Keep PostgreSQL off the public network.
- Run the application process as a non-root user; root is allowed only in the
  short-lived container entrypoint initialization.
- Use minimum filesystem permissions.
- Use least-privilege database roles.
- Keep relay tokens and database credentials in environment or secret files.
- Do not expose Docker's socket to clients.
- Do not weaken the whitelist for local development.
- A stolen client key grants that client's access until its EndpointId is
  disabled or removed.

## Client administration

Initial enrollment uses the server bootstrap password:

```text
AIQIEZI_BOOTSTRAP_PASSWORD   # deployed compose.yaml, first startup only
whoami                       # server command, prints the server EndpointId
login                        # client command over ALPN_AIQIEZI_AUTH
change-password              # required after the first login
```

`aiqiezi-server whoami` is read-only, prints only the persistent server
EndpointId, and requires an existing server key. It never prints the endpoint
ticket.

The first `login` with the bootstrap password returns
`password_change_required` and does not enroll the client. `change-password`
verifies the old password, stores a new Argon2id hash, clears the
must-change state, clears the whitelist, and enrolls the caller's EndpointId.
The caller must use its persistent iroh key; a temporary key would enroll the
wrong identity. After that, the bootstrap value is ignored.

The whitelist is the permanent authorization store. It is file-backed under
`keys/` until Phase 3 migrates client state to PostgreSQL.

Administrators can manage enrolled clients:

```text
aiqiezi-server client-add <endpoint-id> [--label NAME]
aiqiezi-server client-list
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-enable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

`set-password` changes the password through the running daemon over the local
admin socket. It verifies the current password, stores a new Argon2id hash,
and by default clears the whitelist so every client must re-enroll. The
`--keep-clients` option preserves the whitelist for password-only rotation.

The `--offline` option is a recovery path for terminal users. It requires the
daemon to be stopped and holds the instance lock. Do not use the offline path
through `docker compose run` while the daemon is running; the one-off container
does not share the runtime directory lock.

Clients generate and persist their own iroh secret key. They never store the
server password.

## Commands

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
sqlx migrate run
cargo run -p aiqiezi-server -- start
cargo run -p aiqiezi-server -- whoami
cargo run -p aiqiezi-server -- set-password
cargo run -p aiqiezi-server -- set-password --offline
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
- Test bootstrap login, `password_change_required`, password change, restart
  persistence, rate limiting, and weak-password rejection.
- Test online admin `set-password`, `--keep-clients`, stale-password rejection,
  and the offline lock fallback.
- Test that passwords and hashes never appear in logs or responses.
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
