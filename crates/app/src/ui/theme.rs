//! Visual tokens as gpui types: the one place a colour or a size is written
//! down.
//!
//! Colors live in [`Theme`], which comes in a light and a dark set picked from
//! the window's appearance; sizes and the type scale are appearance-independent
//! consts. Everything the views need should come from here — no literal colors
//! or magic numbers in `popover.rs`.
//!
//! The popover is monochrome, the way the system Battery menu is: ink on a
//! near-white material. Colour never carries meaning in scorebar. Who is ahead
//! in a matchup is said with [`Theme::text`] against [`Theme::secondary`] —
//! ink for the leader, grey for the trailer — because a scoreboard that
//! coloured the leader green would be unreadable to a chunk of its audience
//! and would still need the ink contrast to be legible at menu bar size.
//! There is no accent and no warning colour either: a failure the popover has
//! to report, it reports in words.

use gpui::{px, Pixels, Rgba, WindowAppearance};

/// `const`-friendly hex -> [`Rgba`] (gpui's own `rgb()` is not `const`).
const fn hex(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.0,
        g: ((value >> 8) & 0xff) as f32 / 255.0,
        b: (value & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Same, with an explicit alpha in 0.0..=1.0.
const fn hex_a(value: u32, alpha: f32) -> Rgba {
    let c = hex(value);
    Rgba { a: alpha, ..c }
}

// ── Colors ───────────────────────────────────────────────────────────────────

/// The appearance-dependent half of the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Popover background — the material the rows sit on.
    pub bg: Rgba,
    /// Hairline border around the popover.
    pub border: Rgba,
    /// Primary text, and — the popover has no accent — the fill of every
    /// meter. Also the ink the leading score is set in.
    pub text: Rgba,
    /// Secondary text: the trailing score, projections, captions.
    pub secondary: Rgba,
    /// Tertiary text: section labels and the notes under a settings row.
    pub tertiary: Rgba,
    /// Separator rules, and — the same value — the track a meter is drawn in.
    pub separator: Rgba,
    /// Hover wash on a menu row.
    pub hover: Rgba,
}

/// Light appearance.
pub const LIGHT: Theme = Theme {
    // Sampled off the Battery menu on a live screen: its material lands at
    // about (238, 237, 239) over a light page, which this grey at
    // `BG_ALPHA` reproduces. Plain white reads as a card, not a menu.
    bg: Rgba {
        r: 236.0 / 255.0,
        g: 236.0 / 255.0,
        b: 238.0 / 255.0,
        a: BG_ALPHA,
    },
    // The system menu has no visible hairline of its own, only the shadow's
    // edge; a faint rim is all that keeps the corners crisp against a busy
    // desktop.
    border: hex_a(0x000000, 0.06),
    // System label colour: black at 85%, so text sits in the material rather
    // than on top of it.
    text: hex_a(0x000000, 0.85),
    secondary: hex(0x6e6e73),
    tertiary: hex(0xaeaeb2),
    separator: hex_a(0x000000, 0.09),
    hover: hex_a(0x000000, 0.06),
};

/// Dark appearance: the same roles against a dark material.
pub const DARK: Theme = Theme {
    bg: Rgba {
        r: 40.0 / 255.0,
        g: 40.0 / 255.0,
        b: 42.0 / 255.0,
        a: BG_ALPHA,
    },
    border: hex_a(0xffffff, 0.10),
    text: hex_a(0xffffff, 0.85),
    secondary: hex(0x98989d),
    tertiary: hex(0x8e8e93),
    separator: hex_a(0xffffff, 0.12),
    hover: hex_a(0xffffff, 0.10),
};

/// How opaque the popover material is. Over a
/// [`Blurred`](gpui::WindowBackgroundAppearance::Blurred) window this is what
/// gives the popover the translucent look of a system menu: what is behind it
/// shows through as a soft wash, and the text stays fully legible. Checked on
/// a live screen against the Battery menu; `1.0` is the opaque fallback.
pub const BG_ALPHA: f32 = 0.85;

impl Default for Theme {
    fn default() -> Self {
        LIGHT
    }
}

impl Theme {
    /// Pick the set matching the window's appearance.
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => DARK,
            WindowAppearance::Light | WindowAppearance::VibrantLight => LIGHT,
        }
    }

    /// The ink a score is set in: full strength for the side that is ahead,
    /// secondary for the side that is behind.
    ///
    /// This is the whole of how scorebar says "who is winning". A tie is not
    /// ahead, so both sides fall back to secondary and neither one is pointed
    /// at — which is the truthful drawing of a tie.
    pub fn score_ink(&self, leading: bool) -> Rgba {
        if leading {
            self.text
        } else {
            self.secondary
        }
    }

    /// The fill of a meter. Always ink: a meter shows a proportion, and the
    /// proportion is the message.
    pub fn meter_fill(&self) -> Rgba {
        self.text
    }
}

