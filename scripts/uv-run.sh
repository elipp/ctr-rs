#!/usr/bin/env bash
set -euo pipefail
  
exec docker run --rm \
  --network none \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --tmpfs /tmp \
  --user "$(id -u):$(id -g)" \
  --memory 2g --pids-limit 256 \
  -v "$PWD":/work \
  -v uv-cache:/cache \
  -w /work \
  -e UV_CACHE_DIR=/cache \
  -e UV_OFFLINE=1 \
  -e UV_PYTHON_DOWNLOADS=never \
  -e HOME=/tmp \
  ghcr.io/astral-sh/uv:debian \
  uv "$@"
