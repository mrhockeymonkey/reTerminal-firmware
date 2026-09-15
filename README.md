# reTerminal-firmware

Custom Rust firmware for the Seeed **reTerminal E1002** (ESP32-S3 + 7.3"
six-colour E Ink Spectra 6 panel), plus the two tools that make it usable
without touching the hardware every time: a **browser preview** that renders
exactly what the panel will show, and a **local-network content server** the
device pulls its screen from.

```text
PUT /screen (curl)  ─►  server (axum, screen.json)  ─►  GET /screen  ─►  device (WiFi, every 15 min / button)
                                 │                                            │
                                 ▼                                            ▼
                  browser: preview.js + web_preview.wasm          screen-spec → render → panel-backend
                           (same `render` crate, canvas)          (same `render` crate, e-paper)
```

Content is a small JSON "screen spec" (text regions with fonts, alignment,
colours, borders); see `crates/screen-spec/samples/kitchen.json` and the
schema in `crates/screen-spec/src/lib.rs`. Design rationale lives in
`reTerminal-E1002-firmware-design-brief.md`; the implementation plan and
decisions in `docs/IMPLEMENTATION_PLAN.md`.

## Quick start (no hardware needed)

```sh
scripts/build-web-preview.sh                 # wasm32 build of the renderer → crates/server/assets/
cargo run -p server -- --bind 127.0.0.1:8080 # serves /screen and the preview page
```

Open <http://127.0.0.1:8080/> for the preview, then push content:

```sh
curl -X PUT -H 'content-type: application/json' \
     --data-binary @crates/screen-spec/samples/kitchen.json \
     http://127.0.0.1:8080/screen
```

The page re-renders within two seconds; the textarea on the page can also
edit and `PUT` the document directly. Invalid documents are rejected with the
same parser error the device would hit. The current document is persisted to
`screen.json` (`--state-file`) so it survives restarts. To let the device
reach the server, bind to the LAN: `--bind 0.0.0.0:8080`.

## Flashing the device

See `crates/firmware/README.md`: install the esp-rs toolchain with `espup`,
then from `crates/firmware`:

```sh
WIFI_SSID=… WIFI_PASSWORD=… SCREEN_URL=http://<laptop-ip>:8080/screen cargo run --release
```

On each wake the device fetches `/screen`, skips the (20 s, full-flash)
refresh when the content hash is unchanged, otherwise renders and flushes,
then deep-sleeps until the next timer tick or a press of the Refresh button.

## Repository layout

| Crate | Role | Host-buildable |
|---|---|---|
| `screen-spec` | JSON contract (serde, `no_std`, no alloc) | yes |
| `render` | Spectra 6 palette, packed 4bpp frame, composition via `embedded-graphics` + `embedded-text` + `u8g2-fonts` | yes |
| `panel-backend` | `render` frame → GDEP073E01 over SPI (`epdsi`), generic over `embedded-hal` | yes (tested with fake SPI) |
| `web-preview` | `render` compiled to wasm32 behind a tiny C ABI | yes (+ wasm32 target) |
| `server` | `GET`/`PUT /screen`, preview page, state file | yes |
| `firmware` | ESP32-S3 binary: WiFi fetch cycle, PSRAM frame, deep sleep | Xtensa toolchain only (CI) |

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test                                   # everything except firmware
cargo build -p web-preview --target wasm32-unknown-unknown --release
cargo test -p render -- --ignored dump_ppm   # writes sample renders to target/tmp/*.ppm
```

CI runs the same checks plus a real Xtensa link of `firmware`.
