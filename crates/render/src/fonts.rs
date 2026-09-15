//! Font presets behind [`screen_spec::TextStyle`].
//!
//! `u8g2` fonts were chosen over `embedded-graphics`' built-in mono fonts
//! because the largest built-in is 10×20 px, which is unreadably small on an
//! 800×480, 125 PPI wall display. The `_tf` variants carry the full Latin-1
//! glyph set (accents, symbols) with transparent backgrounds.

use screen_spec::TextStyle;
use u8g2_fonts::{fonts, U8g2TextStyle};

use crate::Spectra6;

/// The character style for a preset in a colour.
pub fn text_style(style: TextStyle, color: Spectra6) -> U8g2TextStyle<Spectra6> {
    match style {
        TextStyle::Title => U8g2TextStyle::new(fonts::u8g2_font_logisoso42_tf, color),
        TextStyle::Header => U8g2TextStyle::new(fonts::u8g2_font_helvB24_tf, color),
        TextStyle::Body => U8g2TextStyle::new(fonts::u8g2_font_helvR18_tf, color),
        TextStyle::Small => U8g2TextStyle::new(fonts::u8g2_font_helvR12_tf, color),
    }
}

/// Inner padding between a region's edge and its text, in pixels.
pub const PADDING: u32 = 6;
