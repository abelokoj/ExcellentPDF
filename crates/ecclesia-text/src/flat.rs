//! A document's text as one string (plan U.3), for screen-reader interfaces that count characters from the start:
//! GTK's `GtkAccessibleText` and AppKit's text areas. Each line's characters are followed by a line break, so a
//! reader hears lines as lines; a page whose text isn't read yet adds nothing.
//!
//! Offsets count `char`s. A line's break maps to the caret at that line's end.

use core::ops::Range;

use crate::selection::TextPos;
use crate::walk::DocumentText;

/// One page's place in the flat text.
#[derive(Debug, Clone)]
struct Page {
    /// Its first offset.
    start: usize,
    /// Its lines' characters, on the page.
    lines: Vec<Range<usize>>,
    /// Its number of characters.
    len: usize,
}

/// A document's text, flattened, with the way back to pages.
#[derive(Debug, Clone)]
pub struct FlatText {
    chars: Vec<char>,
    pages: Vec<Page>,
}

impl FlatText {
    /// The flat text of `document`.
    pub fn new(document: &DocumentText<'_>) -> Self {
        let mut chars = Vec::new();
        let mut pages = Vec::with_capacity(document.pages());
        for index in 0..document.pages() {
            let start = chars.len();
            let Some(text) = document.page(index) else {
                pages.push(Page {
                    start,
                    lines: Vec::new(),
                    len: 0,
                });
                continue;
            };
            let layout = text.layout();
            let lines: Vec<Range<usize>> =
                layout.lines.iter().map(|line| line.chars.clone()).collect();
            for line in &lines {
                chars.extend(layout.chars[line.clone()].iter().map(|c| c.c));
                chars.push('\n');
            }
            pages.push(Page {
                start,
                lines,
                len: text.len(),
            });
        }
        Self { chars, pages }
    }

    /// The number of characters.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether there is no text.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The characters of `range`, clamped to the text.
    pub fn text(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.chars.len());
        self.chars[range.start.min(end)..end].iter().collect()
    }

    /// The line holding `offset`, counted from 0 across the document.
    pub fn line_at(&self, offset: usize) -> usize {
        let end = offset.min(self.chars.len());
        self.chars[..end].iter().filter(|&&c| c == '\n').count()
    }

    /// Line `line`'s offsets, its break included; empty past the last line.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let mut start = 0;
        for (n, piece) in self.chars.split_inclusive(|&c| c == '\n').enumerate() {
            if n == line {
                return start..start + piece.len();
            }
            start += piece.len();
        }
        start..start
    }

    /// The offset of `pos`.
    pub fn offset(&self, pos: TextPos) -> usize {
        let Some(page) = self.pages.get(pos.page) else {
            return self.chars.len();
        };
        let index = pos.index.min(page.len);
        // The lines starting at or before `index`; each one before it adds its break.
        let line = page
            .lines
            .partition_point(|line| line.start <= index)
            .saturating_sub(1);
        page.start + index + line
    }

    /// The position at `offset`: on a line's break, the caret at that line's end.
    pub fn pos(&self, offset: usize) -> TextPos {
        let Some(last) = self.pages.len().checked_sub(1) else {
            return TextPos { page: 0, index: 0 };
        };
        let offset = offset.min(self.chars.len());
        // The last page starting at or before `offset` (pages without text start where the next one does).
        let page = self
            .pages
            .partition_point(|page| page.start <= offset)
            .saturating_sub(1)
            .min(last);
        let Page { start, lines, len } = &self.pages[page];
        let local = offset - start;
        for (k, line) in lines.iter().enumerate() {
            // The line's characters and its break.
            if local <= line.end + k {
                return TextPos {
                    page,
                    index: (local - k.min(local)).max(line.start).min(line.end),
                };
            }
        }
        TextPos { page, index: *len }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::tests::page;

    fn at(page: usize, index: usize) -> TextPos {
        TextPos { page, index }
    }

    #[test]
    fn lines_end_in_breaks_and_unread_pages_add_nothing() {
        let first = page(&[&["ab", "c"], &["de"]]);
        let third = page(&[&["xyz"]]);
        let document = DocumentText::new(vec![Some(&first), None, Some(&third)]);
        let flat = FlatText::new(&document);
        assert_eq!(flat.text(0..flat.len()), "ab\nc\nde\nxyz\n");
        assert_eq!(flat.text(3..7), "c\nde");
        assert_eq!(flat.text(10..99), "z\n");
    }

    #[test]
    fn positions_and_offsets_round_trip() {
        let first = page(&[&["ab", "c"]]);
        let third = page(&[&["xyz"]]);
        let document = DocumentText::new(vec![Some(&first), None, Some(&third)]);
        let flat = FlatText::new(&document);
        // "ab\nc\n" then "xyz\n": every character's offset leads back to it.
        for pos in [at(0, 0), at(0, 1), at(0, 2), at(0, 3), at(2, 0), at(2, 2)] {
            assert_eq!(flat.pos(flat.offset(pos)), pos, "{pos:?}");
        }
        assert_eq!(
            flat.offset(at(0, 2)),
            3,
            "c comes after the first line's break"
        );
        assert_eq!(
            flat.offset(at(0, 3)),
            4,
            "the caret after c is its line's break"
        );
        assert_eq!(flat.offset(at(2, 1)), 6);
        // A break is the caret at its line's end.
        assert_eq!(flat.pos(2), at(0, 2));
        assert_eq!(
            flat.pos(5),
            at(2, 0),
            "the unread page holds no offset of its own"
        );
        assert_eq!(flat.pos(99), at(2, 3));
    }

    #[test]
    fn lines_count_from_the_document_start() {
        let first = page(&[&["ab", "c"]]);
        let flat = FlatText::new(&DocumentText::new(vec![Some(&first)]));
        assert_eq!(
            (flat.line_at(0), flat.line_at(2), flat.line_at(3)),
            (0, 0, 1)
        );
        assert_eq!(flat.line_range(1), 3..5);
        assert_eq!(flat.line_range(2), 5..5);
    }

    #[test]
    fn no_pages_is_empty() {
        let flat = FlatText::new(&DocumentText::new(Vec::new()));
        assert!(flat.is_empty());
        assert_eq!(flat.pos(3), at(0, 0));
        assert_eq!(flat.offset(at(1, 1)), 0);
    }
}
