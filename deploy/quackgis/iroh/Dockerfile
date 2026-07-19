FROM docker.io/library/rust:1.91.0-bookworm@sha256:e187887ec511b3d93e45c0231d2f0fd59f1347526c58aa86343aa83c74f3e1a9 AS build

ARG DUMBPIPE_VERSION=0.39.0

RUN cargo install dumbpipe --version "$DUMBPIPE_VERSION" --locked

FROM docker.io/library/debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local/cargo/bin/dumbpipe /usr/local/bin/dumbpipe
COPY listen.sh connect.sh /usr/local/bin/

USER 65534:65534
