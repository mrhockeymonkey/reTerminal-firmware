//! Wires `render`'s `DrawTarget` output to the real GDEP073E01 panel driver
//! (`gdep073e01` or `epdsi`) over SPI (design brief §7).
//!
//! ESP32-S3-only: not a `default-members` crate (see root `Cargo.toml`).
//! Building it requires the Xtensa Rust toolchain — see
//! `crates/firmware/README.md`.
#![no_std]
