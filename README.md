# aiqiezi-server

A durable, searchable, single-backend archive for large photo and video
collections that grow over many years.

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
- home server, NAS, or desktop;
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
    | iroh JSON + iroh-blobs
    v
aiqiezi-server
    |
    +-- PostgreSQL: metadata only
    +-- content-addressed media storage: photos/videos
```

The server is the single source of truth. Clients never access PostgreSQL or
the server filesystem directly.

## Main components

- **PostgreSQL**: metadata, folders, tags, search indexes, client whitelist.
- **Media storage**: photos and videos in a content-addressed tree.
- **iroh JSON**: control and metadata commands.
- **iroh-blobs**: media upload and download.
- **Whitelist**: authorization by client EndpointId.

## Security model

There is no login, password, or session-token flow. Clients are identified by
their persistent iroh EndpointId and authorized by a server-side whitelist.

First enrollment:

```bash
# On the client, print its EndpointId
aiqiezi whoami

# On the server host, authorize it
aiqiezi-server client-add <endpoint-id> --label "laptop"
```

Revocation:

```bash
aiqiezi-server client-disable <endpoint-id>
aiqiezi-server client-remove <endpoint-id>
```

The whitelist is enforced at the iroh connection layer and protects both JSON
and blob access.

Initial blob access is a trusted-whitelist model: any whitelisted client may
fetch any blob if it knows the content hash. Per-asset ACLs are deferred until
multi-user access is required.

## Data layout

Production data lives outside the repository:

```text
/mnt/data/aiqiezi/
├── media/
├── postgres/
├── keys/
├── config/
├── backups/
└── logs/
```

The iroh secret key must be backed up. Losing it changes the backend EndpointId
and requires every client to be reconfigured.

## Development

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Run the Phase 0 server skeleton:

```bash
AIQIEZI_DATABASE_URL='postgres://user:pass@127.0.0.1/aiqiezi' \
  cargo run -p aiqiezi-server -- run
```

## Container image

The server binary is built outside Docker as a static musl binary:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
mkdir -p dist
cp target/x86_64-unknown-linux-musl/release/aiqiezi-server dist/
docker build -t gocpicnic/aiqiezi-server:dev .
```

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

Initial target is `linux/amd64`. ARM64 support is deferred until there is a
real ARM deployment target.

## Status

Phase 0 foundation implemented:

- Cargo workspace and protocol crate
- configuration loading
- structured logging
- `run` command skeleton

PostgreSQL, iroh, protocol handlers, and media storage are not implemented yet.
Next phase: CI foundation.

## Documentation

- [AGENTS.md](AGENTS.md): implementation rules for AI coding agents
- [docs/implementation_plan.md](docs/implementation_plan.md): staged server implementation plan
- `docs/architecture.md`: deeper architecture notes
- `docs/protocol.md`: iroh JSON command definitions
- `docs/database.md`: schema and migration notes
