<!--
  docs/implementation_plan.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Server Implementation Plan

Last updated: 2026-11-04

## Status

Completed: Phase 0, Phase 0B, Phase 1, Phase 1B daemon infrastructure
(instance lock, admin socket, startup/shutdown ordering, and admin tests), and
Phase 1C bootstrap password authentication.

Docker runtime image, Compose stack, entrypoint, and manual build/push/save
workflow are implemented ahead of the main phase order. Docker image tags use
the Cargo package version plus a `YYYYMMDDHHMMSS` UTC build timestamp. CI image
publishing is still deferred.

Deferred to Phase 2/3: database, migration, media-storage, and PostgreSQL
whitelist fields in admin `status`/`healthcheck`, plus `reload-whitelist`.

Next: Phase 2 — PostgreSQL foundation.

## Purpose

Build `aiqiezi-server` as the single backend for a long-term, self-hosted
photo/video archive. See [../README.md](../README.md) for the project goal,
architecture, deployment model, and non-goals.

## Fixed constraints

- PostgreSQL stores metadata only.
- Photo/video bytes live in content-addressed storage.
- Clients connect with iroh JSON and iroh-blobs.
- No HTTP server or HTTP client.
- Initial enrollment uses `AIQIEZI_BOOTSTRAP_PASSWORD`.
- The first login must change the bootstrap password.
- After the change, clients are authorized by an iroh EndpointId whitelist.
- There is no session-token flow.
- The server is the single source of truth.
- There is one daemon process with two listeners:
  - a public iroh endpoint for clients;
  - a local Unix admin socket for server commands.
- Admin commands are local-only and never exposed through iroh.
- The GUI is deferred until the server and CLI are functional.

## Initial technical decisions

```text
Rust edition:       2021
PostgreSQL:         17 (Phase 2; not in current Compose stack)
Content hash:       BLAKE3
Asset ID:           UUID
Search:             PostgreSQL FTS + jieba-rs + pg_trgm
Media storage:      content-addressed filesystem
Deployment:         single-file compose.yaml on Linux Docker Engine
Server build:       x86_64-unknown-linux-musl, built outside Docker
Server runtime:     Debian slim + tini + gosu entrypoint
Container user:     AIQIEZI_UID/AIQIEZI_GID, default 1000:1000
Container startup:  root entrypoint initializes and drops privileges
Setup script:       not required; one self-contained compose.yaml
Server network:     host network
PostgreSQL network: not configured until Phase 2
Data directory:     /mnt/data/aiqiezi
Daemon model:       one process, two listeners
Admin transport:    Unix domain socket /run/aiqiezi/admin.sock
Admin protocol:     JSON Lines, one request per connection
Instance lock:      /run/aiqiezi/server.lock
Server platform:    linux/amd64 only
Docker Desktop:     unsupported on macOS and Windows
Auth transport:     /aiqiezi-auth/1
Auth bootstrap:     AIQIEZI_BOOTSTRAP_PASSWORD, first startup only
Auth storage:       Argon2id hash + temporary file whitelist under keys/
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

## Phase 0B — CI foundation

CI starts after the Cargo workspace exists. Public repositories use GitHub
Actions.

### Step 0B.1 — Base CI workflow

Add:

```text
.github/workflows/ci.yml
```

Triggers:

```text
push to main
pull_request
workflow_dispatch
```

Permissions:

```yaml
permissions:
  contents: read
```

Jobs:

```text
fmt
clippy
test
```

Rules:

- use `ubuntu-latest` GitHub-hosted runners;
- use `cargo fmt --check`;
- use `cargo clippy --workspace --all-targets -- -D warnings`;
- use `cargo test --workspace`;
- cache Cargo dependencies;
- cancel superseded runs;
- pin third-party Actions to commit SHAs;
- do not use `pull_request_target`;
- do not expose repository secrets to fork pull requests.

### Step 0B.2 — Dependency and security automation

Add:

```text
.github/dependabot.yml
SECURITY.md
```

Dependabot updates:

```text
cargo dependencies
GitHub Actions
```

Do not enable automatic merging of dependency updates.

### Step 0B.3 — Branch protection

For `main`:

- require the CI checks;
- disallow force pushes;
- require pull requests once the project is active;
- keep production deployment credentials out of the repository.

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

## Phase 1B — Daemon process and local admin control

The server is one daemon process with two listeners:

```text
aiqiezi-server start
├── iroh endpoint + RouterBuilder
│   ├── /aiqiezi-auth/1
│   ├── /aiqiezi-json/1
│   └── iroh-blobs
└── UnixListener
    └── /run/aiqiezi/admin.sock
