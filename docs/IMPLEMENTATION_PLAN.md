# Implementation plan: reTerminal E1002 Rust firmware + preview + server

## Context

The repo is a six-crate Cargo workspace whose crates are all placeholder stubs
(doc comments and `#![no_std]` markers only), plus a design brief that fixes the
architecture: one `render` crate shared between the real panel and a browser
preview, a JSON "screen spec" as the content contract, a LAN-only `server` the
device pulls from, and a deep-sleep-first ESP32-S3 firmware. The brief leaves
ten open questions (§14) and names candidate crates without pinning them.

This plan resolves those questions from primary sources (crate sources
downloaded from crates.io, Zephyr's board devicetree, esp-generate's template;
docs.rs and the Seeed wiki are blocked here), pins a coherent dependency set,
finalises the v1 screen-spec schema, and lays out the implementation in seven
phases ordered so that everything host-buildable (with tests) lands before the
Xtensa-only firmware, which can only be compile-checked in CI and validated on
real hardware by the user.

Environment constraints that shape the plan:
- `firmware` cannot be built in this sandbox (no Xtensa toolchain, by design —
  see CLAUDE.md). It is compile-checked by CI's `firmware` job only. Hardware
  bring-up (brief milestones 1–3, 7) is the user's step; the plan gives a checklist.
- crates.io is reachable, so dependencies can be added and host crates tested here.
- `wasm32-unknown-unknown` target is installed, but no `wasm-bindgen-cli`/`wasm-pack`.

## Decisions (resolving brief §5, §6, §14)

