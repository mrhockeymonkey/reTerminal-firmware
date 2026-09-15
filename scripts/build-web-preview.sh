#!/usr/bin/env bash
# Builds the `web-preview` crate for wasm32-unknown-unknown and copies the
# module into `crates/server/assets/`, where `server` serves it (and, in
# release builds, embeds it into the binary via rust-embed).
#
# Usage: scripts/build-web-preview.sh [--debug]
set -euo pipefail

cd "$(dirname "$0")/.."

profile=release
profile_flag=--release
if [[ "${1:-}" == "--debug" ]]; then
  profile=debug
  profile_flag=""
fi

rustup target list --installed | grep -q '^wasm32-unknown-unknown$' \
  || rustup target add wasm32-unknown-unknown

# The workspace release profile keeps debug info for the firmware's
# backtraces; that is dead weight in a wasm module, so strip it here.
# shellcheck disable=SC2086
cargo build -p web-preview --target wasm32-unknown-unknown $profile_flag \
  --config 'profile.release.debug=false' \
  --config 'profile.release.strip=true'

src="target/wasm32-unknown-unknown/$profile/web_preview.wasm"
dst="crates/server/assets/web_preview.wasm"
cp "$src" "$dst"
echo "wrote $dst ($(wc -c < "$dst") bytes)"
