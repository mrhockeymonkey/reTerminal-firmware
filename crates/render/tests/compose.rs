use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use render::palette::{expand_rgba, rgb, RGB_BY_CODE};
use render::{
    render, render_error, ErrorKind, Frame, FrameMut, Spectra6, FRAME_BYTES, HEIGHT, STRIDE, WIDTH,
};
use screen_spec::{Colour, ScreenSpec, MAX_TEXT};

const KITCHEN: &str = include_str!("../../screen-spec/samples/kitchen.json");
const MINIMAL: &str = include_str!("../../screen-spec/samples/minimal.json");

fn spec(json: &str) -> ScreenSpec {
    let mut scratch = [0u8; MAX_TEXT];
    screen_spec::parse(json.as_bytes(), &mut scratch).unwrap()
}

fn frame() -> Box<Frame> {
    Box::new(Frame::new())
}

/// Every nibble in the frame is one of the six displayable codes.
fn assert_only_displayable(bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        for code in [b >> 4, b & 0x0f] {
            assert!(
                Spectra6::from_nibble(code).is_some(),
                "byte {i} holds undisplayable code {code:#x}"
            );
        }
    }
}

fn count(f: &Frame, area: Rectangle, c: Spectra6) -> usize {
    area.points()
        .filter(|p| f.get_pixel(p.x as u32, p.y as u32) == Some(c))
        .count()
}

#[test]
fn geometry_constants() {
    assert_eq!(WIDTH, 800);
    assert_eq!(HEIGHT, 480);
    assert_eq!(STRIDE, 400);
    assert_eq!(FRAME_BYTES, 192_000);
    assert_eq!(frame().as_bytes().len(), FRAME_BYTES);
    assert_eq!(frame().size(), Size::new(800, 480));
}

#[test]
fn nibble_codes_match_the_panel() {
    assert_eq!(Spectra6::Black.nibble(), 0x0);
    assert_eq!(Spectra6::White.nibble(), 0x1);
    assert_eq!(Spectra6::Yellow.nibble(), 0x2);
    assert_eq!(Spectra6::Red.nibble(), 0x3);
    assert_eq!(Spectra6::Blue.nibble(), 0x5);
    assert_eq!(Spectra6::Green.nibble(), 0x6);
    assert_eq!(Spectra6::from_nibble(0x4), None);
    assert_eq!(Spectra6::from_nibble(0x7), None);
    assert_eq!(Spectra6::from_nibble(0x8), None);
    for c in Spectra6::ALL {
        assert_eq!(Spectra6::from_nibble(c.nibble()), Some(c));
        assert_eq!(Spectra6::from(Colour::from(c)), c);
    }
    assert_eq!(Spectra6::pack(Spectra6::Black, Spectra6::Red), 0x03);
    assert_eq!(Spectra6::pack(Spectra6::Green, Spectra6::White), 0x61);
}

#[test]
fn pixel_packing_first_pixel_in_high_nibble() {
    let mut f = frame();
    assert!(
        f.as_bytes().iter().all(|&b| b == 0x11),
        "new frame is white"
    );

    f.set_pixel(0, 0, Spectra6::Black);
    assert_eq!(f.as_bytes()[0], 0x01);
    f.set_pixel(1, 0, Spectra6::Red);
    assert_eq!(f.as_bytes()[0], 0x03);
    f.set_pixel(799, 479, Spectra6::Green);
    assert_eq!(f.as_bytes()[FRAME_BYTES - 1], 0x16);
    assert_eq!(f.get_pixel(799, 479), Some(Spectra6::Green));
    assert_eq!(f.get_pixel(798, 479), Some(Spectra6::White));

    // Out of range is ignored, not a panic.
    f.set_pixel(800, 0, Spectra6::Black);
    f.set_pixel(0, 480, Spectra6::Black);
    assert_eq!(f.get_pixel(800, 0), None);
    f.draw_iter([
        Pixel(Point::new(-1, -1), Spectra6::Black),
        Pixel(Point::new(5000, 5000), Spectra6::Black),
    ])
    .unwrap();
}

#[test]
fn fill_solid_matches_per_pixel_on_odd_edges() {
    for (x, y, w, h) in [
        (0, 0, 800, 480),
        (1, 3, 5, 2),
        (3, 0, 1, 1),
        (798, 10, 1, 1),
        (797, 10, 3, 4),
        (-5, -5, 10, 10),
        (795, 475, 20, 20),
        (0, 0, 0, 0),
        (2, 2, 1, 1),
    ] {
        let area = Rectangle::new(Point::new(x, y), Size::new(w, h));

        let mut fast = frame();
        fast.fill_solid(&area, Spectra6::Blue).unwrap();

        let mut slow = frame();
        for p in area.points() {
            if p.x >= 0 && p.y >= 0 {
                slow.set_pixel(p.x as u32, p.y as u32, Spectra6::Blue);
            }
        }
        assert_eq!(fast.as_bytes(), slow.as_bytes(), "area {area:?}");
    }
}

