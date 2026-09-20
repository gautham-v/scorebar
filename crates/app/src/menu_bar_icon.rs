//! The menu bar item's picture: a small scoreboard glyph, optionally followed
//! by a line of text, drawn at runtime with Core Graphics rather than shipped
//! as an asset.
//!
//! The whole item is one image, text included. A status item button has room
//! for one title and one image, and a title picks up AppKit's 13pt control
//! font — visibly larger than the battery percentage next door. Drawing the
//! text into the image instead puts the point size and the gap between the
//! glyph and the numbers under our own control rather than a space
//! character's width.
//!
//! The image is always marked `isTemplate`, so AppKit throws away the colours
//! and keeps only the alpha; it then tints the glyph and the text for the
//! current menu bar appearance — dark, light, and the "reduce transparency"
//! variants — for free. scorebar can do this unconditionally because colour
//! never carries meaning here: nothing in the item ever has to *be* a
//! particular colour, so nothing is ever drawn outside the template.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage,
};
use objc2_core_graphics::{CGContext, CGLineCap, CGLineJoin};
use objc2_foundation::{NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize};

/// Height of the item's box, in points. Exactly the optical height of the
/// system glyphs beside it. Measured against the battery outline and the
/// Control Center switches on a live menu bar rather than guessed: 13pt, which
/// is what the human interface guidelines suggest for a template image, came
/// out visibly smaller than everything around it.
pub const SIZE: f64 = 15.0;

/// Width of the scoreboard glyph, in points. A scoreboard is landscape — the
/// real thing is two numbers side by side — so unlike a ring the glyph is
/// wider than it is tall.
pub const GLYPH_WIDTH: f64 = 17.0;

/// Height of the scoreboard glyph. Shorter than [`SIZE`]: a hollow rectangle
/// drawn to the full 13pt reads heavier than the battery outline next to it,
/// because a rectangle fills its box where a rounded glyph does not.
pub const GLYPH_HEIGHT: f64 = 12.0;

/// Stroke weight of the glyph's outline and its divider, in points. Matched to
/// the battery's outline, which is lighter than a ring of the same size would
/// want: a rectangle has four long straight edges, and a weight that reads as
/// thin on a circle reads as heavy here.
pub const STROKE: f64 = 1.4;

/// Corner radius of the scoreboard, before [`corner_radius`] clamps it. Enough
/// to say "rounded" at 13pt without the corners eating the short edges.
pub const CORNER_RADIUS: f64 = 3.0;

/// Between the glyph and the text after it.
pub const LABEL_GAP: f64 = 4.0;

/// The point size of the text beside the glyph. The menu bar's own font is
/// 13pt, but the system's battery percentage is set smaller and this item sits
/// right beside it. 11pt lined the cap heights up on paper and read as small
/// on a real bar next to the battery percentage, so this is set by eye at the
/// size the system's own numbers appear to be.
pub const TEXT_POINT_SIZE: f64 = 12.5;

/// What a laid-out piece of the item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// The scoreboard glyph, always first.
    Glyph,
    /// The text after it — a score line, a record, or nothing.
    Label,
}

/// A piece of the item and how wide it is, before it has been given an x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub width: f64,
}

/// Lay the segments out left to right. Returns the item's width and each
/// segment's left edge. Pure, so the spacing rules are tested without AppKit.
pub fn place(segments: &[Segment]) -> (f64, Vec<f64>) {
    let mut x = 0.0;
    let mut lefts = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            x += LABEL_GAP;
        }
        lefts.push(x);
        x += segment.width;
    }
    (x, lefts)
}

