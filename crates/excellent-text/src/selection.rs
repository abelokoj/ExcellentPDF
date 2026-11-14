//! A selection across pages: where it started, where it is now, and what it snaps to.

use core::ops::Range;

use crate::page::PageText;

/// What a selection grows by: a drag selects characters, a double-click words, a triple-click lines, a fourth click
/// paragraphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Granularity {
    /// Character by character.
    #[default]
    Char,
    /// Whole words.
    Word,
    /// Whole lines.
    Line,
    /// Whole blocks of lines.
    Paragraph,
}

impl Granularity {
    /// The granularity of a click by its count: 1 characters, 2 words, 3 lines, 4 or more paragraphs.
    pub fn from_clicks(clicks: u32) -> Self {
        match clicks {
            0 | 1 => Self::Char,
            2 => Self::Word,
            3 => Self::Line,
            _ => Self::Paragraph,
        }
    }
}

/// A place in a document's text. With [`Granularity::Char`] the index is a caret (`0..=len`); otherwise it is the
/// character the pointer was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPos {
    /// The page, from 0.
    pub page: usize,
    /// The caret or character on it.
    pub index: usize,
}

/// A stretch of a document's text, possibly across pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where it started: the end that stays put while the pointer moves.
    pub anchor: TextPos,
    /// Where it is now.
    pub focus: TextPos,
    /// What it snaps to.
    pub granularity: Granularity,
}

impl Selection {
    /// A selection starting and ending at `at`: a caret, or the unit (word, line, ...) around a character.
    pub fn new(at: TextPos, granularity: Granularity) -> Self {
        Self {
            anchor: at,
            focus: at,
            granularity,
        }
    }

    /// Its first and last positions, in reading order.
    fn ends(&self) -> (TextPos, TextPos) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }

    /// The pages it touches.
    pub fn pages(&self) -> Range<usize> {
        let (first, last) = self.ends();
        first.page..last.page + 1
    }

    /// Whether nothing is selected: a caret with no width.
    pub fn is_collapsed(&self) -> bool {
        self.granularity == Granularity::Char && self.anchor == self.focus
    }

    /// The characters selected on `page`, whose text is `text`; `None` when it selects nothing there.
    pub fn range_on(&self, page: usize, text: &PageText) -> Option<Range<usize>> {
        let (first, last) = self.ends();
        if !(first.page..=last.page).contains(&page) || text.is_empty() {
            return None;
        }
        let len = text.len();
        let start = if page == first.page {
            self.start(first.index, text)
        } else {
            0
        };
        let end = if page == last.page {
            self.end(last.index, text)
        } else {
            len
        };
        (start < end).then_some(start..end)
    }

    /// The selected text, pages joined by a line break; `text_of` gives each page's text, `None` when it has none.
    pub fn text<'a>(&self, text_of: impl Fn(usize) -> Option<&'a PageText>) -> String {
        let pieces: Vec<String> = self
            .pages()
            .filter_map(|page| {
                let text = text_of(page)?;
                Some(text.text(self.range_on(page, text)?))
            })
            .collect();
        pieces.join("\n")
    }

    fn start(&self, index: usize, text: &PageText) -> usize {
        match self.granularity {
            Granularity::Char => index.min(text.len()),
            g => text.unit(index.min(text.len() - 1), g).start,
        }
    }

    fn end(&self, index: usize, text: &PageText) -> usize {
        match self.granularity {
            Granularity::Char => index.min(text.len()),
            g => text.unit(index.min(text.len() - 1), g).end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::tests::page;

    fn pos(page: usize, index: usize) -> TextPos {
        TextPos { page, index }
    }

    #[test]
    fn a_drag_selects_between_its_carets_either_way() {
        let text = page(&[&["hello world"]]);
        let mut s = Selection::new(pos(0, 8), Granularity::Char);
        assert!(s.is_collapsed());
        assert_eq!(s.range_on(0, &text), None);
        s.focus = pos(0, 2);
        assert_eq!(s.range_on(0, &text), Some(2..8));
        assert_eq!(s.text(|_| Some(&text)), "llo wo");
    }

    #[test]
    fn a_double_click_drag_grows_by_whole_words() {
        let text = page(&[&["one two three"]]);
        let mut s = Selection::new(pos(0, 5), Granularity::Word);
        assert_eq!(s.range_on(0, &text), Some(4..7), "two");
        s.focus = pos(0, 9);
        assert_eq!(s.range_on(0, &text), Some(4..13), "two three");
        s.focus = pos(0, 1);
        assert_eq!(s.range_on(0, &text), Some(0..7), "one two, dragging back");
    }

    #[test]
    fn a_selection_across_pages_takes_all_of_the_pages_between() {
        let first = page(&[&["end of one"]]);
        let middle = page(&[&["all of two"]]);
        let last = page(&[&["start of three"]]);
        let texts = [&first, &middle, &last];
        let s = Selection {
            anchor: pos(2, 5),
            focus: pos(0, 7),
            granularity: Granularity::Char,
        };
        assert_eq!(s.pages(), 0..3);
        assert_eq!(s.range_on(0, &first), Some(7..10));
        assert_eq!(s.range_on(1, &middle), Some(0..10));
        assert_eq!(s.range_on(2, &last), Some(0..5));
        assert_eq!(s.range_on(3, &last), None);
        assert_eq!(s.text(|p| texts.get(p).copied()), "one\nall of two\nstart");
    }

    #[test]
    fn clicks_choose_the_granularity() {
        let g: Vec<_> = (1..=5).map(Granularity::from_clicks).collect();
        use Granularity::*;
        assert_eq!(g, [Char, Word, Line, Paragraph, Paragraph]);
    }
}
