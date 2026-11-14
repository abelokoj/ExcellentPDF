//! One page's text: hit testing, snapping, highlight and copy.
//!
//! Positions are character indexes into the page's [`TextLayout::chars`]. A *caret* is a boundary between characters
//! (`0..=len`); a range of carets is a stretch of text.

use core::ops::Range;

use excellent_mupdf::{PagePoint, PageRect, Quad, TextChar, TextLayout, TextLine};

use crate::selection::Granularity;
use crate::words::{self, Class};

/// How far outside a line, in multiples of its height, the pointer still counts as over its text.
const OVER_TEXT_SLACK: f32 = 0.25;

/// A page's text, ready for selection on the UI thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    layout: TextLayout,
}

impl From<TextLayout> for PageText {
    fn from(layout: TextLayout) -> Self {
        Self::new(layout)
    }
}

impl PageText {
    /// Wraps a page's text as the workers read it.
    pub fn new(layout: TextLayout) -> Self {
        Self { layout }
    }

    /// The number of characters on the page.
    pub fn len(&self) -> usize {
        self.layout.chars.len()
    }

    /// Whether the page has no text (a scan, say).
    pub fn is_empty(&self) -> bool {
        self.layout.chars.is_empty()
    }

    /// The text as MuPDF laid it out: its blocks, lines and characters.
    pub fn layout(&self) -> &TextLayout {
        &self.layout
    }

    /// Whether `p` is over a line of text: where the pointer becomes an I-beam and a drag selects.
    pub fn is_over_text(&self, p: PagePoint) -> bool {
        self.layout.lines.iter().any(|line| {
            let slack = line_height(line) * OVER_TEXT_SLACK;
            distance(&line.bbox, p) <= slack
        })
    }

    /// The caret nearest to `p`: on the nearest line, between the characters either side of it.
    pub fn caret(&self, p: PagePoint) -> Option<usize> {
        let line = self.nearest_line(p)?;
        let t = along(line, p);
        let before = self
            .line_chars(line)
            .iter()
            .filter(|c| along(line, centre(&c.quad)) < t)
            .count();
        Some(line.chars.start + before)
    }

    /// The character nearest to `p`, on the nearest line.
    pub fn nearest_char(&self, p: PagePoint) -> Option<usize> {
        let line = self.nearest_line(p)?;
        let t = along(line, p);
        let chars = self.line_chars(line);
        let nearest = (0..chars.len()).min_by(|&a, &b| {
            let da = (along(line, centre(&chars[a].quad)) - t).abs();
            let db = (along(line, centre(&chars[b].quad)) - t).abs();
            da.total_cmp(&db)
        })?;
        Some(line.chars.start + nearest)
    }

    /// The stretch of text that character `i` belongs to at `granularity`: itself, its word (a run of letters, of
    /// spaces, or one other character), its line or its block.
    ///
    /// # Panics
    ///
    /// `i` is not a character of the page.
    pub fn unit(&self, i: usize, granularity: Granularity) -> Range<usize> {
        assert!(i < self.len(), "character {i} of {}", self.len());
        let line = self.line_of(i);
        match granularity {
            Granularity::Char => i..i + 1,
            Granularity::Word => self.word(i, line.chars.clone()),
            Granularity::Line => line.chars.clone(),
            Granularity::Paragraph => {
                let block = &self.layout.blocks[line.block];
                let first = &self.layout.lines[block.lines.start];
                let last = &self.layout.lines[block.lines.end - 1];
                first.chars.start..last.chars.end
            }
        }
    }

    /// The areas to paint for `range`: one quad per line it touches, from its first character's left edge to its
    /// last character's right edge, following the line's slant.
    pub fn highlight(&self, range: Range<usize>) -> Vec<Quad> {
        self.line_spans(range)
            .map(|chars| {
                let (first, last) = (&chars[0].quad, &chars[chars.len() - 1].quad);
                Quad {
                    ul: first.ul,
                    ll: first.ll,
                    ur: last.ur,
                    lr: last.lr,
                }
            })
            .collect()
    }

    /// The text of `range`: a line break between lines, a blank line between blocks. A word hyphenated across lines
    /// is joined again, as MuPDF's own text output does.
    pub fn text(&self, range: Range<usize>) -> String {
        let mut out = String::new();
        // The block of the last line copied, and whether it ran on into the next.
        let mut previous: Option<(usize, bool)> = None;
        for line in &self.layout.lines {
            let start = line.chars.start.max(range.start);
            let mut end = line.chars.end.min(range.end);
            if start >= end {
                continue;
            }
            match previous {
                Some((_, true)) | None => {}
                Some((block, false)) => {
                    out.push_str(if block == line.block { "\n" } else { "\n\n" })
                }
            }
            let joined = line.joined && end == line.chars.end;
            if joined && is_hyphen(self.layout.chars[end - 1].c) {
                end -= 1;
            }
            out.extend(self.layout.chars[start..end].iter().map(|c| c.c));
            previous = Some((line.block, joined));
        }
        out
    }