// ── Sizes ────────────────────────────────────────────────────────────────────

/// Popover width. Fixed; height grows with content.
pub const POPOVER_WIDTH: Pixels = px(260.);
/// Corner radius of the popover.
pub const POPOVER_RADIUS: Pixels = px(10.);
/// Gap between the menu bar and the top of the popover. Zero: the system
/// menus hang straight off the bar's bottom edge, and a gap reads as a
/// floating window rather than a menu.
pub const POPOVER_TOP_GAP: Pixels = px(0.);
/// The menu inset: the padding between the popover's edge and its rows, as a
/// plain float so the layout constants that add it up stay `const`.
pub const POPOVER_PAD_PX: f32 = 5.0;
/// The same, as gpui's unit.
pub const POPOVER_PAD: Pixels = px(POPOVER_PAD_PX);

/// Horizontal padding inside a row — the text inset every line shares.
pub const ROW_PAD_X: Pixels = px(10.);
/// Vertical padding inside a row.
pub const ROW_PAD_Y_PX: f32 = 3.0;
pub const ROW_PAD_Y: Pixels = px(ROW_PAD_Y_PX);
/// Corner radius of a row's hover wash.
pub const ROW_RADIUS: Pixels = px(6.);
/// How far a settings row is indented under its disclosure row.
pub const ROW_INDENT: Pixels = px(12.);

/// A separator is inset from the popover's edges the way a menu's is.
pub const SEPARATOR_INSET: Pixels = px(10.);
/// The air above and below a separator.
pub const SEPARATOR_MARGIN_PX: f32 = 5.0;
pub const SEPARATOR_MARGIN: Pixels = px(SEPARATOR_MARGIN_PX);
/// A hairline.
pub const HAIRLINE_PX: f32 = 1.0;
pub const HAIRLINE: Pixels = px(HAIRLINE_PX);

/// Height of the bar under a matchup that shows how the two scores divide.
pub const METER_HEIGHT_PX: f32 = 4.0;
pub const METER_HEIGHT: Pixels = px(METER_HEIGHT_PX);
/// Corner radius of a meter.
pub const METER_RADIUS: Pixels = px(2.);
/// The gap between a matchup's label row, its meter and its caption.
pub const METER_GAP_PX: f32 = 4.0;
pub const METER_GAP: Pixels = px(METER_GAP_PX);
/// The gap between one league's block and the next. Wider than the gaps
/// inside a block, so a popover holding four leagues reads as four things.
pub const BLOCK_GAP_PX: f32 = 9.0;
pub const BLOCK_GAP: Pixels = px(BLOCK_GAP_PX);

// ── Type scale ───────────────────────────────────────────────────────────────

/// Section headers (a league's name) and every menu row.
pub const TEXT_TITLE: Pixels = px(13.);
/// Body text — the same size. One type size carries the whole popover; weight
/// is what separates a header from a row.
pub const TEXT_BODY: Pixels = px(13.);
/// Captions: the projection line under a matchup, the notice line.
pub const TEXT_TINY: Pixels = px(11.);
/// Section labels and the notes under a settings row.
pub const TEXT_MICRO: Pixels = px(10.);

