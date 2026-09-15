//! sRGB approximations of the panel's six inks, for the browser preview.
//!
//! Values follow Zephyr's mainline `reterminal_e1002` board port
//! (`zephyr,panel-color-palette`), which look like measured panel colours
//! rather than saturated primaries: e-paper white is slightly warm and the
//! inks are muted. Keeping the table here, next to [`Spectra6`], means the
//! preview and the device share a single definition of "what red is".

use crate::Spectra6;

/// RGB for each native 4-bit code, indexed by code (`0x0..=0x7`).
///
/// Codes `0x4` (ACeP orange, undefined on Spectra 6) and `0x7` (clean) can
/// never appear in a frame produced by this crate; they are given a loud
/// magenta and white respectively so a bug that emits them is obvious in the
/// preview instead of silently plausible.
pub const RGB_BY_CODE: [[u8; 3]; 8] = [
    [0, 0, 0],       // 0x0 black
    [245, 245, 240], // 0x1 white
    [230, 200, 40],  // 0x2 yellow
    [196, 50, 58],   // 0x3 red
    [255, 0, 255],   // 0x4 (orange on ACeP; not displayable here)
    [40, 85, 180],   // 0x5 blue
    [60, 140, 75],   // 0x6 green
    [245, 245, 240], // 0x7 clean (renders as white)
];

/// RGB for a colour.
#[inline]
pub const fn rgb(c: Spectra6) -> [u8; 3] {
    RGB_BY_CODE[c.nibble() as usize]
}

/// Expands a packed 4bpp frame (`src`, two pixels per byte, first pixel in
/// the high nibble) into RGBA8888 (`dst`, four bytes per pixel, alpha 255).
///
/// `dst` must be at least `src.len() * 2 * 4` bytes; extra bytes are left
/// untouched. Returns the number of bytes written.
pub fn expand_rgba(src: &[u8], dst: &mut [u8]) -> usize {
    let mut written = 0;
    for (byte, out) in src.iter().zip(dst.chunks_exact_mut(8)) {
        let hi = RGB_BY_CODE[usize::from(byte >> 4)];
        let lo = RGB_BY_CODE[usize::from(byte & 0x0f)];
        out[0..3].copy_from_slice(&hi);
        out[3] = 255;
        out[4..7].copy_from_slice(&lo);
        out[7] = 255;
        written += 8;
    }
    written
}
