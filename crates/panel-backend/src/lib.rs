//! Wires `render`'s packed frame to the real GDEP073E01 panel (E Ink
//! Spectra 6, ED2208-GCA controller) over SPI, via the `epdsi` driver
//! framework (design brief §5, §7).
//!
//! Written purely against `embedded-hal` 1.0 traits, so it builds and is
//! unit-tested on the host with fake SPI/GPIO; only `firmware` binds it to
//! esp-hal peripherals.
//!
//! The panel has a single, multi-second, full-flash refresh. The API
//! therefore follows the brief's "compose once, flush once" model: build a
//! complete [`render::Frame`] first, then [`Panel::show`] it once.
//!
//! ```ignore
//! let mut panel = Panel::new(spi_device, dc, rst, busy);
//! panel.init(&mut delay)?;             // reset + ED2208 init + power on
//! panel.show(frame.as_bytes(), &mut delay)?; // ~20 s full refresh
//! panel.sleep(&mut delay)?;            // power off + deep sleep; image persists
//! ```
#![no_std]
#![warn(missing_docs)]

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::spi::SpiDevice;
use epdsi::prelude::*;
use render::{Spectra6, FRAME_BYTES, HEIGHT, WIDTH};

/// ED2208 "data start transmission" command: the frame bytes follow.
const ED2208_DATA_START_TRANSMISSION: u8 = 0x10;

/// Errors from the bus: SPI, the DC/RST outputs, the BUSY input, or a
/// validation failure such as a short frame.
pub type Error<SPI, DC, RST, BUSY> = EpdBusError<
    <SPI as embedded_hal::spi::ErrorType>::Error,
    <DC as embedded_hal::digital::ErrorType>::Error,
    <RST as embedded_hal::digital::ErrorType>::Error,
    <BUSY as embedded_hal::digital::ErrorType>::Error,
>;

/// The GDEP073E01 panel behind an SPI device and three control pins.
///
/// * `spi`: an `SpiDevice` (owns chip-select; ≤ 4 MHz per the board's
///   devicetree), SPI mode 0.
/// * `dc`: data/command select output (low = command).
/// * `rst`: reset output (active low).
/// * `busy`: busy input; this panel drives it **low while busy**.
pub struct Panel<SPI, DC, RST, BUSY> {
    epd: EpdDriver<SpiBusWrapper<SPI, DC, RST, BUSY>, Ed2208Controller, GDEP073E01>,
}

impl<SPI, DC, RST, BUSY> Panel<SPI, DC, RST, BUSY>
where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
    BUSY: InputPin,
{
    /// Wraps the bus. Nothing is sent until [`Panel::init`].
    pub fn new(spi: SPI, dc: DC, rst: RST, busy: BUSY) -> Self {
        let bus = SpiBusWrapper::new(spi, dc, rst, busy);
        let controller = Ed2208Controller::new(WIDTH, HEIGHT);
        Panel {
            epd: EpdBuilder::<_, GDEP073E01>::new(controller).build(bus),
        }
    }

    /// Hardware reset, the ED2208 register init sequence, and power-on
    /// (waits for BUSY). Required after power-up and after [`Panel::sleep`].
    pub fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<SPI, DC, RST, BUSY>> {
        self.epd.init(delay)
    }

    /// Sends a packed 4bpp frame (exactly [`FRAME_BYTES`] bytes, as produced
    /// by [`render::Frame::as_bytes`]) and triggers the full refresh,
    /// returning once the panel reports it is done (~20 s).
    ///
    /// A frame of the wrong length is rejected before anything is sent.
    pub fn show<D: DelayNs>(
        &mut self,
        frame: &[u8],
        delay: &mut D,
    ) -> Result<(), Error<SPI, DC, RST, BUSY>> {
        if frame.len() != FRAME_BYTES {
            // epdsi only checks for *short* buffers; a long one would be
            // silently truncated on the wire, so treat both as misuse.
            return Err(EpdBusError::BufferTooSmall {
                required: FRAME_BYTES,
                provided: frame.len(),
            });
        }
        self.epd.write_frame(ColorChannel::Color7(0), frame)?;
        self.epd.refresh(delay)
    }

    /// Fills the whole panel with one colour and refreshes. Handy for
    /// bring-up (brief milestone 1) and for clearing ghosting.
    pub fn clear<D: DelayNs>(
        &mut self,
        colour: Spectra6,
        delay: &mut D,
    ) -> Result<(), Error<SPI, DC, RST, BUSY>> {
        // Not `EpdDriver::clear_frame`: epdsi 0.4 sizes the `Color7`
        // channel as 1 bpp and would send only a quarter of the frame.
        let byte = Spectra6::pack(colour, colour);
        let bus = self.epd.bus_mut();
        bus.send_command(ED2208_DATA_START_TRANSMISSION)?;
        bus.send_data_repeated(byte, FRAME_BYTES)?;
        self.epd.refresh(delay)
    }

    /// Powers the panel off and puts the controller into deep sleep. The
    /// displayed image is retained with no power. Call [`Panel::init`]
    /// before drawing again.
    pub fn sleep<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<SPI, DC, RST, BUSY>> {
        self.epd.sleep(delay)
    }
}
