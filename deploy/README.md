<!--
  deploy/README.md
  Copyright 2026 aiqiezi. All rights reserved.
  Use of this source code is governed by the AGPL-3.0
  license that can be found in the LICENSE file.
-->

# Production deployment

The current server provides the iroh endpoint, bootstrap password
authentication, EndpointId whitelist enforcement, and the local admin socket.
PostgreSQL is not part of the current stack; it will be added in Phase 2.

Deployment uses one self-contained `compose.yaml`. The deployed file contains
secrets, so set it to mode `0600` and never commit it.

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

`APP_VERSION` is read from the `aiqiezi-server` Cargo package version. Every
build tags both a dated image and `latest`:

```text
ghcr.io/gocpicnic/aiqiezi-server:v0.1.0-20261104123045
ghcr.io/gocpicnic/aiqiezi-server:latest
```

The Compose file uses `latest` for simple NAS upgrades. Keep the dated tags for
rollback. `make docker-push` publishes both tags. To refresh only `latest`:

```bash
make docker-push-latest
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

The saved archive contains both the dated tag and `latest`.

## Deploy on the remote server

Create the deployment directory and copy `compose.yaml`:

```bash
mkdir -p /mnt/data/aiqiezi
cd /mnt/data/aiqiezi
cp /path/to/aiqiezi-server/compose.yaml .
chmod 600 compose.yaml
```

Edit `compose.yaml` directly. Replace every placeholder value, especially:

```text
AIQIEZI_UID
AIQIEZI_GID
AIQIEZI_BOOTSTRAP_PASSWORD
```

`AIQIEZI_BOOTSTRAP_PASSWORD` is used only when no password hash exists. It must
meet the password policy: 8-128 characters with uppercase, lowercase, digit,
and special character. The first client login must change it. After that
change, the value is ignored.

The default image is:

```text
ghcr.io/gocpicnic/aiqiezi-server:latest
```

For a pinned deployment, replace it with the exact dated tag printed by the
build.

If using GHCR:

```bash
docker login ghcr.io
```

Validate and start the stack:

```bash
docker compose config
docker compose up -d
docker compose ps
```

The server uses host networking. PostgreSQL is not running.

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
`AIQIEZI_BOOTSTRAP_PASSWORD` in `compose.yaml`, and start the server again.
This clears the whitelist; every client must enroll again.

Alternatively, terminal users can run the offline command while the server is
stopped:

```bash
docker compose stop aiqiezi-server
docker compose run --rm -it aiqiezi-server aiqiezi-server set-password
docker compose up -d aiqiezi-server
```

## Verify the daemon

```bash
docker compose exec aiqiezi-server aiqiezi-server --version
docker compose exec aiqiezi-server aiqiezi-server status
docker compose exec aiqiezi-server aiqiezi-server healthcheck
```

`aiqiezi-server` is the Compose service name. `docker compose exec` runs the
command in the existing running container; `docker compose run` starts a
separate one-off container.

Logs:

```bash
docker compose logs -f aiqiezi-server
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

The default Compose file uses `latest`:

```bash
docker compose pull aiqiezi-server
docker compose up -d aiqiezi-server
```

For an offline image transfer:

```bash
docker load -i /tmp/aiqiezi-server-<tag>.tar
docker compose up -d --force-recreate aiqiezi-server
```

Rollback by changing the image line in `compose.yaml` to the previous dated tag,
then pulling or loading that image and recreating the container. PostgreSQL
migrations do not exist yet; Phase 2 will define them.

## Data layout

Application data is mounted individually:

```text
/mnt/data/aiqiezi/media  -> /data/media
/mnt/data/aiqiezi/keys   -> /data/keys
/mnt/data/aiqiezi/config -> /data/config
```

The entrypoint creates and chowns the application directories. PostgreSQL will
get its own data directory when it is added in Phase 2.

Back up the iroh key and `keys/auth.json`. Losing the iroh key changes the
server EndpointId; losing `auth.json` forces a bootstrap-password reset and
clears the client whitelist.
