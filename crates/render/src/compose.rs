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

/// Where the status bar goes when the spec does not mark one.
const DEFAULT_STATUS_RECT: [i32; 4] = [0, crate::HEIGHT as i32 - 30, crate::WIDTH as i32, 30];

/// Draws `spec` onto `target`: clears to the background colour, then draws
/// each region in order (fill, border, text), each clipped to its rectangle.
///
/// The only way this fails is if the target's own drawing fails; the frame
/// types in this crate are infallible.
pub fn render<D>(spec: &ScreenSpec, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Spectra6>,
{
    render_with_alert(spec, None, target)
}

/// Like [`render`], but with `alert` set the status bar (the first region
/// marked `"status": true`) is drawn red with `alert` as white,
/// right-aligned text in place of whatever the spec put there. A spec with
/// no marked region gets a bar across the bottom of the screen instead.
///
/// This is how the device reports what only it knows (battery low, no
/// usable screen from the server) without a separate error screen.
pub fn render_with_alert<D>(
    spec: &ScreenSpec,
    alert: Option<&str>,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Spectra6>,
{
    target.clear(spec.background.into())?;
    let mut alerted = false;
    for region in &spec.regions {
        match alert {
            Some(msg) if region.status && !alerted => {
                draw_region(&alert_region(region.clone(), msg), target)?;
                alerted = true;
            }
            _ => draw_region(region, target)?,
        }
    }
    if let Some(msg) = alert {
        if !alerted {
            let mut bar = Region::new(DEFAULT_STATUS_RECT);
            bar.style = TextStyle::Small;
            draw_region(&alert_region(bar, msg), target)?;
        }
    }
    Ok(())
}

/// `region` restyled as the red alert bar showing `msg` (truncated to fit).
fn alert_region(mut region: Region, msg: &str) -> Region {
    region.background = Some(Colour::Red);
    region.color = Colour::White;
    region.align = HAlign::Right;
    region.valign = VAlign::Middle;
    region.border = None;
    region.text.clear();
    for ch in msg.chars() {
        if region.text.push(ch).is_err() {
            break; // truncated to MAX_TEXT
        }
    }
    region
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
