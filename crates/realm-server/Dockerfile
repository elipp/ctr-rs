# syntax=docker/dockerfile:1
# Build from the workspace root: docker build -f crates/realm-server/Dockerfile .
FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The cache mounts keep the registry and the target dir between builds, so only what changed is
# compiled again. (The binary is copied out in the same step, as the target dir is not in the
# image.)
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git-db,target=/usr/local/cargo/git/db \
    --mount=type=cache,id=realm-target,target=/src/target \
    cargo build --release --locked -p realm-server \
    && strip -o /realm-server target/release/realm-server

FROM debian:trixie-slim
# The CA certificates are for iroh's relay and address lookup, which are https.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data realm \
    && mkdir /data && chown realm /data
COPY --from=build /realm-server /usr/local/bin/realm-server
USER realm
WORKDIR /data
# The key, which decides the realm's id, lives here.
VOLUME /data
EXPOSE 4747/udp
ENTRYPOINT ["realm-server", "--key", "/data/realm.key"]
CMD ["--port", "4747"]
