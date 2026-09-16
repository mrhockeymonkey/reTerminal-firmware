//! Exercises the C ABI on the host build. All calls share process-wide
//! static buffers, so everything runs inside one test function.

use render::palette::RGB_BY_CODE;
use render::{Frame, FRAME_BYTES};
use screen_spec::MAX_JSON_BYTES;
use web_preview::*;

const KITCHEN: &[u8] = include_bytes!("../../screen-spec/samples/kitchen.json");

fn load(json: &[u8]) -> i32 {
    assert!(json.len() <= wp_spec_buf_len());
    // SAFETY: the pointer is to a static buffer of wp_spec_buf_len() bytes.
    unsafe { std::ptr::copy_nonoverlapping(json.as_ptr(), wp_spec_buf_ptr(), json.len()) };
    wp_render(json.len())
}

fn rgba() -> &'static [u8] {
    // SAFETY: static buffer of width*height*4 bytes, valid after wp_render.
    unsafe { std::slice::from_raw_parts(wp_rgba_ptr(), (wp_width() * wp_height() * 4) as usize) }
}

fn frame_bytes() -> &'static [u8] {
    // SAFETY: static buffer of wp_frame_len() bytes.
    unsafe { std::slice::from_raw_parts(wp_frame_ptr(), wp_frame_len()) }
}

fn error() -> String {
    // SAFETY: static buffer; wp_error_len() bytes are valid UTF-8.
    let bytes = unsafe { std::slice::from_raw_parts(wp_error_ptr(), wp_error_len()) };
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn abi_end_to_end() {
    assert_eq!(wp_width(), 800);
    assert_eq!(wp_height(), 480);
    assert_eq!(wp_spec_buf_len(), MAX_JSON_BYTES);
    assert_eq!(wp_frame_len(), FRAME_BYTES);

    // A good document renders, matches a direct render, and expands to RGBA.
    assert_eq!(load(KITCHEN), STATUS_OK);
    assert_eq!(error(), "");
    let mut scratch = [0u8; screen_spec::MAX_TEXT];
    let spec = screen_spec::parse(KITCHEN, &mut scratch).unwrap();
    let mut direct = Box::new(Frame::new());
    render::render(&spec, &mut *direct).unwrap();
    assert_eq!(frame_bytes(), direct.as_bytes());
    let px = rgba();
    assert_eq!(px.len(), 800 * 480 * 4);
    // Top-left pixel is the black title banner.
    assert_eq!(&px[0..4], &[0, 0, 0, 255]);
    // Bottom-left pixel is the blue footer.
    let last_row = 479 * 800 * 4;
    assert_eq!(&px[last_row..last_row + 3], &RGB_BY_CODE[5]);

    // Malformed JSON: error status, message, and an error screen (red banner).
    assert_eq!(load(b"{\"version\":1,\"regions\":[{"), STATUS_INVALID_SPEC);
    assert!(error().contains("invalid screen spec JSON"), "{}", error());
    assert_eq!(&rgba()[0..3], &RGB_BY_CODE[3], "red banner");

    // Unsupported version: distinct status.
    assert_eq!(load(b"{\"version\":7}"), STATUS_UNSUPPORTED_VERSION);
    assert!(error().contains("unsupported screen spec version 7"));
    assert_eq!(&rgba()[0..3], &RGB_BY_CODE[3]);

    // Too large: rejected up front; the previous image is left in place.
    assert_eq!(wp_render(MAX_JSON_BYTES + 1), STATUS_TOO_LARGE);
    assert!(error().contains("limit"));

    // And back to a good document clears the error.
    assert_eq!(load(b"{\"version\":1}"), STATUS_OK);
    assert_eq!(error(), "");
    assert_eq!(&rgba()[0..3], &RGB_BY_CODE[1], "white screen");
}
