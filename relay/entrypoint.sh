#!/bin/sh
# Fills the hostname and the contact of the relay's config in from the environment.
set -eu
: "${RELAY_HOSTNAME:?set RELAY_HOSTNAME}" "${RELAY_CONTACT:?set RELAY_CONTACT}"
envsubst '${RELAY_HOSTNAME} ${RELAY_CONTACT}' < /etc/iroh-relay.toml.tmpl > /tmp/iroh-relay.toml
exec iroh-relay --config-path /tmp/iroh-relay.toml
