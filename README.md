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

Planned commands once the workspace is scaffolded:

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
sqlx migrate run
cargo run -p aiqiezi-server
```

## Status

Early design phase. The workspace, server, protocol, and database schema are
not implemented yet.

## Documentation

- [AGENTS.md](AGENTS.md): implementation rules for AI coding agents
- `docs/architecture.md`: deeper architecture notes
- `docs/protocol.md`: iroh JSON command definitions
- `docs/database.md`: schema and migration notes