/// The whole menu bar item as one [`NSImage`]: the scoreboard glyph, and after
/// it whatever the caller asked to print.
///
/// `label` is already formatted — the closest game's `"65.44 – 104.32"`, a
/// record like `"2–1"`, or `None` for the glyph on its own. Formatting is the
/// caller's job because it depends on which leagues the user is in, and this
/// module's job is the picture.
///
/// The glyph is drawn whether or not there is text, which is also what keeps
/// the item clickable: nothing drawn is no hit area, and a status item with no
/// hit area cannot open its popover.
pub fn item_image(label: Option<&str>) -> Retained<NSImage> {
    let text = label.map(text_run);

    let mut segments = vec![Segment {
        kind: SegmentKind::Glyph,
        width: GLYPH_WIDTH,
    }];
    if let Some(text) = &text {
        segments.push(Segment {
            kind: SegmentKind::Label,
            width: text.size().width,
        });
    }

    let (width, lefts) = place(&segments);
    let height = text
        .as_ref()
        .map(|text| text.size().height)
        .unwrap_or(SIZE)
        .max(SIZE);

    // Captured by the drawing block, which AppKit may run long after this
    // function returns and again on every appearance change.
    let glyph_x = lefts[0];
    let label_x = lefts.get(1).copied();

    let handler = RcBlock::new(move |_dirty: NSRect| -> Bool {
        draw_glyph(glyph_x, height / 2.0);
        if let (Some(text), Some(x)) = (&text, label_x) {
            // Vertically centred: `drawAtPoint` takes the bottom-left of the
            // run's own box in this bottom-left-origin context.
            let y = (height - text.size().height) / 2.0;
            text.drawAtPoint(NSPoint::new(x, y));
        }
        Bool::YES
    });

    let image = NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(width.max(GLYPH_WIDTH), height),
        false,
        &handler,
    );
    image.setTemplate(true);
    image
}

/// One run of text in the menu bar font.
///
/// Monospaced digits, not the proportional ones: a live score is redrawn every
/// refresh and proportional figures make the item change width — and so shove
/// every status item to its left — each time a `1` becomes a `7`.
///
/// Drawn in opaque black because the image is a template and AppKit keeps only
/// the alpha. `NSColor::labelColor` would be wrong even if it were not: inside
/// a drawing handler it resolves against the *app's* appearance, which is
/// light, and paints near-black onto a dark menu bar.
fn text_run(label: &str) -> Retained<NSAttributedString> {
    // Safety: `NSFontWeightRegular` is an AppKit-owned `CGFloat` constant,
    // initialised before any code of ours runs and never written to again.
    let weight = unsafe { NSFontWeightRegular };
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(TEXT_POINT_SIZE, weight);
    let colour = NSColor::blackColor();
    // Safety: `NSFontAttributeName` documents its value as an `NSFont` and
    // `NSForegroundColorAttributeName` as an `NSColor`, which is what we pass.
    unsafe {
        let attrs = NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font as &AnyObject, &*colour as &AnyObject],
        );
        NSAttributedString::new_with_attributes(
            &objc2_foundation::NSString::from_str(label),
            &attrs,
        )
    }
}

/// The glyph's outline, as the rectangle the stroke is *centred* on.
///
/// Inset by half the stroke on every side, so the outline's outer edge lands
/// exactly on the glyph box rather than half a point outside it. In the
/// image's own bottom-left-origin coordinates, vertically centred on `mid_y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRect {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
}

impl GlyphRect {
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    pub fn height(&self) -> f64 {
        self.top - self.bottom
    }
}

/// The stroke centreline of a glyph whose box starts at `x`.
pub fn glyph_rect(x: f64, mid_y: f64) -> GlyphRect {
    let inset = STROKE / 2.0;
    GlyphRect {
        left: x + inset,
        right: x + GLYPH_WIDTH - inset,
        bottom: mid_y - GLYPH_HEIGHT / 2.0 + inset,
        top: mid_y + GLYPH_HEIGHT / 2.0 - inset,
    }
}

/// Where the divider goes: down the middle, which is what makes the rectangle
/// read as a scoreboard rather than a window or a card.
pub fn divider_x(rect: &GlyphRect) -> f64 {
    (rect.left + rect.right) / 2.0
}

/// The corner radius actually drawn, clamped so it can never exceed half the
/// shorter side — Core Graphics draws a rounded corner larger than that as a
/// bulge, and the constant is a design choice that should not be able to
/// break the glyph if the box is ever made smaller.
pub fn corner_radius(rect: &GlyphRect) -> f64 {
    CORNER_RADIUS.min(rect.width().min(rect.height()) / 2.0)
}

