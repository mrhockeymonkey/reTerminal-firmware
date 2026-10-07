//! ESP32-S3 firmware for the Seeed reTerminal E1002 (design brief §7, §10,
//! §12): on every wake, join WiFi, `GET /screen` from the LAN server, render
//! the document with the shared `render` crate, flush the e-paper panel, and
//! go back to deep sleep until the next timer tick or a press of the
//! Refresh button.
//!
//! What only the device knows — battery low, or no usable screen from the
//! server — is shown by turning the document's status bar red (see
//! `render::render_with_alert`), repainting the last good document from
//! flash when the fetch failed. The panel is refreshed only when the
//! document or the bar state changes.
//!
//! Only compiles for `xtensa-esp32s3-none-elf` with the esp-rs toolchain;
//! see README.md in this directory. CI links it on every push.
#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "esp-hal types may own DMA/peripheral state that must be dropped"
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

mod config;
mod display;
mod net;
mod persist;
mod power;
mod store;

use core::fmt::Write;

use allocator_api2::vec::Vec;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::psram::{PsramConfig, PsramMode};
use esp_hal::system::Cpu;
use esp_hal::timer::timg::TimerGroup;
use log::{error, info, warn};
use render::{FRAME_BYTES, FrameMut};
use screen_spec::{MAX_JSON_BYTES, MAX_TEXT, ParseError, ScreenSpec};
use static_cell::ConstStaticCell;

use crate::display::{Display, DisplayPins};
use crate::net::NetError;
use crate::store::Store;

// App descriptor required by the ESP-IDF 2nd-stage bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

/// Receive buffer: headers + body of `GET /screen`. The body alone may be
/// up to `MAX_JSON_BYTES`; the extra covers the response headers.
static BODY: ConstStaticCell<[u8; MAX_JSON_BYTES + 2048]> =
    ConstStaticCell::new([0; MAX_JSON_BYTES + 2048]);
/// Scratch for decoding JSON string escapes.
static UNESCAPE: ConstStaticCell<[u8; MAX_TEXT]> = ConstStaticCell::new([0; MAX_TEXT]);

#[derive(Debug, Clone, Copy)]
enum Failure {
    Net(NetError),
    Spec(ParseError),
    Display(display::DisplayError),
}

/// What the status bar shows. Stored in RTC memory as [`Bar::code`] so a
/// change (and only a change) costs a panel refresh.
#[derive(Debug, Clone, Copy)]
enum Bar {
    /// As the server sent it.
    Normal,
    /// Red, "battery low".
    BatteryLow,
    /// Red, with the failure. Takes precedence over the battery warning.
    Error(Failure),
}

impl Bar {
    fn code(self) -> u32 {
        match self {
            Bar::Normal => 0,
            Bar::BatteryLow => 1,
            // The message is not part of the code: a WiFi error turning
            // into an HTTP error is not worth a 20 s refresh.
            Bar::Error(_) => 2,
        }
    }

