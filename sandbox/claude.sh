#!/usr/bin/env bash
# Runs Claude Code in a container with only this project mounted (see compose.yaml).
#
#   sandbox/claude.sh                 interactive session (first run: log in; it is kept)
#   sandbox/claude.sh -p "..."        arguments go to `claude`
#   sandbox/claude.sh --shell         a bash in the same container, to poke around
#   sandbox/claude.sh --build         (re)build the images first; --build-pull also updates Claude
#
# Persisted in named volumes: login/settings/history (claude-state), cargo registry and git
# (claude-cargo-*), uv's cache and virtualenv (claude-uv-cache, claude-venv), and the build output (claude-target; the host's `target` is not shared, as
# it holds Mac binaries). Mounted from the host: the project at /work, the disc directory
# read-only at /disc if it exists, and this project's Claude memory directory (read-write).
# Not mounted: ~/.ssh, ~/.aws, the rest of ~/.claude, ~/.gitconfig.
#
# Network: the container sits on an `--internal` network (no route out), and reaches the Claude
# and crates.io domains only through the `proxy` squid service (sandbox/squid.conf is the
# allowlist). The proxy keeps running between sessions; `docker compose -p claude-sandbox down`
# stops it.
set -euo pipefail

cd "$(dirname "$0")/.."
disc=${CTR_DISC_DIR:-$HOME/Games/CTR}
# Claude keys a project's memory by its path with `/` and `.` as `-`; in the container that is /work.
export CLAUDE_MEMORY=$HOME/.claude/projects/$(printf %s "$PWD" | tr "/." "--")/memory
export HOST_UID=$(id -u) HOST_GID=$(id -g)
export GIT_AUTHOR_NAME=$(git config user.name) GIT_AUTHOR_EMAIL=$(git config user.email)
export GIT_COMMITTER_NAME=$GIT_AUTHOR_NAME GIT_COMMITTER_EMAIL=$GIT_AUTHOR_EMAIL
mkdir -p "$CLAUDE_MEMORY"
# External volumes in compose.yaml, so they are made here (a no-op when they exist).
for v in claude-state claude-cargo-registry claude-cargo-git claude-target claude-uv-cache claude-venv; do docker volume create $v >/dev/null; done

compose=(docker compose -f sandbox/compose.yaml)
if [ -f "$disc/ctr-u.bin" ]; then
  export CTR_DISC_DIR=$disc
  compose+=(-f sandbox/compose.disc.yaml)
fi

case "${1:-}" in
  --build)      shift; "${compose[@]}" build ;;
  --build-pull) shift; "${compose[@]}" build --pull --no-cache ;;
esac

# Builds the images if they are missing; recreates the proxy if its image changed.
"${compose[@]}" up -d proxy
"${compose[@]}" run --rm state-init

if [ "${1:-}" = --shell ]; then
  exec "${compose[@]}" run --rm --entrypoint bash claude
fi
exec "${compose[@]}" run --rm claude "$@"
