//! State that survives deep sleep.
//!
//! These statics live in RTC fast memory, which stays powered through deep
//! sleep only because `power::deep_sleep` keeps it on (esp-hal's default
//! deep-sleep config powers it down). They are zeroed on a power-on reset
//! and preserved across timer/button wakes, panics and watchdog resets.
//!
//! Only atomics and plain integers may be placed here; the ESP32-S3 has no
//! 64-bit atomics, so the hash is split in two.

use core::sync::atomic::{AtomicU32, Ordering};

#[esp_hal::ram(rtc_fast, unstable(persistent))]
static LAST_HASH_LO: AtomicU32 = AtomicU32::new(0);
#[esp_hal::ram(rtc_fast, unstable(persistent))]
static LAST_HASH_HI: AtomicU32 = AtomicU32::new(0);
#[esp_hal::ram(rtc_fast, unstable(persistent))]
static FAILURES: AtomicU32 = AtomicU32::new(0);

/// FNV-1a hash of the last document successfully rendered to the panel
/// (0 = none since power-on).
pub fn last_hash() -> u64 {
    (u64::from(LAST_HASH_HI.load(Ordering::Relaxed)) << 32)
        | u64::from(LAST_HASH_LO.load(Ordering::Relaxed))
}

pub fn set_last_hash(hash: u64) {
    LAST_HASH_LO.store(hash as u32, Ordering::Relaxed);
    LAST_HASH_HI.store((hash >> 32) as u32, Ordering::Relaxed);
}

/// Consecutive failed fetch/render cycles.
pub fn failures() -> u32 {
    FAILURES.load(Ordering::Relaxed)
}

pub fn set_failures(n: u32) {
    FAILURES.store(n, Ordering::Relaxed);
}
