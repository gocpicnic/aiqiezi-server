# aiqiezi-server

[![CI](https://github.com/gocpicnic/aiqiezi-server/actions/workflows/ci.yml/badge.svg)](https://github.com/gocpicnic/aiqiezi-server/actions/workflows/ci.yml)

Self-hosted server for managing large personal photo and video collections.

Like [PhotoPrism](https://github.com/photoprism/photoprism) and
[Immich](https://github.com/immich-app/immich), it is built for self-hosted
photo libraries, but aiqiezi is built on
[iroh](https://github.com/n0-computer/iroh), with
[iroh-blobs](https://github.com/n0-computer/iroh-blobs) providing native
resumable uploads and downloads. Users can access the server over the network
without a static IP, public domain name, port forwarding, or reverse proxy.

## Why iroh

The target user self-hosts on a home server, NAS, or desktop behind NAT. They
do not have a static public IP, a stable public domain, port forwarding, a
public TLS certificate, or a reverse proxy.

Unlike PhotoPrism-style HTTP deployments, this project uses iroh as the network
layer:

- the backend is addressed by its persistent EndpointId;
- clients connect through iroh JSON;
- media transfers use iroh-blobs;
- direct connections are preferred, with relay fallback;
- no public HTTP listener, domain, or TLS certificate is required.

This is the primary architectural difference from HTTP-first photo servers.

## Target deployment

- single backend;
- Linux home server, NAS, or desktop running Docker Engine with Compose;
- x86_64 (`linux/amd64`) container image;
- no Docker Desktop on macOS or Windows;
- no native server build on Windows or macOS;
- behind NAT;
- dynamic public IP;
- no public domain or port forwarding required;
- optional custom iroh relay and DNS server;
- native CLI and egui GUI clients;
- initial target: one owner/administrator.

## Non-goals

Unless explicitly added later:

- peer-to-peer client synchronization;
- multi-backend or federated operation;
- offline multi-writer conflict resolution;
- public social sharing;
- media editing;
- storing media bytes in PostgreSQL;
- exposing an HTTP API;
- a browser-only client.

## Architecture

```text
CLI / GUI
    |
    | public iroh endpoint
    | JSON + iroh-blobs
    v
aiqiezi-server daemon
    |
    +-- PostgreSQL: metadata only
    +-- content-addressed media storage: photos/videos

server admin CLI
    |
    | /run/aiqiezi/admin.sock
    v
same aiqiezi-server daemon
```

There is one daemon process with two listeners:

- a public iroh endpoint for clients;
- a local Unix admin socket for server commands.

Both listeners share the same database pool, media storage, whitelist cache,
and shutdown coordinator. Do not run two daemon processes against the same
database and media store.

The server is the single source of truth. Clients never access PostgreSQL or
the server filesystem directly.

## Main components

- **PostgreSQL**: metadata, folders, tags, search indexes, client whitelist.
- **Media storage**: photos and videos in a content-addressed tree.
- **iroh JSON**: client control and metadata commands.
- **iroh-blobs**: media upload and download.
- **Authentication**: bootstrap password, login, and password change over
  `/aiqiezi-auth/1`.
- **Whitelist**: authorization by client EndpointId.
- **Admin socket**: local-only server commands over a Unix domain socket.
- **Instance lock**: prevents two daemons from sharing one data directory.

## Security model

Initial client enrollment uses a bootstrap password configured on the server:

```text
AIQIEZI_BOOTSTRAP_PASSWORD
```

The first login returns `password_change_required`. The client must change the
password over `/aiqiezi-auth/1` before it is enrolled. The password change
stores only an Argon2id hash, clears the whitelist, and enrolls the caller's
EndpointId. After that change, the bootstrap value is ignored.

There is no session-token flow. Enrolled clients are identified by their
persistent iroh EndpointId and authorized by the server whitelist.

First enrollment:

```bash
# Set AIQIEZI_BOOTSTRAP_PASSWORD in the server .env first.

# On the client
aiqiezi login
aiqiezi change-password
aiqiezi whoami
```

The auth ALPN is accepted before enrollment. The JSON and blob ALPNs are
accepted only for whitelisted EndpointIds. Clients must perform login and
password change with their persistent iroh secret key; the server enrolls the
EndpointId of the connection.

Revocation:

```bash
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

The whitelist is file-backed until Phase 3 migrates client state to
PostgreSQL. It is enforced at the iroh connection layer and protects both JSON
and blob access.

Initial blob access is a trusted-whitelist model: any whitelisted client may
fetch any blob if it knows the content hash. Per-asset ACLs are deferred until
multi-user access is required.

## Administration

Server administration uses a local Unix domain socket:

```text
/run/aiqiezi/admin.sock
```

The admin socket is local-only. It is never exposed through iroh or TCP.

The socket and lock file live under `AIQIEZI_RUNTIME_DIR`:

```text
/run/aiqiezi/admin.sock
/run/aiqiezi/server.lock
```

For a manually run development server, the Makefile uses `./data/run` if
`AIQIEZI_RUNTIME_DIR` is not set.

Inside Docker:

```bash
docker compose exec aiqiezi-server aiqiezi-server status
docker compose exec aiqiezi-server aiqiezi-server healthcheck
docker compose exec aiqiezi-server aiqiezi-server shutdown
```

`shutdown` exits the process; with the Compose restart policy Docker may start
it again. Use `docker compose stop aiqiezi-server` when it should stay stopped.

`status`, `healthcheck`, and `shutdown` are implemented. Bootstrap login,
`change-password`, Argon2id password storage, and the EndpointId firewall are
implemented. Database, migration, and media-storage fields currently report
`not_configured` or `database_unavailable` until Phase 2/3. The `client-add`,
`client-list`, `client-disable`, `client-enable`, and `client-remove` commands
are deferred until Phase 3.

The daemon owns all writes. Client whitelist changes take effect for subsequent
connections without restarting the daemon.

For terminal-based recovery or offline rotation, stop the daemon and run:

```bash
docker compose stop aiqiezi-server
docker compose run --rm -it aiqiezi-server aiqiezi-server set-password
docker compose up -d aiqiezi-server
```

`set-password` holds the instance lock and clears the whitelist. NAS and
Compose users normally use `change-password` from the client instead and do not
need a terminal on the server host.

Migrations are a one-shot operation outside the running daemon:

```bash
docker compose stop aiqiezi-server
docker compose run --rm aiqiezi-server aiqiezi-server migrate
docker compose up -d aiqiezi-server
```

The daemon refuses to start when migrations are pending.

## Data layout

Production data lives outside the repository:

```text
/mnt/data/aiqiezi/
├── media/
├── postgres/
├── keys/
│   └── auth.json       # Argon2id hash + temporary client whitelist
├── config/
├── backups/
└── logs/
```

Runtime-only files:

```text
/run/aiqiezi/admin.sock
/run/aiqiezi/server.lock
```

For local development, set `AIQIEZI_RUNTIME_DIR=./data/run` or let the
Makefile use that fallback.

The iroh secret key and `keys/auth.json` must be backed up. Losing the key
changes the backend EndpointId and requires every client to be reconfigured.
Losing `auth.json` forces a bootstrap-password reset and clears the client
whitelist.

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Or use the Makefile:

```bash
make env       # create .env from .env.example once
# edit .env with your data directory and database URL
make run       # build and run the server
```

While the server is running, use another terminal:

```bash
make status
make healthcheck
make shutdown
```

Other useful targets: `make help`, `make run-debug`, `make build-static`,
`make version`, `make fmt`, `make check`, `make test`, `make clippy`, and
`make ci`.

`make build` and `make build-static` set `BUILD_DATE`, which is embedded in
the binary. Check it with:

```bash
make version
# aiqiezi-server 0.1.0 (build 20261104123045)
```

Run the daemon manually:

```bash
AIQIEZI_DATABASE_URL='postgres://user:pass@127.0.0.1/aiqiezi' \
AIQIEZI_BOOTSTRAP_PASSWORD='ChangeMe-123!' \
AIQIEZI_RUNTIME_DIR='./data/run' \
  cargo run -p aiqiezi-server -- start
```

The Makefile does the same and falls back to `./data/run` when
`AIQIEZI_RUNTIME_DIR` is unset. On first start the server creates
`keys/secret_key.bin`, writes `keys/endpoint_ticket.txt`, and stores the
bootstrap password hash and must-change state in `keys/auth.json` under
`AIQIEZI_DATA_DIR`. PostgreSQL is not opened yet; the database URL is validated
but reserved for Phase 2.

Before the client CLI exists, the development example can exercise the auth
protocol against a running server:

```bash
AIQIEZI_PASSWORD='...' AIQIEZI_NEW_PASSWORD='...' \
  cargo run -p aiqiezi-server --example auth_client -- \
  --ticket-file ./data/keys/endpoint_ticket.txt \
  --data-dir ./data/auth-client flow
```

## Container image

The server binary is built outside Docker as a static musl binary:

```bash
make docker-push
```

The build reads the package version from Cargo metadata and appends the UTC
build timestamp, for example:

```text
ghcr.io/gocpicnic/aiqiezi-server:v0.1.0-20261104123045
```

Use that exact tag in the production `AIQIEZI_TAG`.

Use `make docker-save` when the remote server cannot pull from GHCR.

The runtime image uses Debian slim because the container entrypoint needs a
shell, `tini`, and `gosu`. Rust compilation still happens outside Docker.

The entrypoint starts as root only long enough to:

1. create the app-owned data directories,
2. change ownership of those directories,
3. drop privileges,
4. start the server as `AIQIEZI_UID:AIQIEZI_GID`.

Default UID/GID:

```text
AIQIEZI_UID=1000
AIQIEZI_GID=1000
```

Users with a different host UID/GID set these values in `.env`. No setup script
is required. The entrypoint never touches PostgreSQL data.

Initial target is `linux/amd64` on a Linux Docker Engine host. ARM64 support is
deferred until there is a real ARM deployment target. Docker Desktop on macOS
or Windows and native server builds on those platforms are not supported.

Production Compose instructions are in [deploy/README.md](deploy/README.md).
The stack uses host networking for the server, a private bridge for PostgreSQL,
and mounts only the media, keys, and config directories into the app container.

## Status

Phase 1, Phase 1B, and bootstrap authentication are implemented:

- persistent EndpointId key and endpoint ticket
- relay, DNS, and optional bind-address configuration
- SIGINT/SIGTERM shutdown with router and endpoint cleanup
- versioned JSON protocol envelopes in `aiqiezi-proto`
- JSON Lines `server-info` and `status` commands over
  `/aiqiezi-json/1`
- bootstrap password login and `change-password` over `/aiqiezi-auth/1`
- Argon2id password storage, must-change state, login rate limiting, and
  EndpointId whitelist enforcement at the iroh firewall
- exclusive daemon instance lock
- local Unix admin socket with restrictive permissions
- `status`, `healthcheck`, and `shutdown` admin commands
- Docker runtime image, Compose stack, and production deployment guide
- in-process transport and lock/admin tests

PostgreSQL, asset metadata, media storage, iroh-blobs, and admin-managed client
whitelist commands are not implemented yet. Client authorization is currently
file-backed and bootstrap-enrolled. Next phase: Phase 2 PostgreSQL foundation.

## Documentation

- [AGENTS.md](AGENTS.md): implementation rules for AI coding agents
- [SECURITY.md](SECURITY.md): vulnerability reporting policy
- [docs/implementation_plan.md](docs/implementation_plan.md): staged server implementation plan
- [deploy/README.md](deploy/README.md): production Docker Compose deployment
- [docs/branch-protection.md](docs/branch-protection.md): repository branch protection
- `docs/architecture.md`: deeper architecture notes
- `docs/protocol.md`: iroh JSON command definitions
- `docs/database.md`: schema and migration notes
