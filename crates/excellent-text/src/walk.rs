//! Walking a document's text by unit, as a screen reader does through UI Automation's text pattern (plan U.2): a
//! position is a caret on a page ([`TextPos`]), and a unit (character, word, line, paragraph, page, the whole
//! document) is the stretch from one of its starts to the next.
//!
//! The pages whose text isn't read yet count as empty: they hold no character, word, line or paragraph, but are
//! still pages. A unit ends where the next one starts, so the last unit on a page ends at the next page's first
//! start; the document's end ends the last.

use crate::page::PageText;
use crate::selection::{Granularity, TextPos};
use crate::words::{self, Class};

/// What a position moves by. UI Automation's `Format` unit maps to [`TextUnit::Word`] (the next larger unit this
/// text supports, as its rules ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextUnit {
    /// One character.
    Character,
    /// A word and the spaces after it.
    Word,
    /// A line.
    Line,
    /// A block of lines.
    Paragraph,
    /// A page.
    Page,
    /// All of it.
    Document,
}

/// A document's text: each page's, `None` while it isn't read.
#[derive(Debug, Clone)]
pub struct DocumentText<'a> {
    pages: Vec<Option<&'a PageText>>,
}

impl<'a> DocumentText<'a> {
    /// The text of every page, in order; `None` for a page whose text isn't read yet.
    pub fn new(pages: Vec<Option<&'a PageText>>) -> Self {
        Self { pages }
    }

    /// The number of pages.
    pub fn pages(&self) -> usize {
        self.pages.len()
    }

    /// The first position.
    pub fn start(&self) -> TextPos {
        TextPos { page: 0, index: 0 }
    }

    /// The last position: after the last page's last character.
    pub fn end(&self) -> TextPos {
        let page = self.pages.len().saturating_sub(1);
        TextPos {
            page,
            index: self.len(page),
        }
    }

    /// `pos` moved inside the document: onto its last page, and onto its page's text. Positions outlive a book laid
    /// out again (R.28) or a page read since.
    pub fn clamp(&self, pos: TextPos) -> TextPos {
        let last = self.pages.len().saturating_sub(1);
        if pos.page > last {
            // A page the document no longer has lies past its end, whatever the index.
            return TextPos {
                page: last,
                index: self.len(last),
            };
        }
        TextPos {
            page: pos.page,
            index: pos.index.min(self.len(pos.page)),
        }
    }

