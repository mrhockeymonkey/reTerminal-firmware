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
