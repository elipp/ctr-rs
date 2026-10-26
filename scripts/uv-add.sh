#!/usr/bin/env bash
set -euo pipefail
# Human-only: the one place the container gets network access.
# Usage: scripts/uv-add requests "pandas>=2"
[ "$#" -ge 1 ] || { echo "usage: $0 <package>..." >&2; exit 1; }
exec docker run --rm \
  --user "$(id -u):$(id -g)" \
  --memory 2g --pids-limit 256 \
  -v "$PWD":/work \
  -v uv-cache:/cache \
  -w /work \
  -e UV_CACHE_DIR=/cache \
  -e UV_PYTHON_DOWNLOADS=never \
  -e HOME=/tmp \
  --tmpfs /tmp \
  ghcr.io/astral-sh/uv:debian \
  uv add "$@"
