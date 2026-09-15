//! Shared "screen spec" content contract (design brief §8).
//!
//! Serde types for the JSON document that flows: `server` (holds/serves it)
//! -> `firmware` (fetches and renders it) -> `web-preview` (renders the same
//! thing in a browser). Kept `no_std` and allocation-free so it can be used
//! unmodified from the ESP32-S3 firmware; `server` uses the same types with
//! `serde_json` from a `std` context.
//!
//! # v1 schema
//!
//! ```json
//! {
//!   "version": 1,
//!   "background": "white",
//!   "regions": [
//!     { "rect": [0, 0, 800, 80], "text": "Kitchen Display", "style": "title",
//!       "align": "center", "valign": "middle", "color": "black", "background": "yellow" },
//!     { "rect": [20, 100, 760, 360], "text": "Bin day: Thursday\nNext: 14:00", "style": "body" },
//!     { "rect": [0, 470, 800, 10], "background": "blue" }
//!   ]
//! }
//! ```
//!
//! * `version` must equal [`VERSION`]; anything else is rejected with
//!   [`ParseError::UnsupportedVersion`] so old firmware never mis-renders a
//!   newer document.
//! * Unknown fields are ignored, which is what lets the schema grow (e.g. a
//!   bitmap region post-v1) without breaking older consumers.
//! * Regions are drawn in order: background fill, border, then text, each
//!   clipped to its `rect`. Later regions paint over earlier ones.
//! * Sizes are bounded ([`MAX_REGIONS`], [`MAX_TEXT`], [`MAX_JSON_BYTES`]) so
//!   the device can hold a whole document in fixed buffers.
#![no_std]
#![warn(missing_docs)]

use core::fmt;

use heapless::{String, Vec};
use serde::{Deserialize, Serialize};

/// The only schema version this crate understands.
pub const VERSION: u8 = 1;
/// Maximum number of regions in one document.
pub const MAX_REGIONS: usize = 16;
/// Maximum length in bytes of a region's text after unescaping.
pub const MAX_TEXT: usize = 512;
/// Maximum size in bytes of the JSON document on the wire. The device's
/// receive buffer is this size and the server rejects larger `PUT`s.
pub const MAX_JSON_BYTES: usize = 16 * 1024;
/// Panel width in pixels.
pub const SCREEN_WIDTH: u32 = 800;
/// Panel height in pixels.
pub const SCREEN_HEIGHT: u32 = 480;

/// One of the six colours the E Ink Spectra 6 panel can show.
///
/// The panel's native palette encoding also has an "orange" code, but the
/// GDEP073E01 is a Spectra 6 (not ACeP 7) panel and renders it as an
/// undefined colour, so it is deliberately not representable here.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum Colour {
    /// Black.
    Black,
    /// White (the panel's paper colour).
    #[default]
    White,
    /// Yellow.
    Yellow,
    /// Red.
    Red,
    /// Blue.
    Blue,
    /// Green.
    Green,
}

impl Colour {
    /// Every colour, in palette order.
    pub const ALL: [Colour; 6] = [
        Colour::Black,
        Colour::White,
        Colour::Yellow,
        Colour::Red,
        Colour::Blue,
        Colour::Green,
    ];

    /// The colour's lower-case JSON name.
    pub const fn name(self) -> &'static str {
        match self {
            Colour::Black => "black",
            Colour::White => "white",
            Colour::Yellow => "yellow",
            Colour::Red => "red",
            Colour::Blue => "blue",
            Colour::Green => "green",
        }
    }

    const fn black() -> Self {
        Colour::Black
    }
}

/// Font preset for a text region. Actual fonts are chosen by the `render` crate.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextStyle {
    /// Largest: a screen title.
    Title,
    /// Section heading.
    Header,
    /// Regular body text.
    #[default]
    Body,
    /// Footnotes, timestamps.
    Small,
}

/// Horizontal text alignment inside a region.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum HAlign {
    /// Left-aligned.
    #[default]
    Left,
    /// Centred.
    Center,
    /// Right-aligned.
    Right,
}

/// Vertical text alignment inside a region.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    /// Text starts at the top of the region.
    #[default]
    Top,
    /// Text is vertically centred.
    Middle,
    /// Text ends at the bottom of the region.
    Bottom,
}

