//! The Spectra 6 colour type and its native panel codes.

use embedded_graphics::pixelcolor::raw::{RawData, RawU4};
use embedded_graphics::pixelcolor::PixelColor;
use screen_spec::Colour;

/// One of the six colours the GDEP073E01 (E Ink Spectra 6) panel can show.
///
/// The discriminants are the panel's native 4-bit codes, written straight
/// into display RAM (two pixels per byte, first pixel in the high nibble).
/// The codes are shared with GxEPD2, `epdsi`, `gdep073e01` and Zephyr.
///
/// Code `0x4` is "orange" on the older ACeP 7-colour panels and renders as
/// an undefined colour on Spectra 6, and `0x7` is the controller's "clean"
/// code; neither is representable here, so a frame built from this type can
/// never contain them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Spectra6 {
    /// Black, code 0x0.
    Black = 0x0,
    /// White, code 0x1 (the panel's paper colour).
    #[default]
    White = 0x1,
    /// Yellow, code 0x2.
    Yellow = 0x2,
    /// Red, code 0x3.
    Red = 0x3,
    /// Blue, code 0x5.
    Blue = 0x5,
    /// Green, code 0x6.
    Green = 0x6,
}

impl Spectra6 {
    /// Every colour, in palette order.
    pub const ALL: [Spectra6; 6] = [
        Spectra6::Black,
        Spectra6::White,
        Spectra6::Yellow,
        Spectra6::Red,
        Spectra6::Blue,
        Spectra6::Green,
    ];

    /// The panel's native 4-bit code for this colour.
    #[inline]
    pub const fn nibble(self) -> u8 {
        self as u8
    }

    /// The colour for a native 4-bit code, or `None` for the two codes the
    /// panel cannot display (`0x4` orange, `0x7` clean) and out-of-range input.
    #[inline]
    pub const fn from_nibble(code: u8) -> Option<Self> {
        match code {
            0x0 => Some(Spectra6::Black),
            0x1 => Some(Spectra6::White),
            0x2 => Some(Spectra6::Yellow),
            0x3 => Some(Spectra6::Red),
            0x5 => Some(Spectra6::Blue),
            0x6 => Some(Spectra6::Green),
            _ => None,
        }
    }

    /// Two pixels packed into one display-RAM byte, `high` first.
    #[inline]
    pub const fn pack(high: Self, low: Self) -> u8 {
        (high.nibble() << 4) | low.nibble()
    }
}

impl PixelColor for Spectra6 {
    type Raw = RawU4;
}

impl From<Spectra6> for RawU4 {
    fn from(c: Spectra6) -> Self {
        RawU4::new(c.nibble())
    }
}

impl From<RawU4> for Spectra6 {
    /// Undisplayable codes map to white so that raw-image conversions never
    /// smuggle an undefined colour into a frame.
    fn from(raw: RawU4) -> Self {
        Spectra6::from_nibble(raw.into_inner()).unwrap_or(Spectra6::White)
    }
}

impl From<Colour> for Spectra6 {
    fn from(c: Colour) -> Self {
        match c {
            Colour::Black => Spectra6::Black,
            Colour::White => Spectra6::White,
            Colour::Yellow => Spectra6::Yellow,
            Colour::Red => Spectra6::Red,
            Colour::Blue => Spectra6::Blue,
            Colour::Green => Spectra6::Green,
        }
    }
}

impl From<Spectra6> for Colour {
    fn from(c: Spectra6) -> Self {
        match c {
            Spectra6::Black => Colour::Black,
            Spectra6::White => Colour::White,
            Spectra6::Yellow => Colour::Yellow,
            Spectra6::Red => Colour::Red,
            Spectra6::Blue => Colour::Blue,
            Spectra6::Green => Colour::Green,
        }
    }
}
