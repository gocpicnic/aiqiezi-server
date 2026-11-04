<!--
  deploy/README.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Production deployment

The current server is Phase 1B: it provides the iroh endpoint and local admin
socket. PostgreSQL is included in the stack for Phase 2, but the server does
not use it yet.

## Build and publish the image

Build the static binary and image on a development machine:

```bash
make docker-push APP_VERSION=v0.1.0
```

The image tag includes the current UTC build timestamp, for example:

```text
ghcr.io/gocpicnic/aiqiezi-server:v0.1.0-20261104123045
```

Record the exact tag printed by the build and use it as `AIQIEZI_TAG` in the
production `.env`. Never deploy `latest` to production.

To build without pushing and push later, pass the exact tag explicitly:

```bash
make docker-image APP_VERSION=v0.1.0
# copy the printed tag
make docker-push IMAGE_TAG=v0.1.0-20261104123045
```

If the server cannot pull from GHCR, save and copy the image instead:

```bash
make docker-save APP_VERSION=v0.1.0
# copy the exact generated tag from the output
scp imagefiles/aiqiezi-server-v0.1.0-20261104123045.tar user@server:/tmp/
```

On the remote server:

```bash
docker load -i /tmp/aiqiezi-server-v0.1.0-20261104123045.tar
```

## Deploy on the remote server

Create the deployment directory and copy the deployment files:

```bash
mkdir -p /mnt/data/aiqiezi/deploy
cd /mnt/data/aiqiezi/deploy
```

Copy `compose.yaml` and `deploy/.env.example` from this repository:

```bash
cp /path/to/aiqiezi-server/compose.yaml .
cp /path/to/aiqiezi-server/deploy/.env.example .env
chmod 600 .env
```

Edit `.env` and replace every `change-me` value. Set:

```text
AIQIEZI_TAG
AIQIEZI_UID
AIQIEZI_GID
AIQIEZI_DATABASE_URL
POSTGRES_PASSWORD
```

`AIQIEZI_TAG` must be the exact tag printed by `make docker-push` or
`make docker-save`, including the timestamp suffix. The PostgreSQL password
must match `AIQIEZI_DATABASE_URL`. Use a URL-safe password or percent-encode it
in the URL.

If using GHCR:

```bash
docker login ghcr.io
```

Start the stack:

```bash
docker compose pull
docker compose up -d
docker compose ps
```

The server uses host networking. PostgreSQL is published only on
`127.0.0.1:5432`; it is not exposed to the public network.

## Verify the daemon

```bash
docker compose exec aiqiezi-server aiqiezi-server status
docker compose exec aiqiezi-server aiqiezi-server healthcheck
```

Logs:

```bash
docker compose logs -f aiqiezi-server
docker compose logs -f postgres
```

Stop the service cleanly:

```bash
docker compose stop aiqiezi-server
```

The admin socket `shutdown` command also stops the process cleanly, but the
Compose restart policy will start the container again. Use `docker compose
stop` when the service should remain stopped.

The admin socket is inside the container at `/run/aiqiezi/admin.sock` and is
not exposed over TCP or iroh.

## Update and rollback

Use the exact dated image tag for every deployment:

```bash
# set AIQIEZI_TAG in .env to the new vX.Y.Z-YYYYMMDDHHMMSS tag
docker compose pull aiqiezi-server
docker compose up -d aiqiezi-server
```

Rollback by setting the previous tag in `.env`, then repeating the pull and
recreate commands. Database migrations are not automatic and are not rolled
back with the image; use backward-compatible migrations once Phase 2 is
implemented.

## Data layout

Application data is mounted individually so the server cannot access the
PostgreSQL data directory:

```text
/mnt/data/aiqiezi/media  -> /data/media
/mnt/data/aiqiezi/keys   -> /data/keys
/mnt/data/aiqiezi/config -> /data/config
/mnt/data/aiqiezi/postgres -> PostgreSQL data
```

The entrypoint creates and chowns the application directories. PostgreSQL
owns its own data directory. If PostgreSQL reports an ownership error, a
one-time host-side `chown 999:999 /mnt/data/aiqiezi/postgres` is acceptable;
this is troubleshooting, not a required setup step.

Back up the iroh key. Losing it changes the server EndpointId.