    pub(crate) fn page(&self, page: usize) -> Option<&'a PageText> {
        self.pages.get(page).copied().flatten()
    }

    fn len(&self, page: usize) -> usize {
        self.page(page).map_or(0, PageText::len)
    }

    /// Where each `unit` starts on `page`, in order, as carets before its end. The document's start always counts.
    fn marks(&self, page: usize, unit: TextUnit) -> Vec<usize> {
        let first = page == 0;
        let mut marks = match (unit, self.page(page)) {
            (TextUnit::Document, _) => Vec::new(),
            (TextUnit::Page, _) => vec![0],
            (_, None) => Vec::new(),
            (TextUnit::Character, Some(text)) => (0..text.len()).collect(),
            (TextUnit::Word, Some(text)) => word_starts(text),
            (TextUnit::Line, Some(text)) => {
                let lines = &text.layout().lines;
                lines
                    .iter()
                    .filter(|line| !line.chars.is_empty())
                    .map(|line| line.chars.start)
                    .collect()
            }
            (TextUnit::Paragraph, Some(text)) => {
                let layout = text.layout();
                layout
                    .blocks
                    .iter()
                    .filter_map(|block| {
                        let lines = layout.lines.get(block.lines.clone())?;
                        lines
                            .iter()
                            .find(|line| !line.chars.is_empty())
                            .map(|line| line.chars.start)
                    })
                    .collect()
            }
        };
        if first && marks.first() != Some(&0) {
            marks.insert(0, 0);
        }
        marks
    }

    /// The start of the `unit` holding `pos`: the last start at or before it.
    pub fn unit_start(&self, pos: TextPos, unit: TextUnit) -> TextPos {
        let pos = self.clamp(pos);
        for page in (0..=pos.page).rev() {
            let marks = self.marks(page, unit);
            if let Some(&index) = marks
                .iter()
                .rev()
                .find(|&&index| TextPos { page, index } <= pos)
            {
                return TextPos { page, index };
            }
        }
        self.start()
    }

    /// The end of the `unit` starting at or holding `pos`: the next start after it, or the document's end.
    pub fn unit_end(&self, pos: TextPos, unit: TextUnit) -> TextPos {
        let pos = self.clamp(pos);
        for page in pos.page..self.pages.len() {
            let marks = self.marks(page, unit);
            if let Some(&index) = marks.iter().find(|&&index| TextPos { page, index } > pos) {
                return TextPos { page, index };
            }
        }
        self.end()
    }

    /// The `unit` around `pos`: from its start to its end.
    pub fn expand(&self, pos: TextPos, unit: TextUnit) -> (TextPos, TextPos) {
        let start = self.unit_start(pos, unit);
        (start, self.unit_end(start, unit))
    }

    /// Moves `pos` over `count` starts of `unit`, forward when positive, back when negative; with `ends`, the
    /// document's start and end count as places to stop too (an endpoint of a range can be there; a unit can't
    /// start at the end). Returns where it stopped and how many it moved, signed like `count`.
    pub fn move_by(&self, pos: TextPos, unit: TextUnit, count: i32, ends: bool) -> (TextPos, i32) {
        let pos = self.clamp(pos);
        let wanted = count.unsigned_abs();
        let mut moved = 0;
        let mut at = pos;
        if count > 0 {
            'pages: for page in pos.page..self.pages.len() {
                for index in self.marks(page, unit) {
                    let mark = TextPos { page, index };
                    if mark <= pos {
                        continue;
                    }
                    at = mark;
                    moved += 1;
                    if moved == wanted {
                        break 'pages;
                    }
                }
            }
            if ends && moved < wanted && at < self.end() {
                at = self.end();
                moved += 1;
            }
        } else if count < 0 {
            'pages: for page in (0..=pos.page).rev() {
                for index in self.marks(page, unit).into_iter().rev() {
                    let mark = TextPos { page, index };
                    if mark >= pos {
                        continue;
                    }
                    at = mark;
                    moved += 1;
                    if moved == wanted {
                        break 'pages;
                    }
                }
            }
        }
        let moved = i32::try_from(moved).unwrap_or(i32::MAX);
        (at, if count < 0 { -moved } else { moved })
    }

    /// The text from `start` to `end`: each page's as [`PageText::text`] gives it, pages joined by a line break; at
    /// most `max` characters when given.
    pub fn text(&self, start: TextPos, end: TextPos, max: Option<usize>) -> String {
        let (start, end) = (self.clamp(start), self.clamp(end));
        let mut pieces = Vec::new();
        for page in start.page..=end.page.min(self.pages.len().saturating_sub(1)) {
            let Some(text) = self.page(page) else {
                continue;
            };
            let from = if page == start.page { start.index } else { 0 };
            let to = if page == end.page {
                end.index
            } else {
                text.len()
            };
            if from < to {
                pieces.push(text.text(from..to));
            }
        }
        let text = pieces.join("\n");
        match max {
            Some(max) => text.chars().take(max).collect(),
            None => text,
        }
    }

    /// The characters of `start..end` on each page it touches, as ranges on that page.
    pub fn spans(&self, start: TextPos, end: TextPos) -> Vec<(usize, core::ops::Range<usize>)> {
        let (start, end) = (self.clamp(start), self.clamp(end));
        (start.page..=end.page)
            .filter_map(|page| {
                let len = self.len(page);
                let from = if page == start.page { start.index } else { 0 };
                let to = if page == end.page { end.index } else { len };
                (from < to).then_some((page, from..to))
            })
            .collect()
    }
}

/// Where words start on `text`: each run of letters or one other character, its trailing spaces belonging to it.
fn word_starts(text: &PageText) -> Vec<usize> {
    let chars = &text.layout().chars;
    let mut starts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let word = text.unit(i, Granularity::Word);
        if words::class(chars[i].c) != Class::Space || text.unit(i, Granularity::Line).start == i {
            starts.push(i);
        }
        i = word.end.max(i + 1);
    }
    starts
}

#[cfg(test)]
pub(crate) mod tests {
    use excellent_mupdf::{PagePoint, PageRect, Quad, TextBlock, TextChar, TextLayout, TextLine};

    use super::*;

