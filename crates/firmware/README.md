# `firmware`

ESP32-S3 firmware for the Seeed reTerminal E1002. On every wake it joins
WiFi, fetches `GET /screen` from the LAN `server`, renders the document with
the shared `render` crate, flushes the 7.3" Spectra 6 panel, and deep-sleeps
until the next timer tick or a press of the **Refresh** button (GPIO3).

## Building

This crate targets `xtensa-esp32s3-none-elf`, which needs the esp-rs fork of
rustc/LLVM (mainline Rust has the target spec but not Xtensa codegen):

```sh
cargo install espup espflash
espup install                 # installs the `esp` toolchain
source ~/export-esp.sh        # every shell that builds this crate
cd crates/firmware            # picks up rust-toolchain.toml + .cargo/config.toml
WIFI_SSID=home WIFI_PASSWORD=secret SCREEN_URL=http://192.168.1.20:8080/screen \
  cargo run --release         # builds, flashes over USB, opens the serial monitor
```

Always build `--release`: esp-hal's PSRAM support requires an optimised build.

Configuration is baked in at build time (see `src/config.rs`):

| Variable | Default | Meaning |
|---|---|---|
| `WIFI_SSID`, `WIFI_PASSWORD` | placeholders | WPA2-PSK network to join |
| `SCREEN_URL` | `http://192.168.1.2:8080/screen` | the `server`'s `/screen` route (plain HTTP) |
| `POLL_INTERVAL_SECS` | `900` | timer wake interval when healthy (≥ 30) |
| `ESP_LOG` | `info` | esp-println log level filter |

**This crate cannot be built in a Claude Code Remote session** (the toolchain
download hosts are blocked there); it is excluded from the workspace's
`default-members` and compiled by CI's `firmware` job instead.

## What a wake cycle does

1. `esp_hal::init`, heaps (internal + 8 MB octal PSRAM), `esp_rtos::start`.
2. WiFi station connect (15 s timeout) → DHCP (10 s) → `GET /screen` (10 s)
   into a 16 KiB buffer → radio off.
3. FNV-1a hash of the body compared with the last flushed document (kept in
   RTC memory): unchanged ⇒ skip straight to sleep, no panel flash.
4. Parse (`screen-spec`) → `render` into the PSRAM frame → `panel-backend`
   streams 192,000 bytes and refreshes (~20 s) → panel to sleep.
5. Failure policy: keep the last image, retry in 2 min; after 3 consecutive
   failures show the shared error screen once, then back to the normal
   interval.
6. Deep sleep with timer + Refresh-button wake; RTC fast memory stays on so
   the hash/failure counter survive.

## Pins (from Zephyr's `reterminal_e1002_procpu.dts`)

| Function | GPIO |
|---|---|
| EPD SPI SCLK / MOSI / MISO | 7 / 9 / 8 |
| EPD CS / DC / RST / BUSY | 10 / 11 / 12 / 13 (BUSY active-low) |
| Refresh / Left / Right buttons | 3 / 4 / 5 (active-low) |
| Green LED | 6 (active-low, on while awake) |
| SD card CS | 14 (held high) |

## Hardware validation checklist

1. Back up the stock firmware first:
   `esptool -c esp32s3 -p /dev/ttyUSB0 read-flash 0x0 0x2000000 fw-backup-32MB.bin`.
2. Run `server` on the laptop (`cargo run -p server -- --bind 0.0.0.0:8080`),
   flash with `SCREEN_URL` pointing at it, and watch the monitor for: PSRAM
   init, `wifi: connected`, `dhcp: <ip>`, `http: N byte body`, `panel: refresh`,
   `deep sleep for 900 s`.
3. Compare the panel with the browser preview at `http://<laptop>:8080/`.
4. Press Refresh during sleep: the log should show wake cause `Ext0`.
5. Stop the server: after the third failed cycle the red error screen
   appears; start it again and normal content returns.
6. Measure sleep current and awake time to tune `POLL_INTERVAL_SECS`.