```

Both listeners share the same application state. Do not run two daemon
processes against the same PostgreSQL database and media store.

### Step 1B.1 — Daemon instance lock

Implement:

- Unix-only `flock` instance lock;
- lock path `/run/aiqiezi/server.lock`;
- directory `/run/aiqiezi` with restrictive permissions;
- runtime directory configurable with `AIQIEZI_RUNTIME_DIR`; the development
  Makefile uses `./data/run` so the daemon runs without root;
- acquire before database initialization;
- release automatically on process exit;
- `migrate` acquires the same lock to verify the daemon is stopped.

Tests:

- two daemon starts: second fails;
- lock is released after clean shutdown;
- stale lock file without an owner does not block a new start;
- socket and lock paths are in a non-world-writable directory.

### Step 1B.2 — Admin socket and protocol

Implement:

- `tokio::net::UnixListener` at `/run/aiqiezi/admin.sock`;
- directory mode `0750`;
- socket mode `0660` or stricter;
- stale socket removal only when the instance lock is free;
- JSON Lines request/response, one request per connection.

Request envelope:

```json
{
  "version": 1,
  "request_id": "optional",
  "command": "status",
  "params": {}
}
```

Success response:

```json
{
  "version": 1,
  "request_id": "optional",
  "ok": true,
  "data": {}
}
```

Error response:

```json
{
  "version": 1,
  "request_id": "optional",
  "ok": false,
  "error": {
    "code": "invalid_request",
    "message": "..."
  }
}
```

Stable error codes:

```text
invalid_request
not_found
conflict
database_unavailable
migrations_pending
storage_unavailable
endpoint_unavailable
shutting_down
internal
```

### Step 1B.3 — Initial admin commands

Implement:

```text
status
healthcheck
shutdown
reload-whitelist
```

`status` returns safe operational information:

- server version;
- iroh EndpointId;
- uptime;
- database status;
- migration status;
- media storage status;
- enabled client count.

`healthcheck` checks:

- daemon is running;
- PostgreSQL is reachable;
- migrations are current;
- media storage is writable;
- iroh endpoint is bound.

`shutdown` triggers the shared shutdown token.

Never return database URLs, relay tokens, or endpoint tickets from `status`.

### Step 1B.4 — Startup and shutdown ordering

Startup:

```text
1. Load configuration and logging.
2. Acquire the instance lock.
3. Connect to PostgreSQL.
4. Verify migrations are current.
5. Initialize media storage.
6. Create the admin UnixListener.
7. Bind the iroh endpoint.
8. Start the iroh router.
9. Write readiness state and log EndpointId/ticket.
10. Serve both listeners.
```

Shutdown:

```text
1. Stop accepting admin commands.
2. Shut down the iroh router.
3. Close the iroh endpoint.
4. Close the PostgreSQL pool.
5. Remove the admin socket.
6. Release the instance lock.
```

### Step 1B.5 — Admin tests

Test:

- admin socket permissions;
- malformed JSON;
- unknown command;
- status response contains no secrets;
- healthcheck success and failure;
- duplicate daemon start;
- clean shutdown from the admin socket;
- stale socket removal;
- simultaneous admin requests.

## Phase 1C — Bootstrap password authentication

The first client enrollment uses a bootstrap password from
`AIQIEZI_BOOTSTRAP_PASSWORD`. This is not a session-token flow: the password
only bootstraps the EndpointId whitelist. After the first password change, the
bootstrap value is ignored.

### Step 1C.1 — Auth state

Implement:

- `keys/auth.json` with the Argon2id hash, must-change flag, and temporary
  EndpointId whitelist;
- `0600` file permissions and `0700` directory permissions;
- atomic replacement so the hash and whitelist are updated together.

Startup rules:

- if a password hash exists, load it and ignore the bootstrap env value;
- if no hash exists, require `AIQIEZI_BOOTSTRAP_PASSWORD`;
- validate the bootstrap password strength before hashing;
- persist the hash and set the must-change state;
- fail startup clearly when neither a hash nor the env value exists.

Tests:

- missing hash and missing env fails;
- weak bootstrap password fails;
- valid bootstrap creates a hash and must-change state;
- existing hash ignores the env value;
- whitelist persistence round-trip.

### Step 1C.2 — Auth transport

Implement:

- `ALPN_AIQIEZI_AUTH = /aiqiezi-auth/1`;
- `login` and `change-password` over the auth ALPN;
- `login` returns `password_change_required` while the must-change state is
  set and does not enroll the client;
- successful login after the password change enrolls the EndpointId;
- `change-password` verifies the old password, validates the new password,
  rehashes it, clears the must-change state, and resets the whitelist to the
  caller;
- rate-limit failed login attempts per EndpointId;
- run Argon2 hashing and verification in `spawn_blocking`;
- never log or return passwords or hashes.

### Step 1C.3 — Connection firewall

Implement:

- `EndpointHooks` loaded before the endpoint accepts connections;
- accept the auth ALPN before whitelist enforcement;
- accept the JSON and blob ALPNs only for whitelisted EndpointIds;
- reject all other peers with a stable reason;
- do not add a local-IP bypass.

Tests:

- unauthenticated JSON connection is rejected;
- auth ALPN connection is accepted;
- whitelisted JSON connection is accepted;
- disabled/removed clients are rejected;
- login and password-change state survives restart.

### Step 1C.4 — Server commands

Implement:

- `start` as the primary foreground daemon command, with `run` as an alias;
- `whoami` as a read-only command that prints only the persistent server
  EndpointId and does not create a key;
- `set-password` as an optional offline command that holds the instance lock,
  verifies the old password when present, and clears the whitelist.

## Phase 2 — PostgreSQL foundation

### Step 2.1 — Database connection

Implement:

- `sqlx` PostgreSQL pool;
- configuration loading;
- startup connection check;
- health-check query.

When Phase 2 begins, add a development PostgreSQL service (for example under
`compose/` or as a Compose profile).

### Step 2.2 — Migration workflow

Implement:

- sqlx migration directory;
- explicit migration command;
- startup policy: refuse to start when migrations are pending;
- test database reset.

Recommended configuration:

```text
AIQIEZI_AUTO_MIGRATE=false
```

Deployment runs migrations before recreating the server. The server never
silently applies schema changes during a normal restart.

Acceptance:

- empty database can be migrated explicitly;
- startup fails clearly when migrations are pending;
- deployment can run migrations as a separate one-shot step.

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

Do not create users, auth sessions, or login-token tables. The bootstrap
password hash remains a server-local file; the temporary file whitelist is
migrated into the `clients` table.

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

For soft-deleted VFS entries, use a partial unique index instead of a plain
unique constraint:

```sql
create unique index vfs_nodes_parent_name_active
on vfs_nodes(parent_id, name)
where deleted = false;
```

This allows deleting and re-importing the same path.

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

Implement server-host commands over the local admin socket:

```text
aiqiezi-server client-add <endpoint-id> [--label NAME]
aiqiezi-server client-list
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-enable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

