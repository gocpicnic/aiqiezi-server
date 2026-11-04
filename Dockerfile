# Build the static binary outside Docker first:
#   make build-static
#
# Then build the runtime image:
#   make docker-image IMAGE_TAG=v0.1.0
#
# This image intentionally contains no Rust toolchain and no HTTP listener.
# The Makefile builds it with --platform linux/amd64.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       gosu \
       tini \
    && rm -rf /var/lib/apt/lists/*

COPY dist/aiqiezi-server /usr/local/bin/aiqiezi-server
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 0755 \
    /usr/local/bin/aiqiezi-server \
    /usr/local/bin/entrypoint.sh

ENV AIQIEZI_DATA_DIR=/data \
    AIQIEZI_RUNTIME_DIR=/run/aiqiezi

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
CMD ["run"]
