# syntax=docker/dockerfile:1
#
# Claude Code in a container: the project is mounted in, nothing else of the host is.
# Build and run with `sandbox/claude.sh` (see there).

FROM rust:1-slim-trixie

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    bash build-essential ca-certificates curl git jq less openssh-client pkg-config procps ripgrep unzip libwayland-dev libxkbcommon-dev libasound2-dev libudev-dev python3 gcc-multilib \
    && rm -rf /var/lib/apt/lists/*

# Same uid as the host user, so files written to the mounted project are yours.
ARG UID=501
ARG GID=20
RUN (getent group ${GID} >/dev/null || groupadd -g ${GID} claude) \
    && useradd -m -u ${UID} -g ${GID} -s /bin/bash claude

USER claude
WORKDIR /home/claude
ENV PATH=/home/claude/.local/bin:/home/claude/.cargo/bin:$PATH

# Claude Code, native installer (installs to ~/.local/bin/claude). Updates come by rebuilding
# the image with `--pull --no-cache`.
RUN curl -fsSL https://claude.ai/install.sh | bash

RUN curl -LsSf https://astral.sh/uv/install.sh | sh

# `CLAUDE_CONFIG_DIR` puts both `.claude/` and `.claude.json` (login, settings, history) in one
# directory, which the run script keeps in a named volume. `CARGO_HOME` is set because the base
# image's (`/usr/local/cargo`) is not on a volume: the registry and git volumes are mounted under
# `~/.cargo`, and cargo would not use them.
ENV CLAUDE_CONFIG_DIR=/home/claude/.claude-state \
    DISABLE_AUTOUPDATER=1 \
    CARGO_HOME=/home/claude/.cargo \
    CARGO_TARGET_DIR=/target \
    UV_PROJECT=/work/sandbox \
    UV_PROJECT_ENVIRONMENT=/venv \
    UV_PYTHON_DOWNLOADS=never

# Volume mount points, made here so they belong to `claude` and not root.
USER root
RUN mkdir -p /home/claude/.claude-state /home/claude/.cargo/registry /home/claude/.cargo/git /home/claude/.cache/uv /target /venv /work \
    && chown claude:$(id -gn claude) /home/claude/.claude-state /home/claude/.cargo/registry /home/claude/.cargo/git /home/claude/.cache /home/claude/.cache/uv /target /venv /work
USER claude

WORKDIR /work
ENTRYPOINT ["claude"]
