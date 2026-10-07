//! State that survives deep sleep.
//!
//! These statics live in RTC fast memory, which stays powered through deep
//! sleep only because `power::deep_sleep` keeps it on (esp-hal's default
//! deep-sleep config powers it down). They are zeroed on a power-on reset
//! and preserved across timer/button wakes, panics and watchdog resets.
//!
//! esp-hal's `Persistable` marker covers plain integers (on the esp nightly
//! toolchain `AtomicU32` is the generic `Atomic<u32>`, which it does not
//! cover), so these are `static mut`s reached through raw pointers with
//! volatile accesses. They are only touched from the main task, never
//! concurrently, which is what makes that sound.

#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut LAST_HASH: u64 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut FAILURES: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut LAST_BAR: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut BATTERY_LOW: u32 = 0;

/// FNV-1a hash of the last document successfully rendered to the panel
/// (0 = none since power-on).
pub fn last_hash() -> u64 {
    // SAFETY: single-threaded access from the main task; see module docs.
    unsafe { core::ptr::read_volatile(&raw const LAST_HASH) }
}

pub fn set_last_hash(hash: u64) {
    // SAFETY: as above.
    unsafe { core::ptr::write_volatile(&raw mut LAST_HASH, hash) }
}

/// Consecutive failed fetch/render cycles.
pub fn failures() -> u32 {
    // SAFETY: as above.
    unsafe { core::ptr::read_volatile(&raw const FAILURES) }
}

pub fn set_failures(n: u32) {
    // SAFETY: as above.
    unsafe { core::ptr::write_volatile(&raw mut FAILURES, n) }
}

/// What the status bar on the panel currently shows: a `Bar` code
/// (0 = normal after a power-on reset).
pub fn last_bar() -> u32 {
    // SAFETY: as above.
    unsafe { core::ptr::read_volatile(&raw const LAST_BAR) }
}

pub fn set_last_bar(bar: u32) {
    // SAFETY: as above.
    unsafe { core::ptr::write_volatile(&raw mut LAST_BAR, bar) }
}

/// Whether the battery warning is latched (see `config::BATTERY_OK_PERCENT`).
pub fn battery_low() -> bool {
    // SAFETY: as above.
    unsafe { core::ptr::read_volatile(&raw const BATTERY_LOW) != 0 }
}

pub fn set_battery_low(low: bool) {
    // SAFETY: as above.
    unsafe { core::ptr::write_volatile(&raw mut BATTERY_LOW, u32::from(low)) }
}
