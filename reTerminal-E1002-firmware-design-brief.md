# Design Brief: Custom Rust Firmware for Seeed reTerminal E1002

## 1. Overview

Build custom firmware in Rust to render text and layouts to the reTerminal E1002's
e-paper display, replacing Seeed's stock application firmware.

In addition to the on-device firmware, this project includes:

- A **browser-based preview/emulator** that renders exactly what the panel will show,
  before anything is flushed to real hardware.
- A **local-network content delivery model**: the device pulls new screen content from
  a server on the local network (no internet dependency, no cloud service).

**Target hardware:** Seeed Studio reTerminal E1002
Product page: https://www.seeedstudio.com/reTerminal-E1002-p-6533.html

## 2. Hardware Specifications

| Component | Detail |
|---|---|
| MCU | Espressif ESP32-S3R8 (WiFi/Bluetooth) |
| Display panel | Good Display GDEP073E01, 7.3", 800x480 |
| Display technology | E Ink Spectra 6 (6-color: Black, White, Red, Yellow, Blue, Green — **no Orange**, despite using the 7-color ACeP-style 4bpp encoding) |
| Display controller | ED2208-GCA (per Zephyr's mainline board port) |
| Refresh behavior | Full-screen flash refresh only — no partial/fast refresh mode |
| RTOS support | Official Zephyr board support exists, in addition to Arduino/ESP-IDF |

**Known gotcha:** Some community driver crates label this panel "7-color." Confirm
any driver dependency correctly models it as Spectra 6 (6-color) before relying on
palette behavior — sending the 7th (Orange) palette index produces undefined output
on this panel.

**Note on Bluetooth:** the ESP32-S3 has a BLE 5 radio, but this project deliberately
does not use it — see §10 (Content Delivery) for why WiFi-only, pull-based fetch was
chosen instead.

## 3. Reference Documentation

- Schematics (PDF): `https://files.seeedstudio.com/wiki/reterminal_e10xx/res/202004321_reTerminal_E1002_V1_2_SCH_251120.pdf`
- Getting started wiki: `https://wiki.seeedstudio.com/getting_started_with_reterminal_e1002/`
- Arduino cookbook (reference for driver/pin behavior, even though we're not using Arduino): `https://wiki.seeedstudio.com/reterminal_e10xx_with_arduino/`
- Zephyr board docs (source of truth for controller ID `eink,ed2208-gca`): `https://docs.zephyrproject.org/latest/boards/seeed/reterminal_e1002/doc/index.html`
- Seeed's open firmware examples (photo frame w/ web UI, REST API, Home Assistant, LVGL/PlatformIO): `Seeed-Projects/OSHW-reTerminal-Series-E-D` on GitHub
- Community Arduino examples using standard GxEPD2: `melastmohican/reTerminal-E1002-arduino-examples` on GitHub

**Note on Seeed's own ecosystem:** the stock/reference firmware in the repo above
exposes a REST API / web UI *on* the device (device acts as server). This project
intentionally inverts that model — see §10 — so the device can stay in deep sleep
except during a brief periodic fetch.

## 4. Proposed Toolchain

- **Language:** Rust, `no_std`, bare-metal
- **HAL:** `esp-hal` (esp-rs ecosystem) targeting ESP32-S3, using `embedded-hal` 1.0 traits (`SpiDevice`, `OutputPin`, `InputPin`, `DelayNs`)
- **PSRAM:** Enable OPI PSRAM — needed for framebuffer headroom at 800x480x4bpp (~192KB full-frame)
- **Networking (WiFi client only):**
  - `esp-radio` (formerly `esp-wifi`) — no_std WiFi/BLE driver for Espressif chips; used here in WiFi station mode only
  - `embassy-net` — no_std, no-alloc async TCP/IP + DNS stack, paired with `esp-radio`'s WiFi driver
  - `reqwless` — no_std async HTTP client on top of `embassy-net`; used for plain HTTP `GET` only (no TLS needed — see §10, LAN-only, no internet)

This resolves the toolchain question from the original brief: `esp-hal` bare-metal
stays viable for WiFi connectivity (no need to fall back to `esp-idf-hal`/std), because
`esp-radio` + `embassy-net` + `reqwless` is a proven no_std stack for exactly this
"connect, GET, disconnect" usage pattern.

## 5. Display Driver Layer

Two existing Rust crates target this panel directly — evaluate both before writing any custom driver code:

1. **`gdep073e01`** — `embedded-graphics`-native driver exposing `Gdep073e01::new(spi, cs, dc, rst, busy, delay)`, `.init()`, `.clear()`, and standard drawing primitives out of the box.
2. **`epdsi`** — more general `no_std` EPD framework; includes an explicit `ED2208` controller implementation and `GDEP073E01` panel spec, cross-referenced against GxEPD2 and the Zephyr board port. Supports GxEPD2-style paged/closure-based rendering with small stack buffers (useful to avoid a full 192KB framebuffer allocation).

**Action item:** Verify current state of the 6-color (Spectra 6) vs 7-color (ACeP) handling in `gdep073e01`'s source/issues. `epdsi` explicitly documents the Spectra-6-vs-ACeP-7 distinction in its `ColorMode`/`SevenColor` types, so it may be the safer default if `gdep073e01` is ambiguous.

Both implement `embedded-graphics`'s `DrawTarget` trait, so all rendering code above the driver layer is portable between them — and, per §7, portable to the browser preview too.

## 6. Rendering & Layout Stack

Build UI composition on `embedded-graphics` primitives plus:

| Need | Crate | Notes |
|---|---|---|
| Basic text | `embedded-graphics` built-in `MonoTextStyle` + fonts | Single-line, cursor-based |
| Wider font choice | `u8g2-fonts` | For varied sizes/weights (headers vs. body) |
| Word wrap / paragraph text | `embedded-text` (`TextBox`) | Horizontal/vertical alignment, justification |
| Multi-element layout (rows/columns, alignment) | `embedded-layout` | `LinearLayout`, `Chain`/`Views`, `Left/Right/Center`, `TopToBottom`/`BottomToTop` |
| Off-screen composition before flush | `embedded-canvas` | Compose regions before writing to the panel draw target |
| Bitmap/icon assets | `tinybmp` or `tinyqoi` | Pre-convert assets to BMP/QOI at build time; avoid on-device JPEG/PNG decoding — **deferred past v1, see §8** |

**Optional heavier alternative:** a retained-mode UI framework (Slint, or LVGL via
`lvgl-rs` bindings) instead of hand-composed layouts. Slint has a genuine no_std/
bare-metal backend and declarative layout syntax, but a ready-made backend wired to
this specific panel's `DrawTarget` has not been confirmed — treat as a spike, not a
default. LVGL via `lvgl-rs` means bridging a C library rather than staying pure-Rust
(Seeed's own stock firmware uses LVGL, for reference).

**Recommendation:** start with `embedded-graphics` + `embedded-layout` +
`embedded-text`. This covers dashboard-style layouts (headers, text blocks, icons,
grids) without pulling in a full UI framework, and composes directly with either
display driver crate above, and with the wasm preview backend in §7.

## 7. Software Architecture: Shared Crate, Multiple Targets

To make the browser preview a true WYSIWYG match for the physical panel — same
layout math, same 6-color palette quantization/dithering — the rendering logic must
be compiled for two targets from **one** source, not reimplemented per-target
(e.g. once in Rust, once in JS). Proposed workspace split:

| Crate | Contents | Target(s) |
|---|---|---|
| `screen-spec` | Serde types for the screen content contract (§8); JSON (de)serialization via `serde-json-core` (no_std, no-alloc) | shared |
| `render` | Composition logic: turns a `screen-spec` value into drawn primitives via `embedded-graphics` + `embedded-layout` + `embedded-text`, generic over any `DrawTarget`; owns the Spectra-6 palette quantization/dithering | shared, no_std |
| `panel-backend` | Wires `render`'s output to the real `gdep073e01`/`epdsi` driver over SPI | ESP32-S3 firmware only |
| `web-preview` | Implements `DrawTarget` by writing into an in-memory pixel buffer, then blits to an HTML `<canvas>` via `wasm-bindgen`/`web-sys` | `wasm32-unknown-unknown` |
| `firmware` | Binary crate: `esp-hal` init, deep-sleep/wake handling, `render` + `panel-backend`, WiFi fetch cycle (§10) | ESP32-S3 firmware only |

Because `render` and `screen-spec` never depend on hardware or `wasm-bindgen`, the
exact same compiled logic (down to per-pixel color quantization) runs in the browser
and on-device — the preview cannot silently drift from what actually gets flushed to
the panel.

**Prior art, not a dependency:** `embedded-graphics-web-simulator` demonstrates the
canvas-via-`wasm-bindgen` technique, but it's a generic, largely unmaintained
simulator (circa 2019) with no awareness of a custom 6-color palette. `web-preview`
should be a small (~100-200 line) purpose-built backend instead.

**Nice-to-have:** since the real panel's only refresh mode is a full-screen flash
(§9), `web-preview` can optionally play a brief flash/flicker animation before
settling on the final image, so the preview also communicates *how* an update will
visually feel on hardware, not just the end state.

## 8. Screen Content Model ("Screen Spec")

A single JSON document is the shared contract consumed by the browser preview, the
firmware's fetch cycle, and hand-authored/curl-able test content from the local
server. **v1 is text + layout only** — no embedded bitmaps (icon/image support is
deferred to a later milestone, alongside milestone 3's icon work in §9).

Illustrative shape (exact schema to be finalized during implementation):

```json
{
  "version": 1,
  "regions": [
    { "rect": [0, 0, 800, 80], "text": "Kitchen Display", "style": "header", "align": "center" },
    { "rect": [0, 100, 800, 380], "text": "Bin day: Thursday\nNext appointment: 14:00", "style": "body", "align": "left" }
  ]
}
```

- Parsed no_std/no-alloc via `serde-json-core` in the `screen-spec` crate.
- JSON was chosen (over a compact binary format like `postcard`) specifically so the
  local server can be written in any language and content can be hand-crafted or
  `curl`'d during development, at the cost of a slightly larger payload — irrelevant
  at "one screen's worth of text" size.
- `version` exists from day one so the schema can evolve (e.g. adding bitmap regions
  later) without silently breaking older firmware or preview builds — see open
  questions in §13.

## 9. Browser-Based Preview / Emulator

A static web page (built via `wasm-pack` or `trunk`) that:

1. Loads a `screen-spec` JSON document (pasted, loaded from a local file, or fetched
   from the same local dev server the device polls — see §10).
2. Runs it through the `render` crate compiled to `wasm32-unknown-unknown`.
3. Draws the result to an HTML `<canvas>` via the `web-preview` backend (§7).

This requires no hardware to be connected and no panel flush to happen — it's meant
to be the fast iteration loop for "what will this screen spec actually look like,"
including correct 6-color quantization, before ever waking the real device.

## 10. Content Delivery: Local-Network Pull Model

**Model:** the device is an HTTP **client**, never a server. On each wake, it:

1. Joins the configured WiFi network (station mode).
2. Issues a single plain-HTTP `GET` to a configured local server address (e.g. the
   developer's laptop during development) for the current `screen-spec` JSON.
3. Parses it, runs it through `render` + `panel-backend`, flushes to the panel.
4. Tears down the WiFi connection and returns to deep sleep (§12).

**Why pull, not push, and why no internet/BLE:**

- No internet dependency: the server the device talks to is on the local network
  only (during development, the developer's own laptop, address supplied later) —
  no cloud service, no plain-HTTP-over-internet exposure to worry about.
- No TLS needed, since traffic never leaves the LAN — simplifies the `reqwless`
  usage (no `embedded-tls`/`mbedtls` dependency).
- Pull-based beats the device hosting its own REST API/web UI (as Seeed's stock
  firmware does) for this project's power budget: hosting a server means holding a
  listening socket (and usually the whole WiFi stack) up indefinitely, which fights
  the deep-sleep-first design in §12. A brief outbound `GET` per wake keeps the
  radio's on-time to a few seconds.
- Bluetooth (BLE) was considered as an alternative low-power transport but is out of
  scope for this project: WiFi-only was chosen deliberately, so BLE host-stack
  questions (`trouble-host` vs `bleps` vs `esp32-nimble`, MTU/chunking, etc.) are not
  pursued here.

**Wake sources feeding the fetch cycle:** timer interval (periodic poll) and/or a
GPIO/button press (on-demand manual refresh) — both trigger the same
fetch → render → flush → sleep routine from §12.

**Dev workflow:** any lightweight HTTP server on the developer's laptop serving the
current `screen-spec` JSON at a fixed path (e.g. `GET /screen`) is sufficient — the
implementation language of that server doesn't matter, since JSON is the contract.
The device's target IP/port is supplied later (see open questions, §13).

## 11. Refresh Model & Design Constraint

This panel only supports full-screen flash refresh (multi-second), not fast partial
refresh. Layout and rendering logic should be designed around a **"compose once,
flush once"** model:

- Build the complete frame in memory (or via paged rendering) before any call to the
  panel's flush/display method.
- Do not design for incremental/animated redraws — the hardware doesn't support it
  well and each refresh is visually a full-panel flash.

## 12. Power Management

- After compose + flush, put the ESP32-S3 into deep sleep via `esp-hal`'s deep-sleep
  API.
- Wake on timer or GPIO/button interrupt, then run the fetch cycle in §10 before
  composing/flushing.
- The display holds its image with zero power once refreshed — this is the basis of
  the multi-month battery life target — so sleep/wake cycling (now including a brief
  WiFi fetch window per cycle) is central to the power budget, not an optional
  optimization.

## 13. Suggested Milestones

1. **Bring-up:** SPI init, panel init/reset sequence, full-white and full-black clear via chosen driver crate.
2. **Hello world:** Single string rendered via `embedded-graphics` `MonoTextStyle`, flushed to panel.
3. **Layout composition:** Multi-element screen (header + body text block + one icon) via `embedded-layout` + `embedded-text`.
4. **Screen spec + shared `render` crate:** define the v1 JSON schema (§8), implement composition against a generic `DrawTarget`, validate against milestone 3's layout using a hand-authored spec file.
5. **Browser preview:** `web-preview` wasm/canvas backend (§7, §9); confirm its output visually matches what milestone 3 produces on real hardware for the same spec.
6. **WiFi fetch cycle:** `esp-radio` + `embassy-net` + `reqwless` GET against a local dev server, parse into `screen-spec`, render, flush — exercised against a laptop-hosted test server.
7. **Power cycle:** Compose → flush → deep sleep → timer/button wake → fetch → repeat, with battery draw measured across a full cycle including the WiFi fetch window.
8. **(Stretch)** Evaluate Slint or LVGL spike if declarative layout authoring becomes worth the added dependency weight.
9. **(Stretch, post-v1)** Bitmap/icon regions in the screen spec, using `tinybmp`/`tinyqoi`.

## 14. Open Questions for Implementation

- [ ] Confirm `gdep073e01` crate's color handling matches Spectra 6, or standardize on `epdsi`.
- [ ] Confirm pin mapping for the E1002 carrier board (SPI, DC, RST, BUSY) against the schematic PDF linked above.
- [ ] Determine target font(s) and whether `u8g2-fonts` is needed or built-in `embedded-graphics` fonts suffice.
- [ ] Finalize the v1 `screen-spec` JSON schema (region model, text styles, alignment options) beyond the illustrative example in §8.
- [ ] Decide the schema evolution/versioning policy once bitmap regions are added post-v1 (reject unknown `version`? best-effort ignore unknown fields?).
- [ ] Decide WiFi credential + local server IP/port provisioning mechanism for non-dev use (dev can hardcode/env-bake; production likely needs a USB-serial config tool or similar — no BLE-based provisioning, per §10).
- [ ] Decide default poll interval and retry/backoff behavior when a fetch fails (e.g. keep last-rendered content and retry next wake vs. render an error state).
