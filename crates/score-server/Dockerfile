# syntax=docker/dockerfile:1
# Build from the workspace root: docker build -f crates/score-server/Dockerfile .
FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The cache mounts keep the registry and the target dir between builds. (The binary is copied out
# in the same step, as the target dir is not in the image.)
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git-db,target=/usr/local/cargo/git/db \
    --mount=type=cache,id=score-target,target=/src/target \
    cargo build --release --locked -p score-server \
    && strip -o /score-server target/release/score-server

FROM debian:trixie-slim
# The CA certificates are for iroh's relay and address lookup, which are https.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10002 --home-dir /data score \
    && mkdir /data && chown score /data
COPY --from=build /score-server /usr/local/bin/score-server
USER score
WORKDIR /data
# The key, which decides the server's id, and scores.db (with its -wal and -shm files: back up
# with `sqlite3 .backup`, not by copying).
VOLUME /data
EXPOSE 4748/udp
ENTRYPOINT ["score-server", "--key", "/data/score.key", "--db", "/data/scores.db", "--maps", "/maps"]
CMD ["--port", "4748"]
