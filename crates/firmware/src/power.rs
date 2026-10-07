//! Deep sleep and wake sources (design brief §12), and the battery gauge.

use esp_hal::analog::adc::{Adc, AdcCalCurve, AdcConfig, Attenuation};
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig, RtcPinWithResistors};
use esp_hal::peripherals::{ADC1, GPIO1, GPIO3, GPIO21, LPWR};
use log::info;

use crate::config;

/// Resting LiPo voltage (mV) to charge (%), highest first. A coarse curve is
/// enough: the only decision made from it is "below 10%".
const LIPO_CURVE: [(u32, u8); 11] = [
    (4200, 100),
    (4100, 90),
    (4000, 80),
    (3920, 70),
    (3850, 60),
    (3800, 50),
    (3760, 40),
    (3720, 30),
    (3680, 20),
    (3600, 10),
    (3500, 0),
];

/// Measures the battery through its divider on GPIO1 (ADC1 channel 0).
/// GPIO21 switches the divider in, so it draws nothing while asleep.
///
/// Call before the radio starts: its current draw sags the voltage.
pub fn battery_percent(adc1: ADC1<'static>, sense: GPIO1<'static>, enable: GPIO21<'static>) -> u8 {
    const SAMPLES: u32 = 16;

    let mut enable = Output::new(enable, Level::High, OutputConfig::default());
    Delay::new().delay_millis(10);

    let mut adc_config = AdcConfig::new();
    let mut pin =
        adc_config.enable_pin_with_cal::<_, AdcCalCurve<ADC1<'static>>>(sense, Attenuation::_11dB);
    let mut adc = Adc::new(adc1, adc_config);
    // With curve calibration the reading is already in millivolts.
    let pin_mv = (0..SAMPLES)
        .map(|_| u32::from(adc.read_blocking(&mut pin)))
        .sum::<u32>()
        / SAMPLES;
    enable.set_low();

    let mv = pin_mv * config::BATTERY_DIVIDER;
    let percent = lipo_percent(mv);
    info!("battery: {mv} mV (pin {pin_mv} mV) ≈ {percent}%");
    percent
}

/// Linear interpolation along [`LIPO_CURVE`].
fn lipo_percent(mv: u32) -> u8 {
    let (top_mv, top_pc) = LIPO_CURVE[0];
    if mv >= top_mv {
        return top_pc;
    }
    for pair in LIPO_CURVE.windows(2) {
        let ((hi_mv, hi_pc), (lo_mv, lo_pc)) = (pair[0], pair[1]);
        if mv >= lo_mv {
            let span = u32::from(hi_pc - lo_pc);
            return lo_pc + ((mv - lo_mv) * span / (hi_mv - lo_mv)) as u8;
        }
    }
    0
}
use esp_hal::rtc_cntl::Rtc;
use esp_hal::rtc_cntl::sleep::{Ext0WakeupSource, RtcSleepConfig, TimerWakeupSource, WakeupLevel};

/// Enters deep sleep. Wakes after `secs`, or when the Refresh button
/// (GPIO3, active low) is pressed. Never returns; the chip reboots on wake.
///
/// RTC fast memory is kept powered so `persist` survives (a few µA).
pub fn deep_sleep(lpwr: LPWR<'static>, refresh_button: GPIO3<'static>, secs: u64) -> ! {
    // The button line has only the internal pull-up in the devicetree; the
    // Ext0 wake source does not enable it, so do it here or the pin floats.
    refresh_button.rtcio_pullup(true);

    let timer = TimerWakeupSource::new(core::time::Duration::from_secs(secs));
    let button = Ext0WakeupSource::new(refresh_button, WakeupLevel::Low);

    let mut cfg = RtcSleepConfig::deep();
    cfg.set_rtc_fastmem_pd_en(false);

    let mut rtc = Rtc::new(lpwr);
    rtc.sleep(&cfg, &[&timer, &button]);
    unreachable!("deep sleep does not return")
}
