//! The packed 800×480×4bpp frame buffer.
//!
//! Layout matches what the ED2208 controller expects on the wire (and what
//! GxEPD2 / `epdsi` send): rows top to bottom, two pixels per byte, the
//! left-hand pixel of each pair in the high nibble. [`Frame::as_bytes`] is
//! therefore exactly the payload of the panel's "data start transmission"
//! command, and exactly what the browser preview expands into RGBA.

use core::convert::Infallible;

use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;

use crate::Spectra6;

/// Panel width in pixels.
pub const WIDTH: u32 = screen_spec::SCREEN_WIDTH;
/// Panel height in pixels.
pub const HEIGHT: u32 = screen_spec::SCREEN_HEIGHT;
/// Bytes per row (two pixels per byte).
pub const STRIDE: usize = (WIDTH / 2) as usize;
/// Total bytes in a frame: 192,000.
pub const FRAME_BYTES: usize = STRIDE * HEIGHT as usize;

const WHITE_BYTE: u8 = Spectra6::pack(Spectra6::White, Spectra6::White);

// ---- shared pixel operations on a raw byte slice --------------------------

#[inline]
fn set_pixel(bytes: &mut [u8], x: u32, y: u32, c: Spectra6) {
    if x >= WIDTH || y >= HEIGHT {
        return;
    }
    let i = y as usize * STRIDE + (x / 2) as usize;
    let b = &mut bytes[i];
    if x & 1 == 0 {
        *b = (*b & 0x0f) | (c.nibble() << 4);
    } else {
        *b = (*b & 0xf0) | c.nibble();
    }
}

#[inline]
fn get_code(bytes: &[u8], x: u32, y: u32) -> Option<u8> {
    if x >= WIDTH || y >= HEIGHT {
        return None;
    }
    let b = bytes[y as usize * STRIDE + (x / 2) as usize];
    Some(if x & 1 == 0 { b >> 4 } else { b & 0x0f })
}

fn fill_rect(bytes: &mut [u8], area: &Rectangle, c: Spectra6) {
    let bounds = Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT));
    let area = area.intersection(&bounds);
    let Some(br) = area.bottom_right() else {
        return;
    };
    let (x0, y0) = (area.top_left.x as u32, area.top_left.y as u32);
    let (x1, y1) = (br.x as u32, br.y as u32); // inclusive
    let packed = Spectra6::pack(c, c);

    for y in y0..=y1 {
        let row = &mut bytes[y as usize * STRIDE..(y as usize + 1) * STRIDE];
        let mut xs = x0;
        let mut xe = x1;
        // Odd left edge: the pixel sits in a low nibble.
        if xs & 1 == 1 {
            row[(xs / 2) as usize] = (row[(xs / 2) as usize] & 0xf0) | c.nibble();
            xs += 1;
        }
        // Even right edge: the pixel sits in a high nibble.
        if xe & 1 == 0 && xe >= xs {
            row[(xe / 2) as usize] = (row[(xe / 2) as usize] & 0x0f) | (c.nibble() << 4);
            if xe == 0 {
                continue;
            }
            xe -= 1;
        }
        if xs <= xe {
            row[(xs / 2) as usize..=(xe / 2) as usize].fill(packed);
        }
    }
}

fn draw_pixels<I>(bytes: &mut [u8], pixels: I)
where
    I: IntoIterator<Item = Pixel<Spectra6>>,
{
    for Pixel(p, c) in pixels {
        if p.x >= 0 && p.y >= 0 {
            set_pixel(bytes, p.x as u32, p.y as u32, c);
        }
    }
}

// ---- owned frame ------------------------------------------------------------

/// An owned frame buffer (192,000 bytes).
///
/// Large: keep it in a `static` (wasm), on the heap (host tests), or use
/// [`FrameMut`] over a buffer that already lives somewhere sensible
/// (firmware, where it must be in PSRAM). Never put one on an embedded stack.
#[derive(Clone)]
pub struct Frame {
    data: [u8; FRAME_BYTES],
}

impl Frame {
    /// A white frame.
    pub const fn new() -> Self {
        Frame {
            data: [WHITE_BYTE; FRAME_BYTES],
        }
    }

    /// The packed pixel data, ready to send to the panel.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Mutable access to the packed pixel data.
    #[inline]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// A borrowed [`FrameMut`] view.
    #[inline]
    pub fn view(&mut self) -> FrameMut<'_> {
        FrameMut {
            data: &mut self.data,
        }
    }

    /// Sets one pixel; out-of-range coordinates are ignored.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, c: Spectra6) {
        set_pixel(&mut self.data, x, y, c)
    }

    /// Reads one pixel, or `None` when out of range or when the stored code
    /// is not a displayable colour.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Spectra6> {
        get_code(&self.data, x, y).and_then(Spectra6::from_nibble)
    }

    /// Fills the whole frame with one colour.
    pub fn fill(&mut self, c: Spectra6) {
        self.data.fill(Spectra6::pack(c, c));
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &WIDTH)
            .field("height", &HEIGHT)
            .field("bytes", &FRAME_BYTES)
            .finish()
    }
}

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for Frame {
    type Color = Spectra6;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        draw_pixels(&mut self.data, pixels);
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        fill_rect(&mut self.data, area, color);
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.fill(color);
        Ok(())
    }
}

// ---- borrowed frame ---------------------------------------------------------

/// A frame buffer borrowed from any 192,000-byte slice.
///
/// This is what the firmware uses: the bytes live in PSRAM, allocated once,
/// and are lent to the renderer without ever copying a full frame.
pub struct FrameMut<'a> {
    data: &'a mut [u8],
}

impl<'a> FrameMut<'a> {
    /// Wraps `data`, which must be exactly [`FRAME_BYTES`] long.
    pub fn new(data: &'a mut [u8]) -> Option<Self> {
        (data.len() == FRAME_BYTES).then_some(FrameMut { data })
    }

    /// The packed pixel data, ready to send to the panel.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    /// Sets one pixel; out-of-range coordinates are ignored.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, c: Spectra6) {
        set_pixel(self.data, x, y, c)
    }

    /// Reads one pixel, or `None` when out of range or undisplayable.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Spectra6> {
        get_code(self.data, x, y).and_then(Spectra6::from_nibble)
    }

    /// Fills the whole frame with one colour.
    pub fn fill(&mut self, c: Spectra6) {
        self.data.fill(Spectra6::pack(c, c));
    }
}

impl OriginDimensions for FrameMut<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for FrameMut<'_> {
    type Color = Spectra6;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        draw_pixels(self.data, pixels);
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        fill_rect(self.data, area, color);
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.fill(color);
        Ok(())
    }
}