    /// A page of `paragraphs`, each a list of lines, laid out left to right, top to bottom.
    pub(crate) fn page(paragraphs: &[&[&str]]) -> PageText {
        let mut layout = TextLayout {
            blocks: Vec::new(),
            lines: Vec::new(),
            chars: Vec::new(),
        };
        let mut y = 0.0;
        for lines in paragraphs {
            let first_line = layout.lines.len();
            for line in *lines {
                let first = layout.chars.len();
                for (x, c) in line.chars().enumerate() {
                    let x = x as f32 * 10.0;
                    let corner = |x: f32, y: f32| PagePoint { x, y };
                    layout.chars.push(TextChar {
                        c,
                        quad: Quad {
                            ul: corner(x, y),
                            ur: corner(x + 10.0, y),
                            ll: corner(x, y + 12.0),
                            lr: corner(x + 10.0, y + 12.0),
                        },
                        size: 12.0,
                    });
                }
                layout.lines.push(TextLine {
                    bbox: rect(0.0, y, line.len() as f32 * 10.0, y + 12.0),
                    dir: PagePoint { x: 1.0, y: 0.0 },
                    vertical: false,
                    joined: false,
                    block: layout.blocks.len(),
                    chars: first..layout.chars.len(),
                });
                y += 14.0;
            }
            layout.blocks.push(TextBlock {
                bbox: rect(0.0, 0.0, 100.0, y),
                lines: first_line..layout.lines.len(),
            });
        }
        PageText::new(layout)
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> PageRect {
        PageRect { x0, y0, x1, y1 }
    }

    fn at(page: usize, index: usize) -> TextPos {
        TextPos { page, index }
    }

    #[test]
    fn units_run_from_one_start_to_the_next_across_pages() {
        let first = page(&[&["Hello world", "again"], &["Next one"]]);
        let second = page(&[&["Page two"]]);
        let doc = DocumentText::new(vec![Some(&first), None, Some(&second)]);

        assert_eq!(doc.expand(at(0, 2), TextUnit::Word), (at(0, 0), at(0, 6)));
        assert_eq!(doc.text(at(0, 0), at(0, 6), None), "Hello ");
        assert_eq!(
            doc.expand(at(0, 12), TextUnit::Line),
            (at(0, 11), at(0, 16))
        );
        assert_eq!(
            doc.expand(at(0, 3), TextUnit::Paragraph),
            (at(0, 0), at(0, 16))
        );
        // The last line of a page ends where the next page with text starts.
        assert_eq!(doc.expand(at(0, 17), TextUnit::Line), (at(0, 16), at(2, 0)));
        // An unread page is still a page.
        assert_eq!(doc.expand(at(1, 0), TextUnit::Page), (at(1, 0), at(2, 0)));
        assert_eq!(
            doc.expand(at(2, 3), TextUnit::Document),
            (at(0, 0), at(2, 8))
        );
        assert_eq!(
            doc.text(doc.start(), doc.end(), None),
            "Hello world\nagain\n\nNext one\nPage two"
        );
        assert_eq!(doc.text(doc.start(), doc.end(), Some(5)), "Hello");
        // A link's words: the characters inside its area.
        assert_eq!(first.within(rect(0.0, 0.0, 50.0, 12.0)), Some(0..5));
        assert_eq!(first.within(rect(500.0, 0.0, 600.0, 12.0)), None);
    }

    #[test]
    fn moving_counts_the_starts_it_passes_and_stops_at_the_ends() {
        let first = page(&[&["one two", "three"]]);
        let second = page(&[&["four"]]);
        let doc = DocumentText::new(vec![Some(&first), Some(&second)]);

        assert_eq!(
            doc.move_by(at(0, 0), TextUnit::Word, 2, false),
            (at(0, 7), 2)
        );
        assert_eq!(
            doc.move_by(at(0, 7), TextUnit::Word, 1, false),
            (at(1, 0), 1)
        );
        assert_eq!(
            doc.move_by(at(1, 0), TextUnit::Word, -3, false),
            (at(0, 0), -3)
        );
        // Past the last start: a range's end can still reach the document's end, a unit's start can't.
        assert_eq!(
            doc.move_by(at(1, 0), TextUnit::Word, 5, false),
            (at(1, 0), 0)
        );
        assert_eq!(
            doc.move_by(at(1, 0), TextUnit::Word, 5, true),
            (at(1, 4), 1)
        );
        assert_eq!(
            doc.move_by(at(0, 0), TextUnit::Page, 1, false),
            (at(1, 0), 1)
        );
        assert_eq!(
            doc.move_by(at(0, 3), TextUnit::Character, -2, false),
            (at(0, 1), -2)
        );
        assert_eq!(
            doc.move_by(at(0, 3), TextUnit::Line, 0, true),
            (at(0, 3), 0)
        );
    }

    #[test]
    fn positions_from_an_older_layout_are_clamped() {
        let only = page(&[&["short"]]);
        let doc = DocumentText::new(vec![Some(&only)]);
        assert_eq!(doc.clamp(at(4, 99)), at(0, 5));
        assert_eq!(doc.spans(at(0, 1), at(3, 0)), vec![(0, 1..5)]);
        let empty = DocumentText::new(Vec::new());
        assert_eq!((empty.start(), empty.end()), (at(0, 0), at(0, 0)));
        assert_eq!(empty.text(empty.start(), empty.end(), None), "");
    }
}
