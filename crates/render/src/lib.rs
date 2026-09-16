//! Composition logic: turns a [`screen_spec::ScreenSpec`] into drawn
//! primitives via `embedded-graphics` + `embedded-text`, generic over any
//! [`embedded_graphics::draw_target::DrawTarget`] whose colour is
//! [`Spectra6`] (design brief §7).
//!
//! This crate also owns everything colour-related for the E Ink Spectra 6
//! panel: the six-entry palette and its native 4-bit codes ([`Spectra6`]),
//! the RGB values the browser preview uses ([`palette`]), and the packed
//! 800×480×4bpp frame buffer ([`Frame`] / [`FrameMut`]) that both the real
//! panel driver and the preview consume byte-for-byte. Because the same
//! compiled code produces the frame on the device and in the browser, the
//! preview cannot drift from what gets flushed to the panel.
//!
//! No allocation, no `std`, no hardware or wasm dependencies.
#![no_std]
#![warn(missing_docs)]

mod color;
mod compose;
mod fonts;
mod frame;
pub mod palette;

pub use color::Spectra6;
pub use compose::{render, render_error, ErrorKind};
pub use frame::{Frame, FrameMut, FRAME_BYTES, HEIGHT, STRIDE, WIDTH};

pub use embedded_graphics;
pub use screen_spec;