    fn message(self) -> Option<heapless::String<64>> {
        let mut msg = heapless::String::new();
        let _ = match self {
            Bar::Normal => return None,
            Bar::BatteryLow => msg.write_str("battery low"),
            Bar::Error(Failure::Net(e)) => match e {
                NetError::Radio => msg.write_str("WiFi radio failed"),
                NetError::WifiConnect => msg.write_str("no WiFi connection"),
                NetError::Dhcp => msg.write_str("WiFi: no IP address"),
                NetError::Http => msg.write_str("could not reach server"),
                NetError::Status(code) => write!(msg, "server error {code}"),
                NetError::BodyTooLarge => msg.write_str("screen too large"),
            },
            Bar::Error(Failure::Spec(ParseError::UnsupportedVersion(v))) => write!(
                msg,
                "server sent spec v{v}, firmware supports v{}",
                screen_spec::VERSION
            ),
            Bar::Error(Failure::Spec(ParseError::Json(_))) => {
                msg.write_str("invalid screen from server")
            }
            Bar::Error(Failure::Display(e)) => write!(msg, "display: {e:?}"),
        };
        Some(msg)
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Internal heap first (esp-rtos and esp-radio allocate from it), then
    // the 8 MB octal PSRAM for the frame buffer.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72 * 1024);
    esp_alloc::heap_allocator!(size: 96 * 1024);
    esp_alloc::psram_allocator!(
        peripherals.PSRAM,
        esp_hal::psram,
        PsramConfig {
            mode: PsramMode::OctalSpi,
            ..Default::default()
        }
    );

    info!(
        "reTerminal E1002 firmware: reset {:?}, wake {:?}, failures so far {}",
        esp_hal::rtc_cntl::reset_reason(Cpu::ProCpu),
        esp_hal::rtc_cntl::wakeup_cause(),
        persist::failures()
    );

    // Green LED (active low) on while awake; SD card kept off the shared bus.
    let mut led = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());
    let _sd_cs = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());

    // Before the radio starts: its current draw sags the battery voltage.
    let percent = power::battery_percent(peripherals.ADC1, peripherals.GPIO1, peripherals.GPIO21);
    let battery_low = if persist::battery_low() {
        percent < config::BATTERY_OK_PERCENT
    } else {
        percent < config::BATTERY_LOW_PERCENT
    };
    persist::set_battery_low(battery_low);

    // The scheduler must run before the radio is initialised.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Frame buffer in PSRAM: never on the stack, never in internal SRAM.
    let mut frame_buf: Vec<u8, esp_alloc::ExternalMemory> =
        Vec::with_capacity_in(FRAME_BYTES, esp_alloc::ExternalMemory);
    frame_buf.resize(FRAME_BYTES, 0x11);

    let mut display = match Display::new(DisplayPins {
        spi2: peripherals.SPI2,
        sck: peripherals.GPIO7,
        miso: peripherals.GPIO8,
        mosi: peripherals.GPIO9,
        cs: peripherals.GPIO10,
        dc: peripherals.GPIO11,
        rst: peripherals.GPIO12,
        busy: peripherals.GPIO13,
    }) {
        Ok(d) => Some(d),
        Err(e) => {
            error!("display setup failed: {e:?}");
            None
        }
    };
    let mut store = Store::open(peripherals.FLASH);

    let body = BODY.take();
    let unescape = UNESCAPE.take();
    let fetched = fetch(&spawner, peripherals.WIFI, body).await;

    let result = match fetched {
        Ok(len) => {
            let json = &body[..len];
            let hash = screen_spec::content_hash(json);
            let bar = if battery_low {
                Bar::BatteryLow
            } else {
                Bar::Normal
            };
            if hash == persist::last_hash() && bar.code() == persist::last_bar() {
                info!("cycle: unchanged");
                Ok(())
            } else {
                match screen_spec::parse(json, unescape) {
                    Ok(spec) => {
                        if let Some(store) = store.as_mut() {
                            store.save(json, hash);
                        }
                        show(display.as_mut(), &mut frame_buf, &spec, hash, bar)
                            .map(|()| info!("cycle: shown"))
                            .map_err(Failure::Display)
                    }
                    Err(e) => {
                        warn!("spec: {e}");
                        Err(Failure::Spec(e))
                    }
                }
            }
        }
        Err(e) => Err(Failure::Net(e)),
    };

    let failures = match result {
        Ok(()) => 0,
        Err(f) => {
            let n = persist::failures() + 1;
            warn!("cycle: failed ({f:?}); consecutive failures {n}");
            // The panel still shows whatever it showed; only a change in the
            // bar (the error after enough failures, or the battery warning
            // appearing meanwhile) repaints the last good screen.
            let bar = if n >= config::FAILURES_BEFORE_ERROR_BAR {
                Bar::Error(f)
            } else if battery_low {
                Bar::BatteryLow
            } else {
                Bar::Normal
            };
            if bar.code() != persist::last_bar() {
                repaint_saved(
                    display.as_mut(),
                    store.as_mut(),
                    &mut frame_buf,
                    body,
                    unescape,
                    bar,
                );
            }
            n
        }
    };
    persist::set_failures(failures);

    if let Some(d) = display.as_mut() {
        d.sleep();
    }
    led.set_high();

    let secs = if failures > 0 && failures < config::FAILURES_BEFORE_ERROR_BAR {
        config::RETRY_INTERVAL_SECS
    } else {
        config::POLL_INTERVAL_SECS
    };
    info!("deep sleep for {secs} s (or Refresh button)");
    power::deep_sleep(peripherals.LPWR, peripherals.GPIO3, secs)
}

/// Joins WiFi, fetches the document into the start of `body` and returns its
/// length. The radio is off again by the time this returns: rendering and
/// the 20 s panel refresh do not need it.
async fn fetch(
    spawner: &Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    body: &mut [u8],
) -> Result<usize, NetError> {
    let conn = net::connect(spawner, wifi).await?;
    let fetched = net::fetch_screen(conn.stack, body).await;
    net::disconnect(conn).await;
    fetched
}

/// Renders `spec` with `bar` and flushes it, recording what the panel now
/// shows so an identical next wake can skip the refresh.
fn show(
    display: Option<&mut Display>,
    frame_buf: &mut [u8],
    spec: &ScreenSpec,
    hash: u64,
    bar: Bar,
) -> Result<(), display::DisplayError> {
    let display = display.ok_or(display::DisplayError::Spi)?;
    let mut frame = FrameMut::new(frame_buf).expect("frame buffer is FRAME_BYTES long");
    let message = bar.message();
    let _ = render::render_with_alert(spec, message.as_deref(), &mut frame);
    display.show(frame.as_bytes())?;
    persist::set_last_hash(hash);
    persist::set_last_bar(bar.code());
    Ok(())
}

/// Repaints the last good document from flash with `bar`; with nothing
/// saved (or no flash partition), a blank screen with just the bar.
fn repaint_saved(
    display: Option<&mut Display>,
    store: Option<&mut Store>,
    frame_buf: &mut [u8],
    body: &mut [u8],
    unescape: &mut [u8],
    bar: Bar,
) {
    // One `ScreenSpec` (~9 KB) on the stack, not two.
    let mut spec = ScreenSpec::empty();
    let mut hash = 0;
    if let Some((len, saved_hash)) = store.and_then(|s| s.load(body)) {
        match screen_spec::parse(&body[..len], unescape) {
            Ok(saved) => {
                spec = saved;
                hash = saved_hash;
            }
            Err(e) => warn!("store: saved screen does not parse: {e}"),
        }
    }
    info!("cycle: repainting with status bar {bar:?}");
    if let Err(e) = show(display, frame_buf, &spec, hash, bar) {
        error!("could not repaint: {e:?}");
    }
}