/// A stroked border drawn just inside a region's rectangle.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Border {
    /// Stroke colour.
    pub color: Colour,
    /// Stroke width in pixels (default 2).
    #[serde(default = "Border::default_width")]
    pub width: u8,
}

impl Border {
    const fn default_width() -> u8 {
        2
    }
}

/// A rectangular area of the screen with optional fill, border and text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// `[x, y, width, height]` in pixels from the top-left corner. May extend
    /// past the screen edge; drawing is clipped.
    pub rect: [i32; 4],
    /// Text to draw, word-wrapped to the region. `\n` starts a new line.
    #[serde(default)]
    pub text: String<MAX_TEXT>,
    /// Font preset.
    #[serde(default)]
    pub style: TextStyle,
    /// Horizontal alignment.
    #[serde(default)]
    pub align: HAlign,
    /// Vertical alignment.
    #[serde(default)]
    pub valign: VAlign,
    /// Text colour (default black).
    #[serde(default = "Colour::black")]
    pub color: Colour,
    /// Fill colour; `None` leaves whatever was drawn underneath.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Colour>,
    /// Optional border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
}

impl Region {
    /// A region covering `rect` with default styling and no text.
    pub fn new(rect: [i32; 4]) -> Self {
        Region {
            rect,
            text: String::new(),
            style: TextStyle::default(),
            align: HAlign::default(),
            valign: VAlign::default(),
            color: Colour::Black,
            background: None,
            border: None,
        }
    }

    /// Width in pixels (never negative).
    pub fn width(&self) -> u32 {
        self.rect[2].max(0) as u32
    }

    /// Height in pixels (never negative).
    pub fn height(&self) -> u32 {
        self.rect[3].max(0) as u32
    }
}

/// A complete screen: the document served by `GET /screen`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ScreenSpec {
    /// Schema version; must be [`VERSION`].
    pub version: u8,
    /// Colour the whole screen is cleared to before regions are drawn.
    #[serde(default)]
    pub background: Colour,
    /// Regions, drawn in order.
    #[serde(default)]
    pub regions: Vec<Region, MAX_REGIONS>,
}

impl ScreenSpec {
    /// An empty, white screen at the current schema version.
    pub const fn empty() -> Self {
        ScreenSpec {
            version: VERSION,
            background: Colour::White,
            regions: Vec::new(),
        }
    }
}

impl Default for ScreenSpec {
    fn default() -> Self {
        Self::empty()
    }
}

/// Why a document could not be turned into a [`ScreenSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The JSON was malformed, exceeded a size bound, or had a wrongly typed
    /// field.
    Json(serde_json_core::de::Error),
    /// `version` was present but is not one this crate understands.
    UnsupportedVersion(u8),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Json(e) => write!(f, "invalid screen spec JSON: {e}"),
            ParseError::UnsupportedVersion(v) => {
                write!(
                    f,
                    "unsupported screen spec version {v} (expected {VERSION})"
                )
            }
        }
    }
}

impl From<serde_json_core::de::Error> for ParseError {
    fn from(e: serde_json_core::de::Error) -> Self {
        ParseError::Json(e)
    }
}

/// Parses a JSON document without allocating.
///
/// `unescape_buf` is scratch space used to decode escape sequences such as
/// `\n` inside strings; it must be at least [`MAX_TEXT`] bytes or long
/// strings with escapes will fail with [`ParseError::Json`].
pub fn parse(bytes: &[u8], unescape_buf: &mut [u8]) -> Result<ScreenSpec, ParseError> {
    let (spec, _consumed): (ScreenSpec, usize) =
        serde_json_core::from_slice_escaped(bytes, unescape_buf)?;
    if spec.version != VERSION {
        return Err(ParseError::UnsupportedVersion(spec.version));
    }
    Ok(spec)
}

/// Serialises a spec into `out`, returning the number of bytes written.
///
/// Used by the browser preview and tests; the server uses `serde_json`.
pub fn to_json(spec: &ScreenSpec, out: &mut [u8]) -> Result<usize, serde_json_core::ser::Error> {
    serde_json_core::to_slice(spec, out)
}

/// 64-bit FNV-1a hash of a document's raw bytes.
///
/// The device stores the hash of the last document it rendered and skips the
/// (multi-second, full-flash) panel refresh when the next fetch hashes the
/// same; the server exposes the same value as the `ETag` of `GET /screen`.
pub fn content_hash(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes
        .iter()
        .fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}