    fn line_chars(&self, line: &TextLine) -> &[TextChar] {
        &self.layout.chars[line.chars.clone()]
    }

    /// The characters of each line `range` touches, clipped to it.
    fn line_spans(&self, range: Range<usize>) -> impl Iterator<Item = &[TextChar]> {
        self.layout.lines.iter().filter_map(move |line| {
            let start = line.chars.start.max(range.start);
            let end = line.chars.end.min(range.end);
            (start < end).then(|| &self.layout.chars[start..end])
        })
    }

    /// The line holding character `i`. Lines cover the characters in order, so this is a binary search.
    fn line_of(&self, i: usize) -> &TextLine {
        let at = self
            .layout
            .lines
            .partition_point(|line| line.chars.end <= i);
        &self.layout.lines[at]
    }

    /// The nearest line to `p`, by distance to its box: a line beside the pointer beats one above or below.
    fn nearest_line(&self, p: PagePoint) -> Option<&TextLine> {
        self.line_near(p).map(|i| &self.layout.lines[i])
    }

    /// The line nearest to `p`, as an index into [`TextLayout::lines`]; `None` on a page without text.
    pub fn line_near(&self, p: PagePoint) -> Option<usize> {
        self.layout
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.chars.is_empty())
            .min_by(|(_, a), (_, b)| distance(&a.bbox, p).total_cmp(&distance(&b.bbox, p)))
            .map(|(i, _)| i)
    }

    /// The word around character `i`, within its line.
    fn word(&self, i: usize, line: Range<usize>) -> Range<usize> {
        let class = words::class(self.layout.chars[i].c);
        if class == Class::Alone {
            return i..i + 1;
        }
        let same = |j: usize| words::class(self.layout.chars[j].c) == class;
        let start = (line.start..i)
            .rev()
            .take_while(|&j| same(j))
            .last()
            .unwrap_or(i);
        let end = (i + 1..line.end)
            .take_while(|&j| same(j))
            .last()
            .map_or(i + 1, |j| j + 1);
        start..end
    }
}

/// The hyphens MuPDF drops where it joins a word across lines (`fz_is_unicode_hyphen`).
fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{ad}' | '\u{2010}' | '\u{2011}')
}

fn centre(q: &Quad) -> PagePoint {
    PagePoint {
        x: (q.ul.x + q.ur.x + q.ll.x + q.lr.x) / 4.0,
        y: (q.ul.y + q.ur.y + q.ll.y + q.lr.y) / 4.0,
    }
}

/// How far along `line`'s baseline direction `p` is.
fn along(line: &TextLine, p: PagePoint) -> f32 {
    p.x * line.dir.x + p.y * line.dir.y
}

/// The line's height across its baseline (its width, for vertical text).
fn line_height(line: &TextLine) -> f32 {
    if line.dir.x.abs() >= line.dir.y.abs() {
        line.bbox.height()
    } else {
        line.bbox.width()
    }
}

/// The distance from `p` to the rectangle `r` (0 inside it).
fn distance(r: &PageRect, p: PagePoint) -> f32 {
    let dx = (r.x0 - p.x).max(p.x - r.x1).max(0.0);
    let dy = (r.y0 - p.y).max(p.y - r.y1).max(0.0);
    dx.hypot(dy)
}

#[cfg(test)]
pub(crate) mod tests {
    use excellent_mupdf::TextBlock;

    use super::*;

