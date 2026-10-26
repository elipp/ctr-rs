# syntax=docker/dockerfile:1
#
# The only way out of the Claude sandbox: a forward proxy that tunnels HTTPS to the domains in
# `squid.conf` and refuses everything else. Started by `sandbox/claude.sh`.

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends squid \
    && rm -rf /var/lib/apt/lists/*

COPY squid.conf /etc/squid/squid.conf

USER proxy
EXPOSE 3128
# Foreground, so the container stays up and the log goes to `docker logs`.
ENTRYPOINT ["squid", "-N", "-f", "/etc/squid/squid.conf"]