The CLI sends JSON to `/run/aiqiezi/admin.sock`; the daemon owns all database
writes and updates the in-memory whitelist cache immediately.

Acceptance:

- commands update PostgreSQL through the daemon;
- duplicate additions are handled idempotently;
- disabled/removed clients are visible in output;
- whitelist changes take effect for subsequent connections without a restart.

### Step 3.2 — Connection-level enforcement

Implement:

- extract remote EndpointId from iroh;
- check the whitelist;
- reject unknown or disabled clients;
- update `last_seen_at`;
- update the whitelist cache immediately on admin changes;
- keep periodic database reload as a fallback.

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

### Step 4.3 — Asset metadata read and test fixtures

Implement:

- asset lookup by hash/ID;
- internal test fixtures or an internal importer for metadata-only asset rows.

Do not expose a public `asset-create` command. Upload finalization is the
canonical public asset-creation path, so asset, VFS, metadata, and search rows
are created together atomically.

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

Initial security model: trusted whitelist.

Any whitelisted client may fetch any blob if it knows the content hash.
Per-asset ACLs are not implemented in the first version because iroh-blobs does
not consult PostgreSQL per blob request.

This is acceptable for a single-owner archive where all whitelisted clients
belong to the owner. Document the limitation and revisit per-asset authorization
if multi-user access is added.

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

## Phase 7 — Docker, CI/CD, and deployment

The production target is the owner's self-hosted server, behind NAT. GitHub
Actions publishes images; the server pulls explicit versions.

