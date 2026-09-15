//! ESP32-S3 firmware binary (design brief §7): `esp-hal` init, deep-sleep/
//! wake handling, `render` + `panel-backend`, WiFi fetch cycle (§10).
//!
//! Placeholder only — see `README.md` in this directory for why this crate
//! can't be built in a Claude Code Remote session, and CI's `firmware`
//! job for where it actually gets compile-checked.
#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
