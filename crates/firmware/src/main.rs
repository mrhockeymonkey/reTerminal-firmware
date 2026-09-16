//! ESP32-S3 firmware for the Seeed reTerminal E1002 (design brief §7, §10,
//! §12): on every wake, join WiFi, `GET /screen` from the LAN server, render
//! the document with the shared `render` crate, flush the e-paper panel, and
//! go back to deep sleep until the next timer tick or a press of the
//! Refresh button.
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
use render::{ErrorKind, FRAME_BYTES, FrameMut};
use screen_spec::{MAX_JSON_BYTES, MAX_TEXT, ParseError};
use static_cell::ConstStaticCell;

use crate::display::{Display, DisplayPins};
use crate::net::NetError;

// App descriptor required by the ESP-IDF 2nd-stage bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

/// Receive buffer: headers + body of `GET /screen`. The body alone may be
/// up to `MAX_JSON_BYTES`; the extra covers the response headers.
static BODY: ConstStaticCell<[u8; MAX_JSON_BYTES + 2048]> =
    ConstStaticCell::new([0; MAX_JSON_BYTES + 2048]);
/// Scratch for decoding JSON string escapes.
static UNESCAPE: ConstStaticCell<[u8; MAX_TEXT]> = ConstStaticCell::new([0; MAX_TEXT]);

/// One wake cycle's outcome.
enum Outcome {
    /// Content unchanged since the last flush; nothing drawn.
    Unchanged,
    /// A new document was rendered and flushed.
    Shown,
    /// Something failed; the previous image was left on the panel.
    Failed(Failure),
}

#[derive(Debug, Clone, Copy)]
enum Failure {
    Net(NetError),
    Spec(ParseError),
    Display(display::DisplayError),
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

    let outcome = cycle(&spawner, peripherals.WIFI, display.as_mut(), &mut frame_buf).await;

    let failures = match outcome {
        Outcome::Unchanged => {
            info!("cycle: unchanged");
            0
        }
        Outcome::Shown => {
            info!("cycle: shown");
            0
        }
        Outcome::Failed(f) => {
            let n = persist::failures() + 1;
            warn!("cycle: failed ({f:?}); consecutive failures {n}");
            if n == config::FAILURES_BEFORE_ERROR_SCREEN {
                show_error_screen(display.as_mut(), &mut frame_buf, f);
            }
            n
        }
    };
    persist::set_failures(failures);

    if let Some(d) = display.as_mut() {
        d.sleep();
    }
    led.set_high();

    let secs = if failures > 0 && failures < config::FAILURES_BEFORE_ERROR_SCREEN {
        config::RETRY_INTERVAL_SECS
    } else {
        config::POLL_INTERVAL_SECS
    };
    info!("deep sleep for {secs} s (or Refresh button)");
    power::deep_sleep(peripherals.LPWR, peripherals.GPIO3, secs)
}

async fn cycle(
    spawner: &Spawner,
    wifi: esp_hal::peripherals::WIFI<'static>,
    display: Option<&mut Display>,
    frame_buf: &mut [u8],
) -> Outcome {
    let conn = match net::connect(spawner, wifi).await {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(Failure::Net(e)),
    };

    let body = BODY.take();
    let fetched = net::fetch_screen(conn.stack, body).await;
    // Radio off as early as possible: rendering and the 20 s panel refresh
    // do not need it.
    net::disconnect(conn).await;

    let len = match fetched {
        Ok(len) => len,
        Err(e) => return Outcome::Failed(Failure::Net(e)),
    };
    let json = &body[..len];

    let hash = screen_spec::content_hash(json);
    if hash == persist::last_hash() {
        return Outcome::Unchanged;
    }

    let spec = match screen_spec::parse(json, UNESCAPE.take()) {
        Ok(s) => s,
        Err(e) => {
            warn!("spec: {e}");
            return Outcome::Failed(Failure::Spec(e));
        }
    };

    let Some(display) = display else {
        return Outcome::Failed(Failure::Display(display::DisplayError::Spi));
    };
    let mut frame = FrameMut::new(frame_buf).expect("frame buffer is FRAME_BYTES long");
    let _ = render::render(&spec, &mut frame);
    match display.show(frame.as_bytes()) {
        Ok(()) => {
            persist::set_last_hash(hash);
            Outcome::Shown
        }
        Err(e) => Outcome::Failed(Failure::Display(e)),
    }
}

/// Renders the shared error notice so a persistently broken setup is
/// visible on the panel, not just on the serial console.
fn show_error_screen(display: Option<&mut Display>, frame_buf: &mut [u8], failure: Failure) {
    let Some(display) = display else {
        return;
    };
    let mut detail: heapless::String<160> = heapless::String::new();
    let kind = match failure {
        Failure::Net(e) => {
            let _ = write!(detail, "{e:?} — server {}", config::SCREEN_URL);
            ErrorKind::FetchFailed
        }
        Failure::Spec(ParseError::UnsupportedVersion(v)) => {
            let _ = write!(
                detail,
                "server sent version {v}; this firmware understands version {}",
                screen_spec::VERSION
            );
            ErrorKind::UnsupportedVersion
        }
        Failure::Spec(e) => {
            let _ = write!(detail, "{e}");
            ErrorKind::InvalidSpec
        }
        Failure::Display(e) => {
            let _ = write!(detail, "display: {e:?}");
            ErrorKind::FetchFailed
        }
    };
    let mut frame = FrameMut::new(frame_buf).expect("frame buffer is FRAME_BYTES long");
    let _ = render::render_error(kind, &detail, &mut frame);
    if let Err(e) = display.show(frame.as_bytes()) {
        error!("could not show error screen: {e:?}");
    } else {
        // The error screen replaced the last good image.
        persist::set_last_hash(0);
    }
}