/// Draw the scoreboard into whatever context AppKit has made current for the
/// handler, with its box starting at `x`.
fn draw_glyph(x: f64, mid_y: f64) {
    let Some(ctx) = NSGraphicsContext::currentContext() else {
        return;
    };
    let cg = ctx.CGContext();
    let cg = Some(&*cg);

    let rect = glyph_rect(x, mid_y);
    let radius = corner_radius(&rect);

    CGContext::set_should_antialias(cg, true);
    CGContext::set_line_width(cg, STROKE);
    // Butt caps and mitred joins: the divider meets the frame at right angles
    // and a round cap would leave it visibly short of the top and bottom
    // edges.
    CGContext::set_line_cap(cg, CGLineCap::Butt);
    CGContext::set_line_join(cg, CGLineJoin::Miter);
    NSColor::blackColor().setStroke();

    // The frame. Built from arc-to-point corners rather than a rounded-rect
    // path so the frame and the divider below are one begin_path/stroke_path
    // pair rather than two.
    CGContext::begin_path(cg);
    CGContext::move_to_point(cg, rect.left + radius, rect.bottom);
    CGContext::add_arc_to_point(cg, rect.right, rect.bottom, rect.right, rect.top, radius);
    CGContext::add_arc_to_point(cg, rect.right, rect.top, rect.left, rect.top, radius);
    CGContext::add_arc_to_point(cg, rect.left, rect.top, rect.left, rect.bottom, radius);
    CGContext::add_arc_to_point(cg, rect.left, rect.bottom, rect.right, rect.bottom, radius);
    CGContext::close_path(cg);

    // The divider, in the same path: two subpaths stroke in one pass.
    let mid_x = divider_x(&rect);
    CGContext::move_to_point(cg, mid_x, rect.bottom);
    CGContext::add_line_to_point(cg, mid_x, rect.top);

    CGContext::stroke_path(cg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(kind: SegmentKind, width: f64) -> Segment {
        Segment { kind, width }
    }

    /// The stroke's outer edge lands on the glyph box, not outside it: a
    /// status item image is clipped to its own size, and half a point of
    /// outline over the edge is a flat side.
    #[test]
    fn the_glyph_outline_sits_inside_its_box() {
        let rect = glyph_rect(0.0, SIZE / 2.0);
        assert!(rect.left - STROKE / 2.0 >= 0.0);
        assert!(rect.right + STROKE / 2.0 <= GLYPH_WIDTH);
        assert!(rect.bottom - STROKE / 2.0 >= (SIZE - GLYPH_HEIGHT) / 2.0);
        assert!(rect.top + STROKE / 2.0 <= SIZE - (SIZE - GLYPH_HEIGHT) / 2.0);
    }

    /// The glyph is shorter than the item box, so text beside it can be taller
    /// without the glyph growing to match.
    #[test]
    fn the_glyph_is_landscape_and_shorter_than_the_item() {
        const { assert!(GLYPH_WIDTH > GLYPH_HEIGHT) };
        const { assert!(GLYPH_HEIGHT < SIZE) };
    }

    /// A glyph drawn after something else starts where the layout put it, not
    /// at zero.
    #[test]
    fn the_glyph_follows_its_box() {
        let here = glyph_rect(0.0, 6.5);
        let there = glyph_rect(40.0, 6.5);
        assert_eq!(there.left - here.left, 40.0);
        assert_eq!(there.bottom, here.bottom);
    }

    #[test]
    fn the_divider_is_halfway_across() {
        let rect = glyph_rect(0.0, SIZE / 2.0);
        assert!((divider_x(&rect) - (rect.left + rect.width() / 2.0)).abs() < 1e-9);
    }

    /// The constant is a preference; the clamp is the guarantee.
    #[test]
    fn the_corner_radius_never_exceeds_half_the_short_side() {
        let rect = glyph_rect(0.0, SIZE / 2.0);
        assert!(corner_radius(&rect) <= rect.height() / 2.0);
        assert_eq!(corner_radius(&rect), CORNER_RADIUS);

        let squashed = GlyphRect {
            left: 0.0,
            right: 10.0,
            bottom: 0.0,
            top: 3.0,
        };
        assert_eq!(corner_radius(&squashed), 1.5);
    }

    /// The glyph on its own: no gap, and the item is exactly the glyph wide.
    #[test]
    fn a_glyph_only_item_is_one_glyph_wide() {
        let (width, lefts) = place(&[segment(SegmentKind::Glyph, GLYPH_WIDTH)]);
        assert_eq!(lefts, vec![0.0]);
        assert_eq!(width, GLYPH_WIDTH);
    }

    /// Glyph, a small gap, then the text.
    #[test]
    fn a_labelled_item_is_the_glyph_then_its_text() {
        let (width, lefts) = place(&[
            segment(SegmentKind::Glyph, GLYPH_WIDTH),
            segment(SegmentKind::Label, 62.0),
        ]);
        assert_eq!(lefts, vec![0.0, GLYPH_WIDTH + LABEL_GAP]);
        assert_eq!(width, GLYPH_WIDTH + LABEL_GAP + 62.0);
    }

    #[test]
    fn an_empty_item_has_no_width() {
        assert_eq!(place(&[]), (0.0, vec![]));
    }
}
