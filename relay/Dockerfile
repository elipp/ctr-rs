# syntax=docker/dockerfile:1
# Build from this directory: docker build .
# The relay is iroh's own server, at the version the clients link against (see crates/net).
FROM rust:1-slim-trixie AS build
ARG IROH_RELAY_VERSION=1.4.1
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git-db,target=/usr/local/cargo/git/db \
    cargo install iroh-relay --version ${IROH_RELAY_VERSION} --locked --features server \
        --root /out

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libcap2-bin gettext-base \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data relay \
    && mkdir /data && chown relay /data
COPY --from=build /out/bin/iroh-relay /usr/local/bin/iroh-relay
# The relay runs as a plain user but binds 80 and 443.
RUN setcap cap_net_bind_service=+ep /usr/local/bin/iroh-relay
COPY relay.toml.tmpl /etc/iroh-relay.toml.tmpl
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
USER relay
WORKDIR /data
# The Let's Encrypt account and certificate live here, so a restart does not ask for new ones
# (the rate limits are tight).
VOLUME /data
# HTTPS on 443, HTTP on 80; QUIC address discovery on UDP 7842.
EXPOSE 80/tcp 443/tcp 7842/udp
ENTRYPOINT ["entrypoint.sh"]