### Step 7.1 — Server binary and image

Build the server binary outside Docker:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
mkdir -p dist
cp target/x86_64-unknown-linux-musl/release/aiqiezi-server dist/
```

Package it with a Debian slim runtime image:

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates tini gosu \
    && rm -rf /var/lib/apt/lists/*

COPY dist/aiqiezi-server /usr/local/bin/aiqiezi-server
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
CMD ["start"]
```

The entrypoint:

```text
1. Reads AIQIEZI_UID and AIQIEZI_GID (default 1000:1000).
2. Creates app-owned data directories.
3. Chowns only the app-owned directories.
4. Drops privileges with gosu.
5. Executes the server as the selected UID/GID.
```

Requirements:

- no Rust build inside the image;
- no fixed host UID baked into the image;
- server process never runs as root;
- entrypoint never touches PostgreSQL data;
- no setup-host script required;
- no HTTP port;
- no ffmpeg/ffprobe in this image.

The image must contain the server binary and the operational subcommands:

```text
aiqiezi-server start
aiqiezi-server migrate
aiqiezi-server migrate-status
aiqiezi-server healthcheck
```

The initial platform is `linux/amd64` on a Linux Docker Engine host. ARM64 is
deferred until there is a real ARM target; when needed, use a native ARM64
build runner rather than QEMU emulation. Docker Desktop on macOS or Windows and
native server builds on those platforms are not supported.

FFmpeg and video processing will live in a separate media-worker image later.

### Step 7.2 — Compose stack

The current stack contains only `aiqiezi-server`, configured by one
self-contained `compose.yaml` with inline environment values. PostgreSQL is
added in Phase 2; the requirements below describe that future stack.

Current services:

```text
aiqiezi-server
```

Phase 2 adds:

```text
postgres
```

Requirements:

- server uses host networking;
- server image is `ghcr.io/gocpicnic/aiqiezi-server:latest`, with dated tags
  kept for rollback;
- runtime UID/GID is supplied inline in `compose.yaml`:

```yaml
environment:
  AIQIEZI_UID: "1000"
  AIQIEZI_GID: "1000"
```

- the entrypoint creates and chowns only app-owned directories;
- host data directories do not need a separate setup script;
- PostgreSQL uses a private bridge network;
- PostgreSQL binds to `127.0.0.1:5432`;
- PostgreSQL uses its official image entrypoint for data-directory ownership;
- do not set a fixed `user:` on the PostgreSQL service;
- set `PGDATA=/var/lib/postgresql/data/pgdata`;
- persistent data under `/mnt/data/aiqiezi`;
- PostgreSQL data at `/mnt/data/aiqiezi/postgres`;
- media at `/mnt/data/aiqiezi/media`;
- keys at `/mnt/data/aiqiezi/keys`;
- backups at `/mnt/data/aiqiezi/backups`;
- deploy files at `/mnt/data/aiqiezi/deploy`.

Do not check out the full source repository on the production server.

The admin Unix socket stays inside the container by default. Use:

```bash
docker compose exec aiqiezi-server aiqiezi-server status
docker compose exec aiqiezi-server aiqiezi-server client-add <endpoint-id>
docker compose exec aiqiezi-server aiqiezi-server healthcheck
```

Optional host-side access requires a bind-mounted `/run/aiqiezi` directory with
restrictive permissions. Do not expose the admin socket over TCP or iroh.

If PostgreSQL reports a data-directory permission error, a one-time host-side
`chown 999:999 /mnt/data/aiqiezi/postgres` is acceptable. This is troubleshooting,
not a required setup script.

### Step 7.3 — Migration and readiness commands

Implement server subcommands used by deployment:

```text
aiqiezi-server start
aiqiezi-server whoami
aiqiezi-server migrate
aiqiezi-server migrate-status
aiqiezi-server healthcheck
```

`start` starts the normal server process; `run` remains an alias. `whoami`
prints only the server EndpointId. The deploy
script uses the other
subcommands for migrations and readiness.

`status`, `healthcheck`, client administration, and `shutdown` connect to the
local Unix admin socket. `migrate` and `migrate-status` are one-shot commands
that use PostgreSQL directly and must hold the daemon instance lock.

Health checks verify:

- process is alive;
- PostgreSQL is reachable;
- migrations are current;
- media storage is writable;
- iroh endpoint is bound.

No HTTP health endpoint is required.

### Step 7.4 — Backup and restore

