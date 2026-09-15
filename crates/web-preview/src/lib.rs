//! Browser preview backend (design brief §7, §9).
//!
//! Compiled to `wasm32-unknown-unknown`, this crate runs the very same
//! `render` code the firmware runs and exposes the result to JavaScript
//! through a deliberately tiny C ABI — no `wasm-bindgen`, no JS toolchain,
//! just `cargo build --target wasm32-unknown-unknown` and ~80 lines of
//! hand-written glue in `crates/server/assets/preview.js`:
//!
//! 1. JS copies the JSON document into the buffer at [`wp_spec_buf_ptr`]
//!    (at most [`wp_spec_buf_len`] bytes).
//! 2. JS calls [`wp_render`] with the byte count. The document is parsed
//!    and composed into a packed 4bpp [`render::Frame`] — exactly the bytes
//!    the device would send to the panel — which is then expanded to RGBA.
//! 3. JS wraps the RGBA bytes at [`wp_rgba_ptr`] in an `ImageData` and
//!    draws it on an 800×480 canvas.
//!
//! On a parse error the same error screen the device shows is rendered
//! instead, the status code says why, and [`wp_error_ptr`] carries the
//! message.
//!
//! Everything lives in static buffers (the module has no allocator and no
//! `std` on wasm). The host build of this crate (`rlib`) exists so the ABI
//! functions can be unit-tested with a normal `cargo test`; on the host the
//! statics are still process-wide, so tests must not call them concurrently.
#![cfg_attr(target_arch = "wasm32", no_std)]
#![warn(missing_docs)]

use core::cell::UnsafeCell;
use core::fmt::Write;

use render::palette::expand_rgba;
use render::{ErrorKind, Frame, FRAME_BYTES, HEIGHT, WIDTH};
use screen_spec::{ParseError, MAX_JSON_BYTES, MAX_TEXT};

/// `wp_render` succeeded.
pub const STATUS_OK: i32 = 0;
/// The document was malformed; an error screen was rendered.
pub const STATUS_INVALID_SPEC: i32 = 1;
/// The document's `version` is unsupported; an error screen was rendered.
pub const STATUS_UNSUPPORTED_VERSION: i32 = 2;
/// `len` exceeded the spec buffer; nothing was rendered.
pub const STATUS_TOO_LARGE: i32 = -1;

const RGBA_BYTES: usize = (WIDTH * HEIGHT * 4) as usize;
const ERR_BYTES: usize = 256;

/// A `static`-friendly cell. The wasm module is single-threaded, and the
/// host tests serialise their calls, so unsynchronised access is sound in
/// practice; the ABI functions are the only accessors.
struct Slot<T>(UnsafeCell<T>);
// SAFETY: see the type-level comment; access is single-threaded by contract.
unsafe impl<T> Sync for Slot<T> {}
impl<T> Slot<T> {
    const fn new(v: T) -> Self {
        Slot(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: single-threaded access by contract (see above).
        unsafe { &mut *self.0.get() }
    }
}

static SPEC_BUF: Slot<[u8; MAX_JSON_BYTES]> = Slot::new([0; MAX_JSON_BYTES]);
static UNESCAPE_BUF: Slot<[u8; MAX_TEXT]> = Slot::new([0; MAX_TEXT]);
static FRAME: Slot<Frame> = Slot::new(Frame::new());
static RGBA: Slot<[u8; RGBA_BYTES]> = Slot::new([0; RGBA_BYTES]);
static ERR: Slot<ErrBuf> = Slot::new(ErrBuf::new());

struct ErrBuf {
    buf: [u8; ERR_BYTES],
    len: usize,
}

impl ErrBuf {
    const fn new() -> Self {
        ErrBuf {
            buf: [0; ERR_BYTES],
            len: 0,
        }
    }
    fn clear(&mut self) {
        self.len = 0;
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl Write for ErrBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // Truncate at a char boundary rather than fail, so a long message
        // still yields a useful prefix.
        for ch in s.chars() {
            let mut tmp = [0u8; 4];
            let enc = ch.encode_utf8(&mut tmp).as_bytes();
            if self.len + enc.len() > ERR_BYTES {
                break;
            }
            self.buf[self.len..self.len + enc.len()].copy_from_slice(enc);
            self.len += enc.len();
        }
        Ok(())
    }
}

/// Pointer to the buffer JS writes the JSON document into.
#[no_mangle]
pub extern "C" fn wp_spec_buf_ptr() -> *mut u8 {
    SPEC_BUF.get().as_mut_ptr()
}

/// Capacity of the spec buffer in bytes (the same limit the device has).
#[no_mangle]
pub extern "C" fn wp_spec_buf_len() -> usize {
    MAX_JSON_BYTES
}

/// Panel width in pixels.
#[no_mangle]
pub extern "C" fn wp_width() -> u32 {
    WIDTH
}

/// Panel height in pixels.
#[no_mangle]
pub extern "C" fn wp_height() -> u32 {
    HEIGHT
}

/// Pointer to `wp_width() * wp_height() * 4` bytes of RGBA, valid after
/// `wp_render`.
#[no_mangle]
pub extern "C" fn wp_rgba_ptr() -> *const u8 {
    RGBA.get().as_ptr()
}

/// Pointer to the packed 4bpp frame (192,000 bytes) — byte-for-byte what
/// the device sends to the panel. Valid after `wp_render`.
#[no_mangle]
pub extern "C" fn wp_frame_ptr() -> *const u8 {
    FRAME.get().as_bytes().as_ptr()
}

/// Length of the packed frame in bytes.
#[no_mangle]
pub extern "C" fn wp_frame_len() -> usize {
    FRAME_BYTES
}

/// Pointer to the UTF-8 error message from the last `wp_render`.
#[no_mangle]
pub extern "C" fn wp_error_ptr() -> *const u8 {
    ERR.get().buf.as_ptr()
}

/// Length of the error message (0 when the last render succeeded).
#[no_mangle]
pub extern "C" fn wp_error_len() -> usize {
    ERR.get().len
}

/// Parses the first `len` bytes of the spec buffer and renders them.
///
/// Returns [`STATUS_OK`], or one of the other `STATUS_*` codes. For the
/// two "error screen" statuses the RGBA output is still valid and shows
/// the same notice the device would display.
#[no_mangle]
pub extern "C" fn wp_render(len: usize) -> i32 {
    let err = ERR.get();
    err.clear();
    if len > MAX_JSON_BYTES {
        let _ = write!(err, "document is {len} bytes; limit is {MAX_JSON_BYTES}");
        return STATUS_TOO_LARGE;
    }
    let json = &SPEC_BUF.get()[..len];
    let frame = FRAME.get();

    let status = match screen_spec::parse(json, UNESCAPE_BUF.get()) {
        Ok(spec) => {
            let _ = render::render(&spec, frame);
            STATUS_OK
        }
        Err(e) => {
            let _ = write!(err, "{e}");
            let kind = match e {
                ParseError::UnsupportedVersion(_) => ErrorKind::UnsupportedVersion,
                ParseError::Json(_) => ErrorKind::InvalidSpec,
            };
            let _ = render::render_error(kind, err.as_str(), frame);
            match kind {
                ErrorKind::UnsupportedVersion => STATUS_UNSUPPORTED_VERSION,
                _ => STATUS_INVALID_SPEC,
            }
        }
    };

    expand_rgba(frame.as_bytes(), RGBA.get());
    status
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
