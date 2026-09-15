# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Custom Rust firmware for the Seeed reTerminal E1002 (ESP32-S3 + 7.3" 6-color
e-paper panel), plus supporting tooling: a browser-based WYSIWYG preview and a
local-network content server. The full design rationale, hardware details,
and open questions live in `reTerminal-E1002-firmware-design-brief.md` —
read it before making architectural changes; this file only covers what's
needed to build/navigate day to day.

## Commands

Bare `cargo` commands (no `--workspace` flag) only touch the workspace's
`default-members` — `screen-spec`, `render`, `server`, `web-preview` — which
all build with a normal host Rust toolchain. `firmware` and `panel-backend`
are excluded (see "The Xtensa/firmware constraint" below).

- `cargo check` — typecheck the default-members crates
- `cargo fmt --all --check` — format check (matches CI)
- `cargo clippy --all-targets -- -D warnings` — lint (matches CI)
- `cargo test` — run tests across default-members
- `cargo test -p <crate>` — test a single crate (`screen-spec`, `render`, `server`, or `web-preview`)
- `cargo clippy -p <crate> -- -D warnings` — lint a single crate

To touch `firmware`/`panel-backend` explicitly (will fail here, see below):
`cargo check -p firmware -p panel-backend --target xtensa-esp32s3-none-elf`.

## The Xtensa/firmware constraint

The ESP32-S3 is an Xtensa core. Its Rust target *spec* is upstream, but
Xtensa code generation is not — building for it requires the esp-rs fork of
rustc+LLVM, installed via `espup`. `espup` downloads from
`objects.githubusercontent.com` and `dl.espressif.com`, both blocked by this
sandbox's egress policy (confirmed by direct testing, not assumption).

Consequences:
- `firmware` and `panel-backend` are workspace members but are **not** in
  `default-members` (root `Cargo.toml`) — this is deliberate, not an
  oversight, so routine `cargo check`/`clippy`/`test` don't fail on them.
- They're compile-checked in CI instead: `.github/workflows/ci.yml`'s
  `firmware` job installs the Xtensa toolchain via the
  `esp-rs/xtensa-toolchain` GitHub Action on a GitHub-hosted runner (not
  behind this proxy).
- Don't "fix" a failure to build these crates in a Claude Code session by
  trying to install espup, work around the proxy, or loosen the workspace
  config — that failure is expected here. See `crates/firmware/README.md`.

## Architecture

Six-crate Cargo workspace, structured so the rendering logic is shared
between real hardware and a browser preview instead of reimplemented per
target (design brief §7):

| Crate | Role | Depends on | Builds here? |
|---|---|---|---|
| `screen-spec` | Serde types for the JSON "screen spec" content contract (§8) | — | yes |
| `render` | Composes a `screen-spec` into drawn primitives via `embedded-graphics`, generic over any `DrawTarget`; owns the 6-color (Spectra 6) palette quantization | `screen-spec` | yes |
| `panel-backend` | Wires `render`'s output to the real GDEP073E01 panel driver over SPI | `render` | **no** — Xtensa-only |
| `web-preview` | `DrawTarget` impl that blits to an HTML `<canvas>` via `wasm-bindgen`; compiled to `wasm32-unknown-unknown` | `render` | yes (host target; no wasm-only deps yet) |
| `server` | Native binary: holds current `screen-spec`, serves `GET /screen` (device fetches this) and `PUT /screen` (how content gets updated), hosts the `web-preview` UI | `screen-spec` | yes |
| `firmware` | ESP32-S3 binary: `esp-hal` init, deep-sleep/wake, WiFi fetch cycle, `render` + `panel-backend` | `render`, `panel-backend` | **no** — Xtensa-only |

Key architectural decisions (see design brief for full rationale):
- **One render crate, two `DrawTarget` backends.** `render` never depends on
  hardware or `wasm-bindgen`; `panel-backend` and `web-preview` are thin
  adapters. This is what makes the browser preview an actual WYSIWYG match
  for the panel rather than an approximation.
- **Device is an HTTP client, not a server.** On wake it does one `GET
  /screen` against `server`, renders, flushes, sleeps — no internet, no
  Bluetooth, no device-hosted listener (that would fight the deep-sleep
  power budget).
- **`server` is both the content host and the preview host** — the preview
  page fetches from the same `GET /screen` route the device uses, so the
  browser and the device are never looking at different content.

All crates other than `firmware` are currently placeholder stubs (`#![no_std]`
markers and doc comments, no real logic) — implementation follows the
milestones in the design brief's §13.
