//! Turns a [`ScreenSpec`] into drawing calls.

use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle, StrokeAlignment};
use embedded_graphics::text::LineHeight;
use embedded_text::alignment::{HorizontalAlignment, VerticalAlignment};
use embedded_text::style::{HeightMode, TextBoxStyleBuilder, VerticalOverdraw};
use embedded_text::TextBox;
use screen_spec::{Colour, HAlign, Region, ScreenSpec, TextStyle, VAlign};

use crate::fonts::{self, PADDING};
use crate::Spectra6;

/// Draws `spec` onto `target`: clears to the background colour, then draws
/// each region in order (fill, border, text), each clipped to its rectangle.
///
/// The only way this fails is if the target's own drawing fails; the frame
/// types in this crate are infallible.
pub fn render<D>(spec: &ScreenSpec, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Spectra6>,
{
    target.clear(spec.background.into())?;
    for region in &spec.regions {
        draw_region(region, target)?;
    }
    Ok(())
}

fn draw_region<D>(region: &Region, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Spectra6>,
{
    let rect = Rectangle::new(
        Point::new(region.rect[0], region.rect[1]),
        Size::new(region.width(), region.height()),
    );
    if rect.is_zero_sized() {
        return Ok(());
    }
    // `clipped` keeps absolute coordinates; it only limits what gets drawn.
    let mut clipped = target.clipped(&rect);

    if let Some(bg) = region.background {
        clipped.fill_solid(&rect, bg.into())?;
    }

    let mut inset = PADDING;
    if let Some(border) = region.border {
        let width = u32::from(border.width);
        if width > 0 {
            rect.into_styled(
                PrimitiveStyleBuilder::new()
                    .stroke_color(border.color.into())
                    .stroke_width(width)
                    .stroke_alignment(StrokeAlignment::Inside)
                    .build(),
            )
            .draw(&mut clipped)?;
            inset += width;
        }
    }

    if region.text.is_empty() {
        return Ok(());
    }
    let text_bounds = shrink(rect, inset);
    if text_bounds.is_zero_sized() {
        return Ok(());
    }

    let character_style = fonts::text_style(region.style, region.color.into());
    let textbox_style = TextBoxStyleBuilder::new()
        .alignment(match region.align {
            HAlign::Left => HorizontalAlignment::Left,
            HAlign::Center => HorizontalAlignment::Center,
            HAlign::Right => HorizontalAlignment::Right,
        })
        .vertical_alignment(match region.valign {
            VAlign::Top => VerticalAlignment::Top,
            VAlign::Middle => VerticalAlignment::Middle,
            VAlign::Bottom => VerticalAlignment::Bottom,
        })
        // `Hidden` overflows (debug panic) in embedded-text 0.7.3 when a line
        // falls entirely below the box, and `FullRowsOnly` drops a line that
        // is even one pixel too tall. `Visible` draws everything and the
        // clipped target (the region's rect) does the actual bounding.
        .height_mode(HeightMode::Exact(VerticalOverdraw::Visible))
        .line_height(LineHeight::Percent(115))
        .build();

    TextBox::with_textbox_style(&region.text, text_bounds, character_style, textbox_style)
        .draw(&mut clipped)?;
    Ok(())
}

fn shrink(rect: Rectangle, by: u32) -> Rectangle {
    let by2 = by.saturating_mul(2);
    if rect.size.width <= by2 || rect.size.height <= by2 {
        return Rectangle::zero();
    }
    Rectangle::new(
        rect.top_left + Point::new(by as i32, by as i32),
        Size::new(rect.size.width - by2, rect.size.height - by2),
    )
}

/// What went wrong, for [`render_error`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The document could not be parsed.
    InvalidSpec,
    /// The document's `version` is not supported by this build.
    UnsupportedVersion,
    /// The device could not reach the server or the request failed.
    FetchFailed,
}

impl ErrorKind {
    fn title(self) -> &'static str {
        match self {
            ErrorKind::InvalidSpec => "Invalid screen spec",
            ErrorKind::UnsupportedVersion => "Unsupported spec version",
            ErrorKind::FetchFailed => "Could not fetch screen",
        }
    }
}

/// Draws a full-screen error notice, so the device and the preview show the
/// same thing when a document is unusable. `detail` is truncated to fit.
pub fn render_error<D>(kind: ErrorKind, detail: &str, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Spectra6>,
{
    let mut spec = ScreenSpec::empty();

    let mut banner = Region::new([0, 0, crate::WIDTH as i32, 96]);
    let _ = banner.text.push_str(kind.title());
    banner.style = TextStyle::Header;
    banner.align = HAlign::Center;
    banner.valign = VAlign::Middle;
    banner.color = Colour::White;
    banner.background = Some(Colour::Red);

    let mut body = Region::new([
        24,
        120,
        crate::WIDTH as i32 - 48,
        crate::HEIGHT as i32 - 144,
    ]);
    for ch in detail.chars() {
        if body.text.push(ch).is_err() {
            break; // truncated to MAX_TEXT
        }
    }
    body.style = TextStyle::Body;

    let _ = spec.regions.push(banner);
    let _ = spec.regions.push(body);
    render(&spec, target)
}
