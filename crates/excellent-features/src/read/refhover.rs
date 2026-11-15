//! Citation and equation hover (plan R.25), the model: given a page's glyphs, find the citation under the pointer and
//! the slice of the target page worth showing in a popover.
//!
//! Ported from SumatraPDF Enhanced (`src/RefHoverDetect.cpp`, `src/RefHoverTextDetect.cpp`) with its tests
//! (`src/base/tests/RefHover_ut.cpp`). The heuristics work on one glyph per character with whole-point boxes in page
//! space (y down), as Enhanced's do, so they behave the same on the same pages.
//!
//! - [`cite`] finds a citation at a point: numeric (`[12]`, `[3-5]`) or author-year (`(Smith et al., 2020)`), and
//!   finds its entry in the references by plain text when the document has no link.
//! - [`layout`] decides what to show on the target page: one bibliography entry, a labelled equation, or a
//!   landscape strip below the target.
//! - [`resolve`] ties them together: the entry of a citation without a link, the region to show for a target, and the
//!   zoom that fits it in a popover.

use excellent_mupdf::TextLayout;

pub mod cite;
pub mod layout;
pub mod resolve;

#[cfg(test)]
mod tests;

/// A box in whole page points, y down, with SumatraPDF's semantics: it holds `x..x+dx` and `y..y+dy`, half-open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub dx: i32,
    /// Height.
    pub dy: i32,
}

impl Rect {
    /// A box from its left, top, width and height.
    pub const fn new(x: i32, y: i32, dx: i32, dy: i32) -> Self {
        Self { x, y, dx, dy }
    }

    /// Its right edge, outside it.
    pub const fn right(self) -> i32 {
        self.x + self.dx
    }

    /// Its bottom edge, outside it.
    pub const fn bottom(self) -> i32 {
        self.y + self.dy
    }

    /// Whether it has no area.
    pub const fn is_empty(self) -> bool {
        self.dx == 0 || self.dy == 0
    }

    /// Whether it holds the point.
    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The box grown by `ix` on the left and right and `iy` on the top and bottom.
    #[must_use]
    pub const fn inflate(self, ix: i32, iy: i32) -> Self {
        Self::new(self.x - ix, self.y - iy, self.dx + 2 * ix, self.dy + 2 * iy)
    }
}

/// A box in page points, y down.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RectF {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub dx: f32,
    /// Height.
    pub dy: f32,
}

impl RectF {
    /// A box from its left, top, width and height.
    pub const fn new(x: f32, y: f32, dx: f32, dy: f32) -> Self {
        Self { x, y, dx, dy }
    }

    /// Its right edge.
    pub fn right(self) -> f32 {
        self.x + self.dx
    }

    /// Its bottom edge.
    pub fn bottom(self) -> f32 {
        self.y + self.dy
    }

    /// Whether it has no area.
    pub fn is_empty(self) -> bool {
        self.dx <= 0.0 || self.dy <= 0.0
    }
}

impl From<Rect> for RectF {
    fn from(r: Rect) -> Self {
        Self::new(r.x as f32, r.y as f32, r.dx as f32, r.dy as f32)
    }
}

/// A page's text, one character per glyph with the glyph's box, in reading order as the document draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    chars: Vec<char>,
    rects: Vec<Rect>,
}

impl PageText {
    /// An empty page.
    pub fn new() -> Self {
        Self::default()
    }

    /// A page from its characters and their boxes.
    ///
    /// # Panics
    ///
    /// If the two have different lengths.
    pub fn from_parts(chars: Vec<char>, rects: Vec<Rect>) -> Self {
        assert_eq!(chars.len(), rects.len(), "one box per character");
        Self { chars, rects }
    }

    /// A page from MuPDF's text: each character with its box rounded out to whole points, and a line break after each
    /// line, as zero-width glyph at the line's end (SumatraPDF's page text has the same shape).
    pub fn from_layout(layout: &TextLayout) -> Self {
        let mut page = Self::new();
        for line in &layout.lines {
            let mut end = None;
            for c in &layout.chars[line.chars.clone()] {
                let b = c.quad.bounds();
                let (x0, y0) = (b.x0.floor() as i32, b.y0.floor() as i32);
                let r = Rect::new(x0, y0, b.x1.ceil() as i32 - x0, b.y1.ceil() as i32 - y0);
                page.push(c.c, r);
                end = Some(r);
            }
            if let Some(r) = end {
                page.push('\n', Rect::new(r.right(), r.y, 0, r.dy));
            }
        }
        page
    }

    /// Adds a glyph.
    pub fn push(&mut self, ch: char, rect: Rect) {
        self.chars.push(ch);
        self.rects.push(rect);
    }

    /// How many glyphs it has.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether it has no glyphs.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Its characters.
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// Its glyph boxes.
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// The page ready for [`layout`]: watermark glyphs dropped, then each line's glyphs given the line's top and
    /// height (see [`layout::strip_watermark_glyphs`] and [`layout::normalize_glyph_lines`]).
    #[must_use]
    pub fn prepared(&self) -> Self {
        let kept = layout::strip_watermark_glyphs(self);
        let rects = layout::normalize_glyph_lines(&kept.rects);
        Self {
            chars: kept.chars,
            rects,
        }
    }

    fn ch(&self, i: usize) -> char {
        self.chars[i]
    }

    fn rect(&self, i: usize) -> Rect {
        self.rects[i]
    }

    /// The glyphs that leave ink: everything but spaces, tabs and line breaks.
    fn ink(&self) -> impl Iterator<Item = (usize, char, Rect)> + '_ {
        self.chars
            .iter()
            .zip(&self.rects)
            .enumerate()
            .filter(|(_, (c, _))| !is_space(**c))
            .map(|(i, (c, r))| (i, *c, *r))
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Lowercase regardless of locale, so "SECCIÓN" matches "sección".
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}
