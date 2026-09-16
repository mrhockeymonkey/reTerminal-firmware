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
`default-members` — `screen-spec`, `render`, `panel-backend`, `server`,
`web-preview` — which all build with a normal host Rust toolchain. `firmware`
is excluded (see "The Xtensa/firmware constraint" below).

- `cargo check` — typecheck the default-members crates
- `cargo fmt --all --check` — format check (matches CI)
- `cargo clippy --all-targets -- -D warnings` — lint (matches CI)
- `cargo test` — run tests across default-members
- `cargo test -p <crate>` — test a single crate (`screen-spec`, `render`, `panel-backend`, `server`, or `web-preview`)
- `cargo clippy -p <crate> -- -D warnings` — lint a single crate
- `scripts/build-web-preview.sh` — wasm32 release build of `web-preview`, copied to `crates/server/assets/web_preview.wasm` (git-ignored; `server` embeds it in release builds)
- `cargo run -p server -- --bind 127.0.0.1:8080` — content server + preview page; `curl -X PUT --data-binary @crates/screen-spec/samples/kitchen.json localhost:8080/screen`
- `cargo test -p render -- --ignored dump_ppm` — writes the sample renders as PPM under `target/tmp/` for eyeballing
- `cargo tree -p firmware --target xtensa-esp32s3-none-elf` — resolves the firmware's dependency graph on the host (works here; building does not)

Building `firmware` explicitly (will fail here, see below) is done from its
own directory so its `.cargo/config.toml` and `rust-toolchain.toml` apply:
`cd crates/firmware && cargo build --release`.

## The Xtensa/firmware constraint

The ESP32-S3 is an Xtensa core. Its Rust target *spec* is upstream, but
Xtensa code generation is not — building for it requires the esp-rs fork of
rustc+LLVM, installed via `espup`. `espup` downloads from
`objects.githubusercontent.com` and `dl.espressif.com`, both blocked by this
sandbox's egress policy (confirmed by direct testing, not assumption).

Consequences:
- `firmware` is a workspace member but is **not** in `default-members`
  (root `Cargo.toml`) — this is deliberate, not an oversight, so routine
  `cargo check`/`clippy`/`test` don't fail on it. (`panel-backend` *is*
  host-buildable: it only uses `embedded-hal` traits, and its tests drive it
  with fake SPI/GPIO.)
- It's compiled in CI instead: `.github/workflows/ci.yml`'s `firmware` job
  installs the Xtensa toolchain via the `esp-rs/xtensa-toolchain` GitHub
  Action on a GitHub-hosted runner (not behind this proxy) and does a real
  release link from `crates/firmware`. CI runs on pushes to `main` and
  `claude/**`, on PRs, and on manual dispatch; read the job logs with the
  GitHub MCP tools to iterate on firmware compile errors.
- Firmware configuration is baked in from env vars at build time
  (`WIFI_SSID`, `WIFI_PASSWORD`, `SCREEN_URL`, `POLL_INTERVAL_SECS`; see
  `crates/firmware/src/config.rs`), with placeholders so CI needs no secrets.
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
| `render` | `Spectra6` colour (native panel nibble codes), packed 800×480×4bpp `Frame`/`FrameMut`, preview RGB palette, and composition of a `screen-spec` via `embedded-graphics` + `embedded-text` + `u8g2-fonts`, generic over any `DrawTarget<Color = Spectra6>` | `screen-spec` | yes |
| `panel-backend` | `Panel` wrapping `epdsi`'s ED2208 driver: init/show/clear/sleep for the GDEP073E01, generic over `embedded-hal` 1.0 traits | `render` | yes (host-tested with fake SPI/GPIO) |
| `web-preview` | `render` compiled to `wasm32-unknown-unknown` behind a nine-function plain C ABI (`wp_*`), no `wasm-bindgen`; `crates/server/assets/preview.js` is the JS glue | `render` | yes (host `rlib` tests the ABI; wasm via `scripts/build-web-preview.sh`) |
| `server` | Native `axum` binary: `GET /screen` (ETag = FNV-1a content hash, `If-None-Match` → 304), `PUT`/`POST /screen` (validated with the device's parser, persisted to `--state-file`), preview page via `rust-embed` | `screen-spec` | yes |
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

Other decisions worth knowing before changing things (full list in
`docs/IMPLEMENTATION_PLAN.md`):
- **Palette codes** `Black=0x0 White=0x1 Yellow=0x2 Red=0x3 Blue=0x5 Green=0x6`;
  `0x4` (ACeP orange) and `0x7` are undisplayable on this Spectra 6 panel and
  `render::Spectra6` cannot represent them. Pixel packing is high nibble first.
- **Screen spec v1** is text + layout only: `version` must be `1`, ≤ 16
  regions, ≤ 512 bytes of text per region, ≤ 16 KiB document. Unknown fields
  are ignored. Text fields are `heapless::String` because serde-json-core can
  only unescape into owned strings.
- **Skip-unchanged**: the device hashes the body (FNV-1a, `screen_spec::content_hash`)
  and does not re-flush identical content; the server exposes the same hash as
  the ETag.
- **Frame memory on device** is a PSRAM `Vec` viewed through `render::FrameMut`;
  never construct `render::Frame` (192 KB) on an embedded stack.
- `panel-backend::Panel::clear` deliberately bypasses `epdsi`'s `clear_frame`,
  which sizes the 4bpp channel as 1bpp in epdsi 0.4.