| Question | Decision | Why |
|---|---|---|
| Panel driver crate | **`epdsi` 0.4** (`Ed2208Controller` + `GDEP073E01` panel) | Models Spectra 6 explicitly (`SevenColor` docs + panel doc say Orange is undefined here); `write_frame(ColorChannel::Color7(0), &[u8])` accepts a pre-packed 4bpp frame, which we need anyway for the preview; BUSY handled active-low (matches devicetree `GPIO_ACTIVE_LOW`). `gdep073e01` 0.4 polls `while busy.is_high()` (inverted for this panel), forces `alloc` + `Box<[u8]>` buffer, and exposes a 7-colour `Color` with `Orange`. |
| Palette / nibble codes | `render::Spectra6 { Black=0x0, White=0x1, Yellow=0x2, Red=0x3, Blue=0x5, Green=0x6 }`; 0x4 (orange) and 0x7 never emitted | Same codes in epdsi, gdep073e01, GxEPD2 and Zephyr; pixel packing = high nibble first (`SevenColor::pack(high, low)`). |
| Preview RGB values | Zephyr board palette: Black (0,0,0), White (245,245,240), Yellow (230,200,40), Red (196,50,58), Blue (40,85,180), Green (60,140,75) | Panel-measured-looking values from the mainline board port; stored once in `render::palette` and exported to the wasm/JS side, so preview and firmware share one table. |
| Fonts | `u8g2-fonts` 0.8 (`embedded_graphics_textstyle` feature) for 4 presets: title `u8g2_font_logisoso42_tf`, header `u8g2_font_helvB24_tf`, body `u8g2_font_helvR18_tf`, small `u8g2_font_helvR12_tf` (all verified present in the crate) | Built-in `embedded-graphics` mono fonts top out at 10×20 px, too small on an 800×480 wall display. |
| Word wrap / alignment | `embedded-text` 0.7 `TextBox` with `U8g2TextStyle` | Verified in source: `U8g2TextStyle<C>` implements both `TextRenderer` and `CharacterStyle`, which is exactly `TextBox::new`'s bound; gives wrapping + `HorizontalAlignment`/`VerticalAlignment`. |
| `embedded-layout`, `embedded-canvas` | Not used in v1 | Regions carry absolute rects; nothing to lay out. Can be added later without schema change. |
| Screen-spec parsing | `serde` (derive, no_std) + `heapless` 0.9 strings; device parses with `serde_json_core::from_slice_escaped`; server with `serde_json` | Escaped strings (`"\n"`) only reach `&str` fields as *borrowed* in serde-json-core, which fails after unescaping; `heapless::String<N>` fields work on both sides. |
| Schema versioning | `version` must be `1`; otherwise render an "unsupported spec version" error screen. Unknown fields ignored (serde default). | Forward-compatible for the post-v1 bitmap region. Test pins the "unknown field ignored" behaviour under serde-json-core. |
| web-preview ⇄ JS glue | **Plain `extern "C"` exports + ~80 lines of hand-written JS**, no `wasm-bindgen`/`web-sys`. Built with `cargo build -p web-preview --target wasm32-unknown-unknown --release`. | Removes the `wasm-bindgen-cli` toolchain + exact-version lockstep; the ABI is five functions over two static buffers. (Deviation from brief §7's "via wasm-bindgen" — the brief permits adaptation.) |
| wasm assets into `server` | `rust-embed` 8 over `crates/server/assets/` (`index.html`, `preview.js` checked in; `web_preview.wasm` produced by `scripts/build-web-preview.sh`, git-ignored). Debug builds read the folder from disk; release embeds → single binary. | Brief §9 option 2, without a fragile `build.rs` that shells out to cargo. |
| `server` persistence | Yes: `--state-file <path>` (default `./screen.json`); written atomically on every accepted `PUT`, loaded at start, falls back to a built-in sample spec. | Cheap; survives laptop restarts. |
| `PUT /screen` access control | None in v1; bind `127.0.0.1` by default, `--bind 0.0.0.0:8080` to expose on the LAN. | LAN-only dev tool; documented in README. |
| WiFi credentials + server address | Build-time env: `WIFI_SSID`, `WIFI_PASSWORD`, `SCREEN_URL` (e.g. `http://192.168.1.20:8080/screen`), optional `POLL_INTERVAL_SECS` (default 900). Read via `option_env!` with placeholder defaults so CI compiles without secrets. | Brief §14 allows env-baking for dev; a USB-serial config tool is a later milestone. |
| Poll interval + failure policy | Timer wake every 15 min + Refresh button (GPIO3) wake. On fetch/parse failure keep the last image (no flush), retry after 2 min; after 3 consecutive failures render one error screen, then return to the normal interval. | Avoids a 20 s flash on every transient WiFi failure. |
| Skip-unchanged | Device stores a 64-bit FNV-1a hash of the last successfully rendered body in RTC memory; identical body ⇒ no render, no flush. | The panel refresh is a multi-second full flash; most 15-minute polls will see unchanged content. |
| Frame memory | `render::Frame` = 800×480×4 bpp = 192,000 bytes. On device allocated once in **octal PSRAM** (8 MB on S3R8) via `esp-alloc`'s external-memory capability; static in wasm; boxed in host tests. | Internal SRAM must also hold esp-radio (~100 KB heap) + esp-rtos stacks + net buffers. Blocking SPI writes from PSRAM are fine (no DMA in v1). |
| `panel-backend` build target | Written purely against `embedded-hal` 1.0 traits + `epdsi` ⇒ **host-buildable and unit-testable**; move it into `default-members`. Only `firmware` stays Xtensa-only. | Lets CI/tests cover the SPI command stream with `embedded-hal-mock`. Update CLAUDE.md and the crate doc comment. |

## Pinned dependency set

Host crates (edition 2021 kept for workspace consistency; CI uses stable):

| Crate | Version | Notes |
|---|---|---|
| serde | 1 (`default-features = false`, `derive`) | shared |
| heapless | 0.9 (`serde`) | shared strings/vecs |
| serde-json-core | 0.6 | device + tests |
| serde_json | 1 | server only |
| embedded-graphics | 0.8.1 | render, panel-backend, web-preview |
| embedded-text | 0.7 | render |
| u8g2-fonts | 0.8 (`embedded_graphics_textstyle`) | render |
| epdsi | 0.4 (defaults: `blocking`, `graphics`) | panel-backend |
| embedded-hal | 1.0 | panel-backend |
| embedded-hal-mock | 0.11 (dev) | panel-backend tests |
| axum | 0.8, tokio 1 (`rt-multi-thread`, `macros`, `net`, `fs`), tower-http 0.6 (`trace`), rust-embed 8, clap 4 (`derive`), tracing + tracing-subscriber | server |

Firmware (coherent set taken from esp-generate's current template for esp-hal 1.1; esp-radio has no release for esp-hal 1.2 yet):

| Crate | Version / features |
|---|---|
| esp-hal | `~1.1.0` — `esp32s3`, `unstable` (there is **no** `psram` feature; `psram`, `rtc_cntl`, `timer`, `rng`, `RtcPin` all sit behind `unstable`) |
| esp-rtos | 0.3.0 — `esp32s3`, `esp-radio`, `esp-alloc`, `embassy` |
| esp-radio | 0.18.0 — `esp32s3`, `wifi`, `esp-alloc`, `unstable` (needed for `is_connected`/`with_country_info`; requires esp-hal `unstable` in the binary, which we have) |
| esp-alloc | 0.10 — `esp32s3`; plus `allocator-api2 = { version = "0.3", default-features = false, features = ["alloc"] }` for `Vec::with_capacity_in(.., esp_alloc::ExternalMemory)` on the stable-ish esp toolchain (esp-alloc's `ExternalMemory` implements `allocator_api2::alloc::Allocator`, not `core::alloc::Allocator`) |
| esp-bootloader-esp-idf | 0.5 — `esp32s3` |
| esp-println | 0.17 — `esp32s3`, `log-04` ; esp-backtrace 0.19 — `esp32s3`, `panic-handler`, `println` |
| embassy-executor 0.10, embassy-time 0.5, embassy-net 0.9.1 (`tcp`, `dhcpv4`, `dns`, `medium-ethernet`), smoltcp 0.13 (`default-features=false`, ipv4/tcp/dns/dhcp sockets), embassy-futures 0.1 |
| reqwless 0.14 (`default-features = false` → no TLS) |
| embedded-hal-bus 0.3 (`ExclusiveDevice` gives `SpiDevice` over esp-hal's `SpiBus` + CS pin) |
| static_cell 2, critical-section 1, log 0.4 |
| `[profile.dev.package.esp-radio] opt-level = 3` (required by esp-radio); `[profile.release] opt-level = "s", lto = "fat", codegen-units = 1, debug = 2` |

Firmware build config lives in `crates/firmware/.cargo/config.toml` (not the
root, so host builds are untouched): `[build] target = "xtensa-esp32s3-none-elf"`,
`[target.xtensa-esp32s3-none-elf] rustflags = ["-C", "link-arg=-nostartfiles"]`,
`[unstable] build-std = ["alloc", "core"]`; `crates/firmware/build.rs` emits
`cargo:rustc-link-arg=-Tlinkall.x`; `crates/firmware/rust-toolchain.toml`
selects channel `esp`. The firmware `Cargo.toml` uses edition 2024 /
`rust-version = "1.95"` as the template does.

## Hardware pin map (from Zephyr `reterminal_e1002_procpu.dts`, source of truth)

| Function | GPIO | Notes |
|---|---|---|
| EPD SPI SCLK / MOSI / MISO | 7 / 9 / 8 | SPI2; 4 MHz max per devicetree |
| EPD CS / DC / RST / BUSY | 10 / 11 / 12 / 13 | RST active-low; BUSY active-low with pull-up |
| Buttons Refresh / Left / Right | 3 / 4 / 5 | pull-up, active-low; GPIO3 is an RTC IO ⇒ deep-sleep wake source |
| Green LED | 6 | active-low; used as "awake" indicator |
| Battery ADC / enable | 1 / 21 | not used in v1 |
| SD card CS / PWR / CD | 14 / 16 / 15 | shares SPI2; keep CS high |
| I2C0 SDA/SCL (SHT4x, PCF8563 RTC) | 19 / 20 | not used in v1 |
| UART0 TX/RX | 43 / 44 | console (esp-println) |
| Memory | 32 MB flash, 8 MB octal PSRAM | |

## Finalised v1 screen-spec schema

```json
{
  "version": 1,
  "background": "white",
  "regions": [
    { "rect": [0, 0, 800, 80],   "text": "Kitchen Display", "style": "title",  "align": "center", "valign": "middle", "color": "black", "background": "yellow" },
    { "rect": [20, 100, 760, 360], "text": "Bin day: Thursday\nNext appointment: 14:00", "style": "body", "align": "left" },
    { "rect": [0, 470, 800, 10], "background": "blue" }
  ]
}
```

- `version: u8` (required, must be 1). `background`: colour, default `white`.
- `regions`: `heapless::Vec<Region, 16>`. `Region { rect: [i32; 4] (x, y, w, h), text: heapless::String<512> (default ""), style: Title|Header|Body|Small (default Body), align: Left|Center|Right (default Left), valign: Top|Middle|Bottom (default Top), color: Colour (default Black), background: Option<Colour>, border: Option<{ color, width: u8 }> }`.
- Colour enum = the six Spectra 6 names, lower-case in JSON.
- Regions draw in order (background fill → border → text), later regions over earlier ones; text is clipped to its rect (`embedded-text` handles overflow).
- Unknown fields ignored; a rect partially off-screen is clipped, not rejected.

## Architecture recap (unchanged from brief §7; data flow)

```
PUT /screen (curl)  ─►  server (axum, state file)  ─►  GET /screen  ─►  firmware (reqwless)
                                   │                                        │
                                   ▼                                        ▼
                    browser: preview.js → web_preview.wasm          screen-spec parse
                             (render::Frame → RGBA → canvas)        render::render(&spec, &mut Frame)
                                                                    panel-backend → epdsi ED2208 → panel
```

`render` never depends on hardware, wasm or the driver; `panel-backend` and
`web-preview` are thin adapters over `render::Frame`'s packed bytes.

## Phase 0 — Workspace scaffolding

Files: root `Cargo.toml`, `.gitignore`, `scripts/build-web-preview.sh`, `.github/workflows/ci.yml`, CLAUDE.md.

- Add `[workspace.dependencies]` for every shared crate above; crates inherit with `workspace = true`.
- Add `panel-backend` to `default-members`; update the root comment, CLAUDE.md ("Builds here?" table, commands), and the crate's doc comment.
- Ignore `crates/server/assets/web_preview.wasm`.
- CI `host-checks`: keep fmt/clippy/test; add `cargo build -p web-preview --target wasm32-unknown-unknown --release` (with `targets: wasm32-unknown-unknown` on the toolchain step) and `cargo test -p panel-backend`. CI `firmware`: `working-directory: crates/firmware`, `cargo build --release` (a real link, not just `check`).
- Commit `Cargo.lock` after dependency resolution.

## Phase 1 — `screen-spec`

Files: `crates/screen-spec/src/lib.rs`, `crates/screen-spec/tests/parse.rs`, `crates/screen-spec/examples/*.json` (sample specs: `kitchen.json`, `error-demo.json`).

- Types per the schema above, `#![no_std]`, `serde` derives, `Default`s, `Colour::ALL`.
- `pub const MAX_REGIONS = 16`, `MAX_TEXT = 512`, `MAX_JSON_BYTES = 16 * 1024` (the device's receive buffer; the server rejects larger `PUT`s with 413).
- `pub fn parse(bytes: &[u8], unescape_buf: &mut [u8]) -> Result<ScreenSpec, ParseError>` wrapping `serde_json_core::from_slice_escaped`, mapping wrong `version` to `ParseError::UnsupportedVersion(n)`.
- Tests (host, std): round-trip via `serde_json`; parse the sample files via `parse`; escaped `\n` and `\"` land unescaped; unknown field ignored; >16 regions and >512-byte text return `ParseError`, not a panic; `version: 2` → `UnsupportedVersion`.

## Phase 2 — `render`

Files: `crates/render/src/{lib.rs, color.rs, palette.rs, frame.rs, fonts.rs, compose.rs}`, `crates/render/tests/compose.rs`.

- `color.rs`: `Spectra6` enum (`PixelColor` with `Raw = RawU4`), `nibble()`, `from_nibble()`, `From<screen_spec::Colour>`.
- `palette.rs`: `pub const RGB: [[u8; 3]; 7]` indexed by nibble (index 4 = a deliberate magenta so a stray orange is obvious in the preview), plus `rgb(Spectra6)`.
- `frame.rs`: `WIDTH/HEIGHT/STRIDE/FRAME_BYTES = 192_000` consts; `Frame { data: [u8; FRAME_BYTES] }` (owned; `const fn new()` filled white `0x11`, used as a `static` in wasm and boxed in tests) and `FrameMut<'a>(&'a mut [u8])` (borrowed view over any 192,000-byte slice — the firmware's PSRAM `Vec`). Both share one `DrawTarget` impl via a private `PackedPixels` trait: `set_pixel`, `get_pixel`, `as_bytes()`, `fill(Spectra6)`, fast row/nibble-aware `fill_solid`, `clear`; `OriginDimensions`.
- `fonts.rs`: the four `u8g2` font presets and `text_style(style, color)`.
- `compose.rs`: `pub fn render(spec: &ScreenSpec, target: &mut impl DrawTarget<Color = Spectra6>) -> Result<(), E>`; `pub fn render_error(kind: ErrorKind, detail: &str, target)` used by both firmware and preview for "unsupported version" / "fetch failed" screens; text via `embedded_text::TextBox` with `TextBoxStyleBuilder` (alignment, vertical alignment, `HeightMode::FitToParent`), clipped to the rect via `target.clipped(&rect)`.
- Tests: `Frame` nibble packing (pixel 0 → high nibble; matches epdsi `SevenColor::pack`); `fill_solid` vs per-pixel equivalence on odd offsets; rendering the sample specs is deterministic (hash-pinned) and stays inside the six allowed nibbles (never 0x4/0x7); clipping of off-screen rects; a `Frame` → PPM dump helper behind `cfg(test)` for eyeballing (`cargo test -p render -- --ignored dump`).

## Phase 3 — `web-preview`

Files: `crates/web-preview/src/lib.rs`, `crates/server/assets/{index.html, preview.js}`, `scripts/build-web-preview.sh`.

- `#![cfg_attr(target_arch = "wasm32", no_std)]`, `crate-type = ["cdylib", "rlib"]` (already set). Statics: `SPEC_BUF: [u8; 16 KiB]`, `UNESCAPE_BUF`, `FRAME: render::Frame`, `RGBA: [u8; 800*480*4]`, `ERR_BUF: [u8; 256]`.
- Exports: `spec_buf_ptr() -> *mut u8`, `spec_buf_len() -> usize`, `render_spec(len: usize) -> i32` (0 = ok; negative codes for parse / version / overflow; on error it renders the same error screen the device would show and still returns the RGBA), `rgba_ptr() -> *const u8`, `width() / height()`, `error_ptr()/error_len()`. Host `rlib` build keeps `cargo check`/clippy green; a `#[panic_handler]` only under `target_arch = "wasm32"`.
- `preview.js`: `WebAssembly.instantiateStreaming(fetch('web_preview.wasm'))`; `refresh()` = `fetch('/screen')` → bytes → copy into `spec_buf` → `render_spec` → `new ImageData(new Uint8ClampedArray(memory.buffer, rgba_ptr, w*h*4), w, h)` → `putImageData`. Poll every 2 s (or `EventSource` later). Nice-to-have flag: 3-frame black/white flash before settling, mimicking the panel.
- `index.html`: 800×480 canvas scaled to fit, status line (last update, error text), a textarea + "PUT" button that posts the JSON so the whole loop works from the browser.
- `scripts/build-web-preview.sh`: builds release wasm, copies `target/wasm32-unknown-unknown/release/web_preview.wasm` to `crates/server/assets/`.
- Verification here: the wasm builds; a small Node/Playwright smoke test is optional — at minimum start `server` and load `/` in the pre-installed Chromium via Playwright to confirm the canvas renders (screenshot to scratchpad).

## Phase 4 — `server`

Files: `crates/server/src/{main.rs, state.rs, routes.rs}`, `crates/server/assets/`, `crates/server/tests/http.rs`, `README.md`.

- `clap` args: `--bind` (default `127.0.0.1:8080`), `--state-file` (default `screen.json`), `--assets-dir` override (dev).
- State: `Arc<RwLock<Current { json: String, etag: String, updated: SystemTime }>>`; `etag` = hex of the same FNV-1a the device uses (`screen_spec::content_hash`).
- Routes: `GET /screen` → `application/json`, `ETag`, `Cache-Control: no-store`, honours `If-None-Match` → 304; `PUT /screen` (also `POST`) → reject >16 KiB (413), parse with `screen_spec::parse` (400 with the error text), persist atomically (write temp + rename), 204; `GET /` and `/preview.js`, `/web_preview.wasm` from `rust-embed` (`Assets`), 404 with a hint if the wasm has not been built.
- Tests with `axum::Router` + `tower::ServiceExt::oneshot`: GET returns the sample; PUT invalid → 400; PUT valid → GET reflects it and the state file exists; oversized → 413; `If-None-Match` → 304.

## Phase 5 — `panel-backend`

Files: `crates/panel-backend/src/lib.rs`, `crates/panel-backend/tests/spi_stream.rs`.

- `pub struct Panel<SPI, DC, RST, BUSY>` wrapping `epdsi::EpdDriver<SpiBusWrapper<..>, Ed2208Controller, GDEP073E01>`; generic over `embedded_hal::spi::SpiDevice`, `OutputPin`, `InputPin`; `new(spi, dc, rst, busy)`, `init(&mut delay)`, `show(&mut self, frame: &render::Frame, &mut delay)` = `write_frame(ColorChannel::Color7(0), frame.as_bytes())` + `refresh`, `sleep(&mut delay)`, `clear(colour)` via `write_frame_pattern`/`clear_frame`.
- Test with `embedded-hal-mock` 0.11: `show` on a known frame emits `0x10` then exactly 192,000 data bytes equal to `frame.as_bytes()`, then `0x12 0x00`; `init` emits the ED2208 init sequence; BUSY is polled for low.

## Phase 6 — `firmware` (compile-checked in CI, validated by the user on hardware)

Files: `crates/firmware/{Cargo.toml, build.rs, rust-toolchain.toml, .cargo/config.toml, README.md, src/main.rs, src/{config.rs, net.rs, display.rs, power.rs, persist.rs}}`.

Boot sequence in one `#[esp_rtos::main] async fn main(spawner: Spawner)` (API
names below were verified against the extracted esp-hal 1.1.2 / esp-radio
0.18 / esp-rtos 0.3 / esp-alloc 0.10 sources by a source-level review):
1. `esp_hal::init(Config::default().with_cpu_clock(CpuClock::max()))`; `esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 72 KiB)` + `heap_allocator!(size: 96 KiB)` (internal heap must exist **before** `esp_rtos::start`); `esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram, PsramConfig { mode: PsramMode::OctalSpi, ..Default::default() })` (auto-detect is documented as unreliable; S3R8 is octal); `esp_println` logger.
2. `persist.rs`: `#[ram(rtc_fast, unstable(persistent))] static LAST_HASH: AtomicU64`, `FAILURES: AtomicU32` (only atomics/integers are `Persistable`; the section must be `rtc_fast` — plain `persistent` has no linker section). Zeroed only on power-on reset, so no magic needed. Log `rtc_cntl::reset_reason(Cpu::ProCpu)` / `wakeup_cause()`; LED (GPIO6, active-low) on while awake.
3. `esp_rtos::start(TimerGroup::new(peripherals.TIMG0).timer0, SoftwareInterruptControl::new(peripherals.SW_INTERRUPT).software_interrupt0)`; then `let (mut controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())?` — **`new` starts the radio; there is no `start()`/`stop()`**. `controller.set_config(&Config::Station(StationConfig::default().with_ssid(SSID).with_password(PASSWORD.into())))?` (WPA2-PSK default; SSID ≤ 32, password ≤ 64) then `controller.connect_async()` under `embassy_time::with_timeout(15 s)`.
4. `embassy_net::new(interfaces.station, Config::dhcpv4(Default::default()), StackResources<3>, esp_hal::rng::Rng::new().random() as u64)` (`Interface<'d>` implements `embassy_net_driver::Driver` and is `Copy`); spawn `runner`; `wait_link_up` + `wait_config_up` (10 s).
5. `net.rs`: `HttpClient::new(&TcpClient, &DnsSocket)` (embassy-net 0.9.1 implements `embedded-nal-async` 0.9, matching reqwless 0.14), `client.request(Method::GET, SCREEN_URL).await?.send(&mut rx_buf).await?` then `resp.body().read_to_end().await?` — headers and body share one `static RX_BUF: [u8; 16 KiB]`; the server must send `Content-Length` (axum does). 10 s timeout. FNV-1a hash of the body == `LAST_HASH` ⇒ skip to step 8 with `failures = 0`.
6. `screen_spec::parse` → `render::render` into the PSRAM `Frame`: allocate once with `allocator_api2::vec::Vec::<u8, _>::with_capacity_in(192_000, esp_alloc::ExternalMemory)` + `resize(192_000, 0x11)` and wrap as `render::FrameMut<'_>` (a borrowed-slice view `render` provides alongside the owned `Frame`, so no 192 KB array is ever materialised on the stack; a PSRAM `static` is impossible — esp-hal has no PSRAM linker section).
7. `display.rs`: `Spi::new(peripherals.SPI2, spi::master::Config::default().with_frequency(Rate::from_mhz(4)))?.with_sck(GPIO7).with_mosi(GPIO9).with_miso(GPIO8)`; `ExclusiveDevice::new(spi, Output::new(GPIO10, Level::High, OutputConfig::default()), Delay::new())?`; DC=GPIO11, RST=GPIO12 outputs; BUSY=`Input::new(GPIO13, InputConfig::default().with_pull(Pull::Up))`; SD-card CS GPIO14 driven high. `Panel::init` → `show` → `sleep` (blocking SPI writes from PSRAM are plain CPU FIFO copies, no DMA, ~0.4 s). On success `FAILURES = 0`, `LAST_HASH = hash`.
8. Failure policy per the decisions table (`FAILURES += 1`; error screen only when it reaches 3).
9. `power.rs`: `controller.disconnect_async().await` (ignore `NotConnected`), `drop(controller)` (runs `wifi_deinit` and releases the radio clocks); `RtcPinWithResistors::rtcio_pullup(&peripherals.GPIO3, true)` so the button line does not float in sleep; `let timer = TimerWakeupSource::new(core::time::Duration::from_secs(interval))` (note `core::time`, not `esp_hal::time`); `let ext0 = Ext0WakeupSource::new(peripherals.GPIO3, WakeupLevel::Low)` (takes the GPIO singleton, not an `Input`; keep it alive until sleep); **`RtcSleepConfig::deep()` powers RTC memory off**, so use `let mut cfg = RtcSleepConfig::deep(); cfg.set_rtc_fastmem_pd_en(false); Rtc::new(peripherals.LPWR).sleep(&cfg, &[&timer, &ext0]); unreachable!()` instead of `sleep_deep`. `interval` = 120 s while `0 < FAILURES < 3`, else `POLL_INTERVAL_SECS`.

Optional (esp-radio `unstable`): `ControllerConfig::default().with_country_info("GB")` — the default regdomain is `CN`; channels 1–13 work either way, so this is a nicety, not a blocker.

`README.md` for the crate: espup install, `source ~/export-esp.sh`, `WIFI_SSID=… WIFI_PASSWORD=… SCREEN_URL=… cargo run --release` with `espflash` runner (`runner = "espflash flash --monitor --chip esp32s3"` in the crate's `.cargo/config.toml`), and the hardware validation checklist below.

## Phase 7 — Docs and brief amendments

- Update `reTerminal-E1002-firmware-design-brief.md`: §5 → epdsi chosen (reasons above); §6 → fonts/embedded-text, drop embedded-layout/canvas from v1; §7 → wasm without wasm-bindgen; §8 → link to the finalised schema (keep the doc in `crates/screen-spec/README.md` or `docs/screen-spec.md`); §14 → each checkbox ticked with the decision.
- Root `README.md`: what it is, quick start (`cargo run -p server`, `scripts/build-web-preview.sh`, curl `PUT` example, open `http://127.0.0.1:8080/`), firmware flashing pointer.
- CLAUDE.md: panel-backend now host-built; new commands (`scripts/build-web-preview.sh`, wasm build); firmware env vars.

## Hardware validation checklist (user, on the device; maps to brief milestones 1–3, 6–7)

1. Back up stock firmware (`esptool -c esp32s3 read-flash 0x0 0x2000000 fw-backup-32MB.bin`).
2. Flash a build with `SCREEN_URL` pointing at the laptop running `server`; watch the serial monitor for: PSRAM detected (8 MB octal), WiFi connected + DHCP address, HTTP 200 with body length, "frame hash unchanged"/"rendering", panel init/refresh timings (expect ~20 s), deep-sleep entry.
3. Confirm the panel shows the same image as the browser preview for `examples/kitchen.json` (colours, wrap points, alignment).
4. Press Refresh (GPIO3) during sleep → wake reason `Ext0`, fetch runs. Timer wake after the interval.
5. Unplug the server → after the 3rd failure the error screen appears once; plug it back → normal content returns.
6. Measure sleep current and awake duration for the power budget (brief milestone 7); tune `POLL_INTERVAL_SECS`.

## Verification (this environment)

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (screen-spec, render, panel-backend, server, web-preview host build).
- `cargo build -p web-preview --target wasm32-unknown-unknown --release` then `scripts/build-web-preview.sh`.
- Run `cargo run -p server -- --bind 127.0.0.1:8080`, `curl -X PUT --data-binary @crates/screen-spec/examples/kitchen.json localhost:8080/screen`, `curl localhost:8080/screen`, then load `/` in the pre-installed Chromium via Playwright and screenshot the canvas to the scratchpad to eyeball the render.
- Firmware: push and let CI's `firmware` job link `crates/firmware` with the Xtensa toolchain; iterate on CI failures there (expected: the first pushes of Phase 6 may need API-name fixes since only CI can compile it).

## Risks / watch items

- serde-json-core: verified it implements `deserialize_ignored_any` (unknown fields skipped) and unescapes into owned strings via `visit_str`; Phase 1 tests pin both.
- Firmware API names were verified against extracted sources (see Phase 6), but only CI can compile the crate — budget one or two CI round-trips for the pieces not extracted here: `#[esp_rtos::main]`'s exact signature, `embedded_hal_bus::spi::ExclusiveDevice::new` returning `Result`, and embassy-net's `DnsSocket` short-circuiting IP-literal hosts (reqwless always calls `get_host_by_name`; if it does not short-circuit, resolve the literal ourselves with `SCREEN_URL` parsed into `IpAddress` + `TcpConnect` directly).
- GPIO3 wake: handled by `rtcio_pullup(true)` before sleep (Phase 6 step 9); confirm on hardware that no spurious wakes occur. Failure counter/hash in RTC fast memory relies on `set_rtc_fastmem_pd_en(false)`; it costs a few µA in sleep — measure in milestone 7.
- `with_timeout` around `connect_async` only drops the future; the blob may still connect later. We drop the controller afterwards anyway, so it is harmless, but `disconnect_async` may return `NotConnected` (ignored).
- 192 KB frame in PSRAM: esp-hal's PSRAM docs say it must be built in release mode; the firmware README and CI use `--release`. Never place atomics in PSRAM.
- `Frame` must never be constructed on the stack on-device: only `FrameMut` over the PSRAM `Vec` is used there (a `#![deny(clippy::large_stack_frames)]` lint in `firmware`, as esp-generate's template does, catches regressions).
