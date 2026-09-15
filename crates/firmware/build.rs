//! Build-time configuration for the firmware binary.
//!
//! * Emits the esp-hal linker script (`linkall.x`) — it must be the last
//!   linker script, hence here rather than in `.cargo/config.toml`.
//! * Re-runs when the env-baked settings (`WIFI_SSID`, `WIFI_PASSWORD`,
//!   `SCREEN_URL`, `POLL_INTERVAL_SECS`) change, so `option_env!` in
//!   `src/config.rs` picks them up.
//! * Fails early with a readable message when the Xtensa toolchain's
//!   linker is not on `PATH` (i.e. `espup`'s export file was not sourced).

fn main() {
    for var in [
        "WIFI_SSID",
        "WIFI_PASSWORD",
        "SCREEN_URL",
        "POLL_INTERVAL_SECS",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    check_xtensa_linker_available();
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

#[cfg(unix)]
fn check_xtensa_linker_available() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.starts_with("xtensa-") {
        return;
    }
    println!("cargo:rerun-if-env-changed=PATH");
    let linker = std::env::var(format!(
        "CARGO_TARGET_{}_LINKER",
        target.replace('-', "_").to_ascii_uppercase()
    ))
    .unwrap_or_else(|_| {
        let chip = target
            .strip_prefix("xtensa-")
            .and_then(|t| t.strip_suffix("-none-elf"))
            .unwrap_or("esp32");
        format!("xtensa-{chip}-elf-gcc")
    });
    if std::process::Command::new(&linker)
        .arg("--version")
        .output()
        .is_ok()
    {
        return;
    }
    panic!(
        "Xtensa linker `{linker}` not found on PATH.\n\
         This crate needs the esp-rs toolchain: install `espup`, run `espup install`,\n\
         then `source ~/export-esp.sh` before building. See crates/firmware/README.md."
    );
}

#[cfg(not(unix))]
fn check_xtensa_linker_available() {}
