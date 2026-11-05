<!--
  deploy/README.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Production deployment

The current server provides the iroh endpoint, bootstrap password
authentication, EndpointId whitelist enforcement, and the local admin socket.
PostgreSQL is included in the stack for Phase 2, but the server does not use it
yet.

## Supported host

- Linux host with Docker Engine and Compose v2.
- `linux/amd64` server image.
- Host networking for the server service.
- Docker Desktop on macOS or Windows is not supported.
- ARM64 hosts are not supported yet.

## Build and publish the image

Build the static binary and image on a development machine:

```bash
make docker-push
```

`APP_VERSION` is read from the `aiqiezi-server` Cargo package version. The
image tag also includes the current UTC build timestamp, for example:

```text
ghcr.io/gocpicnic/aiqiezi-server:v0.1.0-20261104123045
```

The same build also tags `ghcr.io/gocpicnic/aiqiezi-server:latest` for manual
and development testing. Record the exact dated tag printed by the build and
use it as `AIQIEZI_TAG` in the production `.env`. Never deploy `latest` to
production.

If needed, override the Cargo-derived version explicitly:

```bash
make docker-push APP_VERSION=v0.1.0-rc1
```

To build without pushing and push later, pass the exact tag explicitly:

```bash
make docker-image
# copy the printed tag
make docker-push IMAGE_TAG=v0.1.0-20261104123045
```

If the server cannot pull from GHCR, save and copy the image instead:

```bash
make docker-save
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

Edit `.env` and replace every placeholder value. Set:

```text
AIQIEZI_TAG
AIQIEZI_UID
AIQIEZI_GID
AIQIEZI_DATABASE_URL
AIQIEZI_BOOTSTRAP_PASSWORD
POSTGRES_PASSWORD
```

`AIQIEZI_TAG` must be the exact tag printed by `make docker-push` or
`make docker-save`, including the timestamp suffix. The PostgreSQL password
must match `AIQIEZI_DATABASE_URL`. Use a URL-safe password or percent-encode it
in the URL.

`AIQIEZI_BOOTSTRAP_PASSWORD` is used only when no password hash exists. It must
meet the password policy: 8-128 characters with uppercase, lowercase, digit,
and special character. The first client login must change it. After that
change, the value is ignored.

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

## First client enrollment

On the client, log in with `AIQIEZI_BOOTSTRAP_PASSWORD` and change it:

```bash
aiqiezi login
aiqiezi change-password
```

The first login returns `password_change_required` and does not grant access.
The password change enrolls the client EndpointId in the server whitelist. No
terminal on the server host is required.

Do not edit `keys/auth.json` manually. Use `change-password` from a client or
the offline `set-password` command.

If the password is lost, stop the server, remove
`/mnt/data/aiqiezi/keys/auth.json`, set a new
`AIQIEZI_BOOTSTRAP_PASSWORD`, and start the server again. This clears the
whitelist; every client must enroll again.

Alternatively, terminal users can run the offline command while the server is
stopped:

```bash
docker compose stop aiqiezi-server
docker compose run --rm -it aiqiezi-server aiqiezi-server set-password
docker compose up -d aiqiezi-server
```

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

Back up the iroh key and `keys/auth.json`. Losing the iroh key changes the
server EndpointId; losing `auth.json` forces a bootstrap-password reset and
clears the client whitelist.