#[test]
fn frame_mut_shares_behaviour_with_frame() {
    let mut owned = frame();
    let mut buf = vec![0u8; FRAME_BYTES];
    {
        let mut view = FrameMut::new(&mut buf).unwrap();
        view.fill(Spectra6::White);
        render(&spec(KITCHEN), &mut view).unwrap();
    }
    render(&spec(KITCHEN), &mut *owned).unwrap();
    assert_eq!(owned.as_bytes(), &buf[..]);

    assert!(FrameMut::new(&mut [0u8; 10]).is_none());
    assert!(FrameMut::new(&mut vec![0u8; FRAME_BYTES + 1]).is_none());
}

#[test]
fn kitchen_sample_renders_deterministically_and_within_palette() {
    let s = spec(KITCHEN);
    let mut a = frame();
    render(&s, &mut *a).unwrap();
    let mut b = frame();
    b.fill(Spectra6::Green); // starting content must not matter
    render(&s, &mut *b).unwrap();
    assert_eq!(a.as_bytes(), b.as_bytes());
    assert_only_displayable(a.as_bytes());

    // Title banner: black background with white glyphs.
    let banner = Rectangle::new(Point::new(0, 0), Size::new(800, 90));
    let black = count(&a, banner, Spectra6::Black);
    let white = count(&a, banner, Spectra6::White);
    assert!(black > 800 * 90 / 2, "banner mostly black: {black}");
    assert!(white > 200, "title glyphs present: {white}");
    // The corner pixel (outside any glyph) is the fill colour.
    assert_eq!(a.get_pixel(0, 0), Some(Spectra6::Black));

    // Blue header text on white.
    let header = Rectangle::new(Point::new(24, 110), Size::new(752, 60));
    assert!(count(&a, header, Spectra6::Blue) > 100);

    // Green border, 3 px, inside the list region; interior stays white.
    assert_eq!(a.get_pixel(24, 180), Some(Spectra6::Green));
    assert_eq!(a.get_pixel(26, 182), Some(Spectra6::Green));
    assert_eq!(a.get_pixel(27, 183), Some(Spectra6::White));
    assert_eq!(a.get_pixel(24 + 359, 180 + 249), Some(Spectra6::Green));
    assert_eq!(a.get_pixel(23, 179), Some(Spectra6::White));

    // Yellow box with red text.
    let appt = Rectangle::new(Point::new(416, 180), Size::new(360, 250));
    assert!(count(&a, appt, Spectra6::Red) > 100);
    assert_eq!(a.get_pixel(416, 180), Some(Spectra6::Yellow));

    // Footer strip is blue with white text; nothing below y=480 exists.
    assert_eq!(a.get_pixel(0, 479), Some(Spectra6::Blue));
    let footer = Rectangle::new(Point::new(0, 450), Size::new(800, 30));
    assert!(count(&a, footer, Spectra6::White) > 50);
}

#[test]
fn minimal_sample_centres_text() {
    let mut f = frame();
    render(&spec(MINIMAL), &mut *f).unwrap();
    assert_only_displayable(f.as_bytes());
    let full = Rectangle::new(Point::zero(), Size::new(800, 480));
    let ink = count(&f, full, Spectra6::Black);
    assert!(ink > 500 && ink < 800 * 480 / 4, "ink pixels {ink}");
    // Centred: ink in the middle band, none in the top or bottom quarters.
    let top = Rectangle::new(Point::zero(), Size::new(800, 120));
    let bottom = Rectangle::new(Point::new(0, 360), Size::new(800, 120));
    let mid = Rectangle::new(Point::new(0, 200), Size::new(800, 80));
    assert_eq!(count(&f, top, Spectra6::Black), 0);
    assert_eq!(count(&f, bottom, Spectra6::Black), 0);
    assert!(count(&f, mid, Spectra6::Black) > 0);
    // And horizontally: nothing in the outer 10% either side.
    let left = Rectangle::new(Point::zero(), Size::new(80, 480));
    let right = Rectangle::new(Point::new(720, 0), Size::new(80, 480));
    assert_eq!(count(&f, left, Spectra6::Black), 0);
    assert_eq!(count(&f, right, Spectra6::Black), 0);
}

