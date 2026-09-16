//! The e-paper panel on the E1002's SPI2 bus (pins from Zephyr's board
//! devicetree; see docs/IMPLEMENTATION_PLAN.md).

use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::peripherals::{GPIO7, GPIO8, GPIO9, GPIO10, GPIO11, GPIO12, GPIO13, SPI2};
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use log::{error, info};
use panel_backend::Panel;

/// Everything the panel needs, moved out of `Peripherals`.
pub struct DisplayPins {
    pub spi2: SPI2<'static>,
    pub sck: GPIO7<'static>,
    pub miso: GPIO8<'static>,
    pub mosi: GPIO9<'static>,
    pub cs: GPIO10<'static>,
    pub dc: GPIO11<'static>,
    pub rst: GPIO12<'static>,
    pub busy: GPIO13<'static>,
}

type SpiDev = ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>;
type PanelT = Panel<SpiDev, Output<'static>, Output<'static>, Input<'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayError {
    /// SPI peripheral configuration was rejected.
    Spi,
    /// The panel driver reported an error (already logged).
    Panel,
}

/// The panel, initialised lazily: creating this sends nothing.
pub struct Display {
    panel: PanelT,
    delay: Delay,
    initialised: bool,
}

impl Display {
    pub fn new(pins: DisplayPins) -> Result<Self, DisplayError> {
        // 4 MHz is the devicetree's `mipi-max-frequency`; mode 0.
        let spi = Spi::new(
            pins.spi2,
            SpiConfig::default().with_frequency(Rate::from_mhz(4)),
        )
        .map_err(|e| {
            error!("SPI config: {e:?}");
            DisplayError::Spi
        })?
        .with_sck(pins.sck)
        .with_mosi(pins.mosi)
        .with_miso(pins.miso);
        let cs = Output::new(pins.cs, Level::High, OutputConfig::default());
        let dev = ExclusiveDevice::new(spi, cs, Delay::new()).map_err(|_| DisplayError::Spi)?;

        let dc = Output::new(pins.dc, Level::Low, OutputConfig::default());
        let rst = Output::new(pins.rst, Level::High, OutputConfig::default());
        let busy = Input::new(pins.busy, InputConfig::default().with_pull(Pull::Up));
        Ok(Display {
            panel: Panel::new(dev, dc, rst, busy),
            delay: Delay::new(),
            initialised: false,
        })
    }

    fn ensure_init(&mut self) -> Result<(), DisplayError> {
        if !self.initialised {
            info!("panel: init");
            self.panel.init(&mut self.delay).map_err(|e| {
                error!("panel init: {e:?}");
                DisplayError::Panel
            })?;
            self.initialised = true;
        }
        Ok(())
    }

    /// Sends a packed frame and performs the full refresh (~20 s), then
    /// puts the panel to sleep. The image persists with no power.
    pub fn show(&mut self, frame: &[u8]) -> Result<(), DisplayError> {
        self.ensure_init()?;
        info!("panel: refresh ({} bytes)", frame.len());
        let result = self.panel.show(frame, &mut self.delay).map_err(|e| {
            error!("panel show: {e:?}");
            DisplayError::Panel
        });
        self.sleep();
        result
    }

    /// Powers the panel down. Safe to call whether or not it was used.
    pub fn sleep(&mut self) {
        if self.initialised {
            if let Err(e) = self.panel.sleep(&mut self.delay) {
                error!("panel sleep: {e:?}");
            }
            self.initialised = false;
        }
    }
}
