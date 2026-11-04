#!/bin/sh
#
# docker/entrypoint.sh
# Copyright 2026 aiqiezi. All rights reserved.
# Use of this source code is governed by the AGPL-3.0
# license that can be found in the LICENSE file.
#
set -eu

DATA_DIR="${AIQIEZI_DATA_DIR:-/data}"
RUNTIME_DIR="${AIQIEZI_RUNTIME_DIR:-/run/aiqiezi}"
UID_VALUE="${AIQIEZI_UID:-1000}"
GID_VALUE="${AIQIEZI_GID:-1000}"

validate_id() {
    value="$1"
    name="$2"

    case "$value" in
        ''|*[!0-9]*)
            echo "error: $name must be a numeric ID, got '$value'" >&2
            exit 1
            ;;
    esac

    if [ "$value" -eq 0 ]; then
        echo "error: $name must not be root (0)" >&2
        exit 1
    fi
}

validate_id "$UID_VALUE" AIQIEZI_UID
validate_id "$GID_VALUE" AIQIEZI_GID

mkdir -p \
    "$DATA_DIR/keys" \
    "$DATA_DIR/media" \
    "$DATA_DIR/config" \
    "$RUNTIME_DIR"

chmod 0700 "$DATA_DIR/keys"
chmod 0750 "$DATA_DIR/media" "$DATA_DIR/config" "$RUNTIME_DIR"

# Keys and runtime files contain sensitive state and should always belong to
# the application user. Media is only chowned at the directory level because
# it can contain a very large number of files.
chown -R "$UID_VALUE:$GID_VALUE" \
    "$DATA_DIR/keys" \
    "$DATA_DIR/config" \
    "$RUNTIME_DIR"
chown "$UID_VALUE:$GID_VALUE" "$DATA_DIR/media"

umask 027

GOSU_BIN="$(command -v gosu || true)"
if [ -z "$GOSU_BIN" ] || [ ! -x "$GOSU_BIN" ]; then
    echo "error: gosu is not installed" >&2
    exit 1
fi

exec "$GOSU_BIN" "$UID_VALUE:$GID_VALUE" \
    /usr/local/bin/aiqiezi-server "$@"