    /// A page laid out like a monospaced printout: each block a list of lines, each character 10 pt wide and 12 pt
    /// tall, lines 20 pt apart, blocks 20 pt further apart.
    pub(crate) fn page(blocks: &[&[&str]]) -> PageText {
        let mut layout = TextLayout::default();
        let mut y = 0.0;
        for (b, lines) in blocks.iter().enumerate() {
            let first_line = layout.lines.len();
            for text in *lines {
                let first_char = layout.chars.len();
                for (x, c) in text.chars().enumerate() {
                    let x0 = x as f32 * 10.0;
                    let at = |x, y| PagePoint { x, y };
                    layout.chars.push(TextChar {
                        c,
                        quad: Quad {
                            ul: at(x0, y),
                            ur: at(x0 + 10.0, y),
                            ll: at(x0, y + 12.0),
                            lr: at(x0 + 10.0, y + 12.0),
                        },
                        size: 12.0,
                    });
                }
                let width = text.chars().count() as f32 * 10.0;
                layout.lines.push(TextLine {
                    bbox: PageRect {
                        x0: 0.0,
                        y0: y,
                        x1: width,
                        y1: y + 12.0,
                    },
                    dir: PagePoint { x: 1.0, y: 0.0 },
                    vertical: false,
                    joined: text.ends_with('-'),
                    block: b,
                    chars: first_char..layout.chars.len(),
                });
                y += 20.0;
            }
            layout.blocks.push(TextBlock {
                bbox: PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 0.0,
                    y1: 0.0,
                },
                lines: first_line..layout.lines.len(),
            });
            y += 20.0;
        }
        PageText::new(layout)
    }

    fn at(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    #[test]
    fn the_caret_falls_between_the_characters_either_side_of_the_pointer() {
        let text = page(&[&["hello world", "second"]]);
        assert_eq!(text.caret(at(0.0, 6.0)), Some(0));
        assert_eq!(text.caret(at(14.0, 6.0)), Some(1), "left half of the e");
        assert_eq!(text.caret(at(16.0, 6.0)), Some(2), "right half of the e");
        assert_eq!(
            text.caret(at(500.0, 6.0)),
            Some(11),
            "past the end of the line"
        );
        assert_eq!(
            text.caret(at(5.0, 26.0)),
            Some(11),
            "the s of the second line"
        );
        assert_eq!(
            text.caret(at(15.0, 15.0)),
            Some(1),
            "just under the first line, nearer it"
        );
        assert_eq!(PageText::default().caret(at(0.0, 0.0)), None);
    }

    #[test]
    fn the_pointer_is_over_text_on_a_line_or_just_beside_it() {
        let text = page(&[&["hello"]]);
        assert!(text.is_over_text(at(25.0, 6.0)));
        assert!(
            text.is_over_text(at(52.0, 6.0)),
            "within a quarter line of the end"
        );
        assert!(!text.is_over_text(at(80.0, 6.0)));
        assert!(!text.is_over_text(at(25.0, 40.0)));
    }

    #[test]
    fn words_lines_and_paragraphs_snap_around_a_character() {
        let text = page(&[&["don't stop,  now", "next"], &["block two"]]);
        assert_eq!(text.unit(1, Granularity::Word), 0..5, "don't");
        assert_eq!(text.unit(10, Granularity::Word), 10..11, "the comma alone");
        assert_eq!(text.unit(11, Granularity::Word), 11..13, "both spaces");
        assert_eq!(
            text.unit(14, Granularity::Word),
            13..16,
            "now, which ends the line"
        );
        assert_eq!(
            text.unit(16, Granularity::Word),
            16..20,
            "next, on its own line"
        );
        assert_eq!(text.unit(3, Granularity::Char), 3..4);
        assert_eq!(text.unit(17, Granularity::Line), 16..20);
        assert_eq!(text.unit(17, Granularity::Paragraph), 0..20);
        assert_eq!(text.unit(22, Granularity::Paragraph), 20..29);
    }

    #[test]
    fn a_highlight_is_one_quad_per_line() {
        let text = page(&[&["hello world", "second"]]);
        let quads = text.highlight(6..13);
        assert_eq!(quads.len(), 2);
        assert_eq!((quads[0].ul, quads[0].lr), (at(60.0, 0.0), at(110.0, 12.0)));
        assert_eq!((quads[1].ul, quads[1].lr), (at(0.0, 20.0), at(20.0, 32.0)));
        assert!(text.highlight(3..3).is_empty());
    }

    #[test]
    fn copied_text_breaks_lines_and_separates_blocks() {
        let text = page(&[&["hello world", "second"], &["third"]]);
        assert_eq!(text.text(6..13), "world\nse");
        assert_eq!(text.text(0..text.len()), "hello world\nsecond\n\nthird");
        assert_eq!(text.text(2..2), "");
    }

    #[test]
    fn a_word_hyphenated_across_lines_is_copied_whole() {
        let text = page(&[&["the ver-", "sion is"]]);
        assert_eq!(text.text(0..text.len()), "the version is");
        assert_eq!(text.text(4..10), "versi", "a selection across the join");
    }

    #[test]
    fn the_nearest_character_is_the_one_under_the_pointer() {
        let text = page(&[&["hello"]]);
        assert_eq!(text.nearest_char(at(16.0, 6.0)), Some(1));
        assert_eq!(text.nearest_char(at(99.0, 6.0)), Some(4));
    }
}