#[test]
fn text_wraps_and_is_clipped_to_its_region() {
    let long = "word ".repeat(90); // 450 bytes, under MAX_TEXT
    let json = format!(
        r#"{{"version":1,"regions":[{{"rect":[100,100,200,50],"text":"{long}","style":"body","background":"yellow"}}]}}"#
    );
    let mut f = frame();
    render(&spec(&json), &mut *f).unwrap();
    let region = Rectangle::new(Point::new(100, 100), Size::new(200, 50));
    let outside_ink = Rectangle::new(Point::zero(), Size::new(800, 480))
        .points()
        .filter(|p| !region.contains(*p))
        .filter(|p| f.get_pixel(p.x as u32, p.y as u32) != Some(Spectra6::White))
        .count();
    assert_eq!(outside_ink, 0, "nothing drawn outside the region");
    assert!(
        count(&f, region, Spectra6::Black) > 50,
        "some text drawn inside"
    );
    assert!(
        count(&f, region, Spectra6::Yellow) > 1000,
        "background filled"
    );
}

#[test]
fn off_screen_and_degenerate_regions_are_harmless() {
    let json = r#"{"version":1,"background":"green","regions":[
        {"rect":[-100,-100,150,150],"background":"red","border":{"color":"black","width":4},"text":"x"},
        {"rect":[750,450,500,500],"background":"blue","text":"clipped bottom right"},
        {"rect":[10,10,0,0],"background":"black","text":"nothing"},
        {"rect":[10,10,-5,-5],"background":"black","text":"negative"},
        {"rect":[10,10,4,4],"border":{"color":"black","width":10},"text":"too small for text"},
        {"rect":[2000,2000,10,10],"background":"black","text":"way off"}
    ]}"#;
    let mut f = frame();
    render(&spec(json), &mut *f).unwrap();
    assert_only_displayable(f.as_bytes());
    assert_eq!(f.get_pixel(0, 0), Some(Spectra6::Red));
    assert_eq!(
        f.get_pixel(49, 49),
        Some(Spectra6::Black),
        "border inside edge"
    );
    assert_eq!(f.get_pixel(50, 50), Some(Spectra6::Green));
    assert_eq!(f.get_pixel(799, 479), Some(Spectra6::Blue));
    assert_eq!(
        f.get_pixel(10, 10),
        Some(Spectra6::Black),
        "border of tiny region"
    );
    assert_eq!(
        f.get_pixel(13, 13),
        Some(Spectra6::Black),
        "oversized border fills it"
    );
    assert_eq!(
        f.get_pixel(15, 15),
        Some(Spectra6::Red),
        "back inside the red region"
    );
    assert_eq!(f.get_pixel(400, 240), Some(Spectra6::Green));
}

#[test]
fn error_screen_renders_and_truncates() {
    let mut f = frame();
    let detail = "é".repeat(2000);
    render_error(ErrorKind::FetchFailed, &detail, &mut *f).unwrap();
    assert_only_displayable(f.as_bytes());
    let banner = Rectangle::new(Point::zero(), Size::new(800, 96));
    assert!(count(&f, banner, Spectra6::Red) > 800 * 96 / 2);
    assert!(count(&f, banner, Spectra6::White) > 100);
    let body = Rectangle::new(Point::new(24, 120), Size::new(752, 336));
    assert!(count(&f, body, Spectra6::Black) > 100);

    for kind in [ErrorKind::InvalidSpec, ErrorKind::UnsupportedVersion] {
        render_error(kind, "", &mut *f).unwrap();
        assert_only_displayable(f.as_bytes());
    }
}

#[test]
fn palette_expansion() {
    assert_eq!(rgb(Spectra6::White), [245, 245, 240]);
    assert_eq!(RGB_BY_CODE[4], [255, 0, 255], "undisplayable code is loud");
    let src = [Spectra6::pack(Spectra6::Black, Spectra6::Red), 0x11];
    let mut dst = [0u8; 16];
    assert_eq!(expand_rgba(&src, &mut dst), 16);
    assert_eq!(&dst[0..4], &[0, 0, 0, 255]);
    assert_eq!(&dst[4..8], &[196, 50, 58, 255]);
    assert_eq!(&dst[8..12], &[245, 245, 240, 255]);
    // Short destination: only whole pixel pairs are written.
    let mut short = [0u8; 9];
    assert_eq!(expand_rgba(&src, &mut short), 8);
}

/// `cargo test -p render -- --ignored dump_ppm` writes the samples as PPM
/// files under the target dir for eyeballing.
#[test]
#[ignore]
fn dump_ppm() {
    for (name, json) in [("kitchen", KITCHEN), ("minimal", MINIMAL)] {
        let mut f = frame();
        render(&spec(json), &mut *f).unwrap();
        let mut rgba = vec![0u8; FRAME_BYTES * 8];
        expand_rgba(f.as_bytes(), &mut rgba);
        let mut ppm = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        for px in rgba.chunks_exact(4) {
            ppm.extend_from_slice(&px[..3]);
        }
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.ppm"));
        std::fs::write(&path, ppm).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
