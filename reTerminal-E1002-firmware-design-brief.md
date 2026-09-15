# Design Brief: Custom Rust Firmware for Seeed reTerminal E1002

## 1. Overview

Build custom firmware in Rust to render text and layouts to the reTerminal E1002's
e-paper display, replacing Seeed's stock application firmware.

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

## 3. Reference Documentation

- Schematics (PDF): `https://files.seeedstudio.com/wiki/reterminal_e10xx/res/202004321_reTerminal_E1002_V1_2_SCH_251120.pdf`
- Getting started wiki: `https://wiki.seeedstudio.com/getting_started_with_reterminal_e1002/`
- Arduino cookbook (reference for driver/pin behavior, even though we're not using Arduino): `https://wiki.seeedstudio.com/reterminal_e10xx_with_arduino/`
- Zephyr board docs (source of truth for controller ID `eink,ed2208-gca`): `https://docs.zephyrproject.org/latest/boards/seeed/reterminal_e1002/doc/index.html`
- Seeed's open firmware examples (photo frame w/ web UI, REST API, Home Assistant, LVGL/PlatformIO): `Seeed-Projects/OSHW-reTerminal-Series-E-D` on GitHub
- Community Arduino examples using standard GxEPD2: `melastmohican/reTerminal-E1002-arduino-examples` on GitHub

## 4. Proposed Toolchain

- **Language:** Rust, `no_std`, bare-metal
- **HAL:** `esp-hal` (esp-rs ecosystem) targeting ESP32-S3, using `embedded-hal` 1.0 traits (`SpiDevice`, `OutputPin`, `InputPin`, `DelayNs`)
  - Alternative if WiFi/BLE integration becomes a priority: `esp-idf-hal` (std, sits on ESP-IDF/FreeRTOS) — heavier, but simpler networking story
- **PSRAM:** Enable OPI PSRAM — needed for framebuffer headroom at 800x480x4bpp (~192KB full-frame)

## 5. Display Driver Layer

Two existing Rust crates target this panel directly — evaluate both before writing any custom driver code:

1. **`gdep073e01`** — `embedded-graphics`-native driver exposing `Gdep073e01::new(spi, cs, dc, rst, busy, delay)`, `.init()`, `.clear()`, and standard drawing primitives out of the box.
2. **`epdsi`** — more general `no_std` EPD framework; includes an explicit `ED2208` controller implementation and `GDEP073E01` panel spec, cross-referenced against GxEPD2 and the Zephyr board port. Supports GxEPD2-style paged/closure-based rendering with small stack buffers (useful to avoid a full 192KB framebuffer allocation).

**Action item:** Verify current state of the 6-color (Spectra 6) vs 7-color (ACeP) handling in `gdep073e01`'s source/issues. `epdsi` explicitly documents the Spectra-6-vs-ACeP-7 distinction in its `ColorMode`/`SevenColor` types, so it may be the safer default if `gdep073e01` is ambiguous.

Both implement `embedded-graphics`'s `DrawTarget` trait, so all rendering code above the driver layer is portable between them.

## 6. Rendering & Layout Stack

Build UI composition on `embedded-graphics` primitives plus:

| Need | Crate | Notes |
|---|---|---|
| Basic text | `embedded-graphics` built-in `MonoTextStyle` + fonts | Single-line, cursor-based |
| Wider font choice | `u8g2-fonts` | For varied sizes/weights (headers vs. body) |
| Word wrap / paragraph text | `embedded-text` (`TextBox`) | Horizontal/vertical alignment, justification |
| Multi-element layout (rows/columns, alignment) | `embedded-layout` | `LinearLayout`, `Chain`/`Views`, `Left/Right/Center`, `TopToBottom`/`BottomToTop` |
| Off-screen composition before flush | `embedded-canvas` | Compose regions before writing to the panel draw target |
| Bitmap/icon assets | `tinybmp` or `tinyqoi` | Pre-convert assets to BMP/QOI at build time; avoid on-device JPEG/PNG decoding |

**Optional heavier alternative:** a retained-mode UI framework (Slint, or LVGL via
`lvgl-rs` bindings) instead of hand-composed layouts. Slint has a genuine no_std/
bare-metal backend and declarative layout syntax, but a ready-made backend wired to
this specific panel's `DrawTarget` has not been confirmed — treat as a spike, not a
default. LVGL via `lvgl-rs` means bridging a C library rather than staying pure-Rust
(Seeed's own stock firmware uses LVGL, for reference).

**Recommendation:** start with `embedded-graphics` + `embedded-layout` +
`embedded-text`. This covers dashboard-style layouts (headers, text blocks, icons,
grids) without pulling in a full UI framework, and composes directly with either
display driver crate above.

## 7. Refresh Model & Design Constraint

This panel only supports full-screen flash refresh (multi-second), not fast partial
refresh. Layout and rendering logic should be designed around a **"compose once,
flush once"** model:

- Build the complete frame in memory (or via paged rendering) before any call to the
  panel's flush/display method.
- Do not design for incremental/animated redraws — the hardware doesn't support it
  well and each refresh is visually a full-panel flash.

## 8. Power Management

- After compose + flush, put the ESP32-S3 into deep sleep via `esp-hal`'s deep-sleep
  API.
- Wake on timer or GPIO/button interrupt.
- The display holds its image with zero power once refreshed — this is the basis of
  the multi-month battery life target — so sleep/wake cycling is central to the
  power budget, not an optional optimization.

## 9. Suggested Milestones

1. **Bring-up:** SPI init, panel init/reset sequence, full-white and full-black clear via chosen driver crate.
2. **Hello world:** Single string rendered via `embedded-graphics` `MonoTextStyle`, flushed to panel.
3. **Layout composition:** Multi-element screen (header + body text block + one icon) via `embedded-layout` + `embedded-text`.
4. **Power cycle:** Compose → flush → deep sleep → timer wake → repeat, with battery draw measured across a cycle.
5. **(Stretch)** Evaluate Slint or LVGL spike if declarative layout authoring becomes worth the added dependency weight.

## 10. Open Questions for Implementation

- [ ] Confirm `gdep073e01` crate's color handling matches Spectra 6, or standardize on `epdsi`.
- [ ] Decide `esp-hal` (bare-metal) vs `esp-idf-hal` (std/FreeRTOS) based on whether WiFi connectivity is in scope for v1.
- [ ] Confirm pin mapping for the E1002 carrier board (SPI, DC, RST, BUSY) against the schematic PDF linked above.
- [ ] Determine target font(s) and whether `u8g2-fonts` is needed or built-in `embedded-graphics` fonts suffice.
- [ ] Decide whether v1 needs any bitmap/icon assets, or text-only layouts are sufficient for first milestone.
