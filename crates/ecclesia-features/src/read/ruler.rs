//! The reading ruler (plan R.20), the model: how the band looks, how tall it is around a line, and which line a
//! keyboard step moves it to. A port of SumatraPDF Enhanced's reading bar (`src/ReadingBar.cpp`): the same default
//! colour, opacity and height, and its invert mode, which dims the page outside the band.
//!
//! Unlike Enhanced's free band, this one snaps to the text lines the text layer (K.13) reads, and steps through them in
//! MuPDF's reading order, so in a two-column paper it reads one column to its end before the next.

use ecclesia_text::TextLayout;

/// The least band height a setting may ask for, in pixels at 96 dpi (Enhanced's `kMinHeight`).
pub const MIN_HEIGHT: f32 = 16.0;
/// The greatest band height a setting may ask for, in pixels at 96 dpi.
pub const MAX_HEIGHT: f32 = 400.0;

/// How the ruler looks: the settings `reading_ruler_height`, `reading_ruler_color`, `reading_ruler_opacity` and
/// `reading_ruler_invert` (Enhanced's `ReadingBar.Height`, `ReadingBar.Background`, `ReadingBar.Invert`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The band's least height, in pixels at 96 dpi; it grows to hold a taller line.
    pub height: f32,
    /// Its colour, `0xRRGGBB`.
    pub color: u32,
    /// How strongly the colour tints the page, from 0 (not at all) to 1 (the colour itself).
    pub opacity: f32,
    /// Whether the page outside the band is dimmed, instead of the band tinted.
    pub invert: bool,
}

impl Default for Style {
    /// Enhanced's: a warm yellow (`#ffe082`) at `0x66` of full strength, 48 pixels tall, tinting the band.
    fn default() -> Self {
        Self {
            height: 48.0,
            color: 0xffe082,
            opacity: f32::from(0x66_u8) / 255.0,
            invert: false,
        }
    }
}

impl Style {
    /// The style with every value in range: a hand-edited settings file may hold anything. A value that isn't a
    /// number falls back to its default.
    pub fn clamped(self) -> Self {
        let default = Self::default();
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        Self {
            height: finite(self.height, default.height).clamp(MIN_HEIGHT, MAX_HEIGHT),
            color: self.color & 0xff_ffff,
            opacity: finite(self.opacity, default.opacity).clamp(0.0, 1.0),
            invert: self.invert,
        }
    }

    /// The colour to draw the band in, darkest-wins over the page: the colour mixed with white by the opacity. On
    /// white paper it looks the same as the colour laid over at that opacity, and black text under it stays black.
    pub fn tint(&self) -> u32 {
        let mix = |shift: u32| {
            let channel = ((self.color >> shift) & 0xff) as f32;
            let mixed = 255.0 - (255.0 - channel) * self.opacity;
            (mixed.round() as u32).min(255) << shift
        };
        mix(16) | mix(8) | mix(0)
    }
}

/// Which way a keyboard step goes, in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// To the next line (the Down arrow).
    Next,
    /// To the line before (the Up arrow).
    Previous,
}

/// The band's top and bottom around a line from `top` to `bottom`: centred on it, `height` tall, or taller when the
/// line and `pad` above and below it need more. All in device pixels.
pub fn band(top: f32, bottom: f32, height: f32, pad: f32) -> (f32, f32) {
    let centre = (top + bottom) / 2.0;
    let half = height.max(bottom - top + 2.0 * pad) / 2.0;
    (centre - half, centre + half)
}

/// The line a step moves to from line `from` of a page's `layout`, as an index into [`TextLayout::lines`]. It goes in
/// MuPDF's reading order, skipping lines with nothing visible on them and the other pieces of the printed line it
/// leaves (MuPDF splits a line at a wide gap). `None` when the step leaves the page.
pub fn step(layout: &TextLayout, from: usize, step: Step) -> Option<usize> {
    let row = layout.lines.get(from)?;
    let mut candidates: Box<dyn Iterator<Item = usize>> = match step {
        Step::Next => Box::new(from + 1..layout.lines.len()),
        Step::Previous => Box::new((0..from).rev()),
    };
    candidates.find(|&i| {
        let line = &layout.lines[i];
        is_visible(layout, i) && !(line.block == row.block && same_row(&line.bbox, &row.bbox))
    })
}

/// The line a step reaches on entering a page: its first visible line going forward, its last going back.
pub fn entry(layout: &TextLayout, step: Step) -> Option<usize> {
    let mut lines = 0..layout.lines.len();
    match step {
        Step::Next => lines.find(|&i| is_visible(layout, i)),
        Step::Previous => lines.rfind(|&i| is_visible(layout, i)),
    }
}

/// Whether line `i` has a character that shows: not only spaces.
fn is_visible(layout: &TextLayout, i: usize) -> bool {
    layout.lines[i]
        .chars
        .clone()
        .any(|c| layout.chars.get(c).is_some_and(|c| !c.c.is_whitespace()))
}