Implement scripts for:

- `pg_dump`;
- media checksum manifest;
- keys/config backup;
- restore verification.

Acceptance:

- a full restore test succeeds;
- documentation exists for recovery.

### Step 7.5 — Controlled deploy script

Create `scripts/deploy.sh` for the production server.

Flow:

```text
1. Acquire an exclusive deployment lock.
2. Validate the requested image tag.
3. Pull the new image.
4. Record the currently running image ID/tag.
5. Back up PostgreSQL.
6. Stop the server container.
7. Run migrations as a one-shot job with the daemon lock free.
8. Recreate only the server container.
9. Wait for readiness through the admin socket.
10. Restore the previous image on failure.
11. Retain the backup and previous image for rollback.
```

Rules:

- use `latest` for NAS/manual upgrades and keep dated tags for rollback;
- never deploy when the backup fails;
- never recreate the server when migrations fail;
- stop the daemon before migrations;
- keep the previous image and backup until readiness succeeds;
- do not automatically upgrade PostgreSQL with the app;
- keep deployment logs;
- image rollback does not roll back database migrations.

Use backward-compatible, expand/contract migrations. Destructive schema changes
require a separate maintenance procedure and a tested restore path.

### Step 7.6 — GitHub image publishing

The release workflow triggers on version tags:

```text
vX.Y.Z
```

Publish to GitHub Container Registry:

```text
ghcr.io/gocpicnic/aiqiezi-server:vX.Y.Z
ghcr.io/gocpicnic/aiqiezi-server:vX.Y.Z-YYYYMMDDHHMMSS
ghcr.io/gocpicnic/aiqiezi-server:latest
```

Build flow:

```text
1. GitHub Actions runner (linux/amd64)
2. cargo build --release --target x86_64-unknown-linux-musl
3. copy binary to dist/
4. docker build (Debian slim runtime + entrypoint)
5. docker push to GHCR for version tags
6. docker save the version, dated, and latest tags
7. attach the image archive, compose.yaml, and SHA256SUMS to the GitHub Release
```

Requirements:

- release builds happen only for `v*` tags;
- NAS/manual deployments use `latest`; keep dated tags for rollback;
- publish an offline `docker save` archive for NAS hosts without GHCR access;
- never deploy `main` or untagged images;
- use `GITHUB_TOKEN` with least privilege;
- no QEMU-based ARM builds initially;
- ARM64 support, when needed, uses a native ARM64 runner in a build matrix;
- require approval for production deployment.

### Step 7.7 — systemd backup and health timers

Use systemd on the remote server for scheduled operations, not automatic code
updates.

Suggested units:

```text
aiqiezi-backup.service
aiqiezi-backup.timer
aiqiezi-healthcheck.service
aiqiezi-healthcheck.timer
aiqiezi-media-check.service
aiqiezi-media-check.timer
```

Suggested cadence:

- PostgreSQL backup: daily or hourly;
- health check: every 5 minutes;
- media checksum/integrity check: weekly;
- retention cleanup: daily.

Deployment remains a controlled manual action until migrations and rollback are
proven.

### Step 7.8 — Watchtower evaluation

Watchtower is optional and must only be added after:

- migrations are backward-compatible;
- automatic backups are verified;
- health checks work;
- rollback is tested;
- production keeps dated tags for rollback; NAS/manual deployments may use
  `latest`.

Constraints:

- watch only the server container;
- exclude PostgreSQL;
- use `latest` for NAS upgrades and keep dated tags for rollback;
- run a backup before updates;
- verify readiness after updates;
- retain the previous image for rollback;
- enable notifications.

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
- whitelist failures;
- admin Unix socket protocol;
- admin request/response envelopes and error codes.

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

Implemented:

- Phase 0 — foundation, configuration, and logging;
- Phase 0B — CI foundation and repository metadata;
- Phase 1 — iroh endpoint, protocol envelopes, and JSON handling;
- Phase 1B — instance lock, admin socket, shutdown ordering, and admin tests;
- Phase 1C — bootstrap password, login/change-password, Argon2id storage, and
  EndpointId firewall;
- Docker runtime image, Compose stack, and deployment documentation.

Not implemented yet:

- Phase 2 PostgreSQL foundation and migrations;
- Phase 3 PostgreSQL-backed client whitelist administration;
- media storage, iroh-blobs, asset metadata, and read APIs.

Next: Phase 2 — PostgreSQL foundation.
