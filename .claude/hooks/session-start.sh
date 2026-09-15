#!/bin/bash
set -euo pipefail

cd "$CLAUDE_PROJECT_DIR"

# This workspace's `firmware` and `panel-backend` crates only build with the
# esp-rs Xtensa toolchain (installed via `espup`), which needs
# objects.githubusercontent.com and dl.espressif.com — both blocked by this
# sandbox's egress policy. They're excluded from the workspace's
# `default-members`, so nothing below touches them; see
# crates/firmware/README.md and .github/workflows/ci.yml (which builds them
# on a GitHub-hosted runner instead).

log=$(mktemp)
trap 'rm -f "$log"' EXIT

run() {
  if ! "$@" >>"$log" 2>&1; then
    echo "session-start: '$*' failed:" >&2
    cat "$log" >&2
    exit 1
  fi
}

# wasm32-unknown-unknown is needed for web-preview; rustup no-ops if present.
run rustup target add wasm32-unknown-unknown

# Warms the local cargo cache (registry index + crate sources) for the
# session; container caching means this is cheap on repeat runs.
run cargo fetch

# Bare `cargo check` (no --workspace) only touches default-members:
# screen-spec, render, server, web-preview.
run cargo check
