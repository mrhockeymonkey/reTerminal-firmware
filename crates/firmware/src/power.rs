//! Deep sleep and wake sources (design brief §12).

use esp_hal::gpio::RtcPinWithResistors;
use esp_hal::peripherals::{GPIO3, LPWR};
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
