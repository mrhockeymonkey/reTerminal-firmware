//! Composition logic: turns a `screen_spec` value into drawn primitives via
//! `embedded-graphics` + `embedded-layout` + `embedded-text`, generic over
//! any `embedded_graphics::draw_target::DrawTarget` (design brief §7).
//!
//! Also owns the Spectra-6 palette quantization/dithering, so both the real
//! panel (`panel-backend`) and the browser preview (`web-preview`) produce
//! identical output for the same spec.
#![no_std]