/// Line box of a 13px row. gpui does not lay text out to a round number, so
/// every row states its line height and the height arithmetic uses these.
pub const LINE_TITLE_PX: f32 = 17.0;
pub const LINE_TITLE: Pixels = px(LINE_TITLE_PX);
/// Line box of an 11px caption.
pub const LINE_TINY_PX: f32 = 14.0;
pub const LINE_TINY: Pixels = px(LINE_TINY_PX);
/// Line box of a 10px label.
pub const LINE_MICRO_PX: f32 = 13.0;
pub const LINE_MICRO: Pixels = px(LINE_MICRO_PX);

/// Monospace family, used for the tabular numbers a score column needs: the
/// scores move every refresh and proportional digits make the column jitter.
pub const MONO_FAMILY: &str = "SF Mono";
/// UI family.
pub const UI_FAMILY: &str = ".SystemUIFont";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_background_is_the_system_menu_material() {
        assert_eq!((LIGHT.bg.r * 255.0).round() as u32, 236);
        assert_eq!((DARK.bg.b * 255.0).round() as u32, 42);
        // Translucent over a blurred window: see BG_ALPHA.
        assert!((LIGHT.bg.a - BG_ALPHA).abs() < 1e-6);
        assert!((DARK.bg.a - BG_ALPHA).abs() < 1e-6);
    }

    #[test]
    fn appearance_picks_the_matching_set() {
        assert_eq!(Theme::for_appearance(WindowAppearance::Light), LIGHT);
        assert_eq!(Theme::for_appearance(WindowAppearance::VibrantLight), LIGHT);
        assert_eq!(Theme::for_appearance(WindowAppearance::Dark), DARK);
        assert_eq!(Theme::for_appearance(WindowAppearance::VibrantDark), DARK);
    }

    #[test]
    fn the_popover_is_260_wide() {
        assert_eq!(POPOVER_WIDTH, px(260.));
        assert_eq!(POPOVER_RADIUS, px(10.));
        assert_eq!(POPOVER_PAD, px(5.));
    }

    #[test]
    fn rows_and_separators_keep_the_menu_inset() {
        assert_eq!(ROW_PAD_X, px(10.));
        assert_eq!(ROW_PAD_Y, px(3.));
        assert_eq!(ROW_RADIUS, px(6.));
        assert_eq!(SEPARATOR_INSET, px(10.));
        assert_eq!(SEPARATOR_MARGIN, px(5.));
    }

    #[test]
    fn meters_are_four_pixels_and_blocks_are_nine_apart() {
        assert_eq!(METER_HEIGHT, px(4.));
        assert_eq!(METER_RADIUS, px(2.));
        assert_eq!(BLOCK_GAP, px(9.));
        // A league block is further from its neighbour than a meter is from
        // its own caption, or the popover reads as one long list.
        const { assert!(BLOCK_GAP_PX > METER_GAP_PX) };
    }

    /// One size for everything readable, two smaller ones for captions.
    #[test]
    fn the_type_scale_is_thirteen_eleven_ten() {
        assert_eq!(TEXT_TITLE, px(13.));
        assert_eq!(TEXT_BODY, TEXT_TITLE);
        assert_eq!(TEXT_TINY, px(11.));
        assert_eq!(TEXT_MICRO, px(10.));
        assert_eq!(LINE_TITLE, px(17.));
        assert_eq!(LINE_TINY, px(14.));
        assert_eq!(LINE_MICRO, px(13.));
    }

    /// Who is ahead is ink versus grey, in both appearances, and never a
    /// colour.
    #[test]
    fn the_leading_score_is_ink_and_the_trailing_one_is_grey() {
        assert_eq!(LIGHT.score_ink(true), LIGHT.text);
        assert_eq!(LIGHT.score_ink(false), LIGHT.secondary);
        assert_eq!(DARK.score_ink(true), DARK.text);
        assert_eq!(DARK.score_ink(false), DARK.secondary);
    }

    /// A meter is a proportion, not a verdict, so it is always ink.
    #[test]
    fn a_meter_is_always_ink() {
        assert_eq!(LIGHT.meter_fill(), LIGHT.text);
        assert_eq!(DARK.meter_fill(), DARK.text);
    }
}