/// Whether two line boxes are pieces of one printed line: they overlap by more than half of the shorter one's height.
fn same_row(a: &ecclesia_mupdf::PageRect, b: &ecclesia_mupdf::PageRect) -> bool {
    let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    let shorter = (a.y1 - a.y0).min(b.y1 - b.y0);
    shorter > 0.0 && overlap > shorter / 2.0
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::{PagePoint, PageRect, Quad, TextBlock, TextChar, TextLine};

    use super::*;

    /// A page of blocks, each a column at `x` of lines (text, top): characters 5 pt wide and 10 pt tall.
    fn page(blocks: &[(f32, &[(&str, f32)])]) -> TextLayout {
        let mut layout = TextLayout::default();
        for (b, (x, lines)) in blocks.iter().enumerate() {
            let first_line = layout.lines.len();
            for &(text, y) in *lines {
                let first_char = layout.chars.len();
                for (i, c) in text.chars().enumerate() {
                    let x0 = x + i as f32 * 5.0;
                    let at = |x, y| PagePoint { x, y };
                    layout.chars.push(TextChar {
                        c,
                        quad: Quad {
                            ul: at(x0, y),
                            ur: at(x0 + 5.0, y),
                            ll: at(x0, y + 10.0),
                            lr: at(x0 + 5.0, y + 10.0),
                        },
                        size: 10.0,
                    });
                }
                layout.lines.push(TextLine {
                    bbox: PageRect {
                        x0: *x,
                        y0: y,
                        x1: x + text.chars().count() as f32 * 5.0,
                        y1: y + 10.0,
                    },
                    dir: PagePoint { x: 1.0, y: 0.0 },
                    vertical: false,
                    joined: false,
                    block: b,
                    chars: first_char..layout.chars.len(),
                });
            }
            layout.blocks.push(TextBlock {
                bbox: PageRect::default(),
                lines: first_line..layout.lines.len(),
            });
        }
        layout
    }

    /// Every line a run of steps visits from the page's entry, in order.
    fn walk(layout: &TextLayout, direction: Step) -> Vec<usize> {
        let mut visited = Vec::new();
        let mut at = entry(layout, direction);
        while let Some(line) = at {
            visited.push(line);
            at = step(layout, line, direction);
        }
        visited
    }

    #[test]
    fn arrow_keys_move_the_ruler_exactly_one_text_line_in_two_columns() {
        // Two columns side by side; MuPDF reads the left one to its end, then the right one.
        let layout = page(&[
            (
                0.0,
                &[("left one", 0.0), ("left two", 14.0), ("left three", 28.0)],
            ),
            (300.0, &[("right one", 0.0), ("right two", 14.0)]),
        ]);
        assert_eq!(walk(&layout, Step::Next), [0, 1, 2, 3, 4]);
        assert_eq!(walk(&layout, Step::Previous), [4, 3, 2, 1, 0]);
    }

    #[test]
    fn blank_lines_and_pieces_of_one_printed_line_are_one_step() {
        // A line MuPDF split at a wide gap, then a line of spaces, then the next line.
        let layout = page(&[(
            0.0,
            &[("Name", 0.0), ("Page", 0.5), ("   ", 14.0), ("next", 28.0)],
        )]);
        assert_eq!(step(&layout, 0, Step::Next), Some(3));
        assert_eq!(step(&layout, 1, Step::Next), Some(3));
        assert_eq!(step(&layout, 3, Step::Previous), Some(1));
        assert_eq!(step(&layout, 1, Step::Previous), None);
    }

    #[test]
    fn a_page_without_text_has_no_entry() {
        assert_eq!(entry(&TextLayout::default(), Step::Next), None);
        assert_eq!(step(&TextLayout::default(), 0, Step::Next), None);
    }

    #[test]
    fn the_band_holds_its_line_and_is_centred_on_it() {
        assert_eq!(band(100.0, 120.0, 48.0, 4.0), (86.0, 134.0));
        // A line taller than the band, at a high zoom: the band grows.
        assert_eq!(band(100.0, 160.0, 48.0, 4.0), (96.0, 164.0));
    }

    #[test]
    fn the_tint_on_white_paper_matches_the_colour_at_its_opacity() {
        let style = Style::default();
        // #ffe082 at 0x66: red stays, green 255 - 31 * 0.4, blue 255 - 125 * 0.4.
        assert_eq!(style.tint(), 0xfff3cd);
        let opaque = Style {
            opacity: 1.0,
            ..style
        };
        assert_eq!(opaque.tint(), 0xffe082);
    }

    #[test]
    fn hand_edited_settings_are_brought_into_range() {
        let wild = Style {
            height: f32::NAN,
            color: 0xff12_3456,
            opacity: 7.0,
            invert: true,
        }
        .clamped();
        assert_eq!(wild.height, Style::default().height);
        assert_eq!(wild.color, 0x12_3456);
        assert_eq!(wild.opacity, 1.0);
        assert_eq!(
            Style {
                height: 2.0,
                ..Style::default()
            }
            .clamped()
            .height,
            MIN_HEIGHT
        );
    }
}
