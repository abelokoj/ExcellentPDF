//! Transfer annotations to another version of a document (A.16), as Enhanced's annotation transfer does: each text
//! markup is found again by the text it marks (K.28's anchor, or the text under it when it has none), on whatever
//! page that text is now; an annotation that marks no text keeps its page and place. The reader reviews what was
//! placed, moved and orphaned before anything is added. Pure: the app reads both documents' text on the workers.

use excellent_core::anchor::{Placement, Quote, locate, new_id};
use excellent_core::annot::{Annotation, Shape};
use excellent_core::edit::Change;
use excellent_mupdf::{Quad, TextLayout};
use excellent_text::{DocumentText, FlatText, PageText, TextPos};

use super::list::Row;

/// What became of an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fate {
    /// Its text was found as it was, on the same page.
    Placed,
    /// Its text was found changed, or on another page.
    Moved,
    /// It marks no text: it keeps its page and place.
    InPlace,
    /// Its text wasn't found, or its page isn't there: it isn't transferred.
    Orphaned,
}

/// One annotation of the source, and what became of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Transferred {
    /// The annotation in the source, as the list shows it (A.14).
    pub row: Row,
    /// What became of it.
    pub fate: Fate,
    /// The annotation to add to the target, with an id of its own; `None` when orphaned.
    pub placed: Option<Annotation>,
}

/// What became of each of `source`'s annotations, the source's pages' text being `source_text` and the target's
/// `target_text`.
#[must_use]
pub fn transfer(
    source: &[Annotation],
    source_text: Vec<TextLayout>,
    target_text: Vec<TextLayout>,
) -> Vec<Transferred> {
    let from = Text::new(source_text);
    let to = Text::new(target_text);
    // shortcut: replies in threads (A.21) aren't transferred, as their parents get new ids; carry them over with
    // their parents when readers ask for it.
    source
        .iter()
        .filter(|annotation| !annotation.is_reply())
        .map(|annotation| one(annotation, &from, &to))
        .collect()
}

/// The changes that add the annotations `transferred` placed: one operation.
#[must_use]
pub fn changes(transferred: &[Transferred]) -> Vec<Change> {
    transferred
        .iter()
        .filter_map(|t| t.placed.clone().map(Change::CreateAnnotation))
        .collect()
}

/// How many of `transferred` met `fate`.
#[must_use]
pub fn count(transferred: &[Transferred], fate: Fate) -> usize {
    transferred.iter().filter(|t| t.fate == fate).count()
}

fn one(annotation: &Annotation, from: &Text, to: &Text) -> Transferred {
    let row = Row::of(annotation, &[]);
    let (fate, placed) = match &annotation.shape {
        Shape::Markup(kind, quads) => {
            let quote = annotation
                .anchor
                .clone()
                .or_else(|| from.quote(annotation.page, quads));
            match quote.and_then(|quote| to.place(&quote, annotation.page)) {
                Some(found) => {
                    let mut moved = annotation.clone();
                    moved.id = new_id();
                    moved.page = found.page;
                    moved.shape = Shape::Markup(*kind, found.quads);
                    moved.anchor = Some(found.anchor);
                    (found.fate, Some(moved))
                }
                None => (Fate::Orphaned, None),
            }
        }
        _ if annotation.page < to.pages.len() => {
            let mut kept = annotation.clone();
            kept.id = new_id();
            (Fate::InPlace, Some(kept))
        }
        _ => (Fate::Orphaned, None),
    };
    Transferred { row, fate, placed }
}

/// A document's text: its pages', and all of it as one string with the way back to pages.
struct Text {
    pages: Vec<PageText>,
    flat: FlatText,
    all: String,
}

/// Where a quote was found in the target.
struct Found {
    fate: Fate,
    page: usize,
    quads: Vec<Quad>,
    anchor: Quote,
}

impl Text {
    fn new(layouts: Vec<TextLayout>) -> Self {
        let pages: Vec<PageText> = layouts.into_iter().map(PageText::new).collect();
        let flat = FlatText::new(&DocumentText::new(pages.iter().map(Some).collect()));
        let all = flat.text(0..flat.len());
        Self { pages, flat, all }
    }

    /// The quote of the text under `quads` on page `page`; `None` when there is none.
    fn quote(&self, page: usize, quads: &[Quad]) -> Option<Quote> {
        let text = self.pages.get(page)?;
        let (start, end) = quads
            .iter()
            .filter_map(|quad| text.within(quad.bounds()))
            .fold(None, |span: Option<(usize, usize)>, range| {
                Some(span.map_or((range.start, range.end), |(start, end)| {
                    (start.min(range.start), end.max(range.end))
                }))
            })?;
        let offset = |index| self.flat.offset(TextPos { page, index });
        Quote::of(&self.all, offset(start)..offset(end))
    }

    /// Where `quote`, of an annotation on page `page` of the source, is in this text; `None` when it isn't.
    fn place(&self, quote: &Quote, page: usize) -> Option<Found> {
        // shortcut: `locate` normalizes the whole text for each quote, so thousands of highlights in a long book
        // take seconds; normalize once and search the normalized text if transfers that big are common.
        let placement = locate(quote, &self.all);
        let range = placement.range()?;
        let start = self.flat.pos(range.start);
        let end = self.flat.pos(range.end);
        let text = self.pages.get(start.page)?;
        // A quote that now runs onto the next page is marked on the first: a markup lies on one page.
        let end = if end.page == start.page {
            end.index
        } else {
            text.len()
        };
        let quads = text.highlight(start.index..end);
        if quads.is_empty() {
            return None;
        }
        let fate = if matches!(placement, Placement::Exact(_)) && start.page == page {
            Fate::Placed
        } else {
            Fate::Moved
        };
        Some(Found {
            fate,
            page: start.page,
            quads,
            anchor: Quote::of(&self.all, range)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Boxed, Markup, Style};
    use excellent_mupdf::{PagePoint, PageRect, TextBlock, TextChar, TextLine};

    use super::*;

    const ADVANCE: f32 = 6.0;
    const LEADING: f32 = 14.0;

    /// A page whose lines are `lines`, each character a box `ADVANCE` wide.
    fn page(lines: &[&str]) -> TextLayout {
        let mut layout = TextLayout::default();
        for (row, line) in lines.iter().enumerate() {
            let top = 40.0 + LEADING * row as f32;
            let first = layout.chars.len();
            for (col, c) in line.chars().enumerate() {
                let left = 40.0 + ADVANCE * col as f32;
                let at = |x: f32, y: f32| PagePoint { x, y };
                layout.chars.push(TextChar {
                    c,
                    quad: Quad {
                        ul: at(left, top),
                        ur: at(left + ADVANCE, top),
                        ll: at(left, top + 10.0),
                        lr: at(left + ADVANCE, top + 10.0),
                    },
                    size: 10.0,
                });
            }
            layout.lines.push(TextLine {
                bbox: PageRect {
                    x0: 40.0,
                    y0: top,
                    x1: 40.0 + ADVANCE * line.chars().count() as f32,
                    y1: top + 10.0,
                },
                dir: PagePoint { x: 1.0, y: 0.0 },
                vertical: false,
                joined: false,
                block: 0,
                chars: first..layout.chars.len(),
            });
        }
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 40.0,
                y0: 40.0,
                x1: 400.0,
                y1: 40.0 + LEADING * lines.len() as f32,
            },
            lines: 0..layout.lines.len(),
        });
        layout
    }

    /// A highlight over characters `range` of `layout` on page `page`, without an anchor (as another app makes it).
    fn highlight(
        layout: &TextLayout,
        page_index: usize,
        range: std::ops::Range<usize>,
    ) -> Annotation {
        let quads = PageText::new(layout.clone()).highlight(range);
        Annotation::new(
            page_index,
            Shape::Markup(Markup::Highlight, quads),
            Style::default(),
            "Ada",
        )
    }

    fn note(page_index: usize) -> Annotation {
        Annotation::new(
            page_index,
            Shape::Boxed(
                Boxed::Note,
                PageRect {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 30.0,
                    y1: 30.0,
                },
            ),
            Style::default(),
            "Ada",
        )
    }

    /// The text the transferred markup `t` marks in `target`.
    fn marked(t: &Transferred, target: &[TextLayout]) -> String {
        let placed = t.placed.as_ref().expect("placed");
        let Shape::Markup(_, quads) = &placed.shape else {
            panic!("a markup")
        };
        let text = PageText::new(target[placed.page].clone());
        let range = quads
            .iter()
            .filter_map(|quad| text.within(quad.bounds()))
            .fold((usize::MAX, 0), |(start, end), r| {
                (start.min(r.start), end.max(r.end))
            });
        text.text(range.0..range.1)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn highlights_follow_their_text_and_other_annotations_keep_their_place() {
        let source = vec![
            page(&["the quick brown fox", "jumps over the lazy dog"]),
            page(&["a page that goes away"]),
        ];
        let target = vec![
            page(&["a new first page of front matter"]),
            page(&["the quick brown fox jumps", "over the lazy dog"]),
        ];
        let all = [
            highlight(&source[0], 0, 4..15),  // "quick brown": on page 1 now
            highlight(&source[0], 0, 25..37), // "over the laz": across a new line break
            highlight(&source[1], 1, 0..6), // "a page": gone, and too short to be found approximately
            note(0),
            note(1),
            note(7),
        ];
        let transferred = transfer(&all, source, target.clone());
        assert_eq!(transferred[0].fate, Fate::Moved, "on another page");
        assert_eq!(marked(&transferred[0], &target), "quick brown");
        assert_eq!(transferred[0].placed.as_ref().expect("placed").page, 1);
        assert_eq!(marked(&transferred[1], &target), "over the laz");
        assert_eq!(transferred[2].fate, Fate::Orphaned);
        assert_eq!(transferred[3].fate, Fate::InPlace);
        assert_eq!(transferred[4].fate, Fate::InPlace);
        assert_eq!(transferred[5].fate, Fate::Orphaned, "page 8 isn't there");
        let placed = transferred[0].placed.as_ref().expect("placed");
        assert_ne!(
            placed.id, all[0].id,
            "a transferred annotation has an id of its own"
        );
        assert_eq!(
            placed.anchor.as_ref().map(|q| q.exact.as_str()),
            Some("quick brown"),
            "and an anchor in the target's text"
        );
        assert_eq!(count(&transferred, Fate::Orphaned), 2);
        assert_eq!(changes(&transferred).len(), 4, "the orphaned aren't added");
    }

    #[test]
    fn text_that_is_gone_orphans_its_highlight_and_text_in_place_is_placed() {
        let source = vec![page(&["keep this line", "drop this sentence entirely"])];
        let target = vec![page(&["keep this line", "something else was written here"])];
        let all = [
            highlight(&source[0], 0, 0..9),
            highlight(&source[0], 0, 14..41),
        ];
        let transferred = transfer(&all, source, target);
        assert_eq!(transferred[0].fate, Fate::Placed);
        assert_eq!(transferred[1].fate, Fate::Orphaned);
        assert!(transferred[1].placed.is_none());
        assert_eq!(changes(&transferred).len(), 1);
    }

    /// Where a sentence is: its page, its first character there, and after its last.
    type Spot = (usize, usize, usize);

    /// `sentences` laid out in lines of at most `width` characters, `rows` lines a page: the pages, and where each
    /// sentence is (its page, its first character there, after its last); `None` for one split across pages.
    fn typeset(
        sentences: &[String],
        width: usize,
        rows: usize,
    ) -> (Vec<TextLayout>, Vec<Option<Spot>>) {
        let mut lines: Vec<String> = vec![String::new()];
        // Each sentence's first and end (line, column).
        let mut spans = Vec::new();
        for sentence in sentences {
            let mut start = None;
            for word in sentence.split(' ') {
                let last = lines.last().expect("a line");
                if !last.is_empty() && last.chars().count() + 1 + word.chars().count() > width {
                    lines.push(String::new());
                }
                let at = lines.len() - 1;
                let line = lines.last_mut().expect("a line");
                if !line.is_empty() {
                    line.push(' ');
                }
                start.get_or_insert((at, line.chars().count()));
                line.push_str(word);
            }
            let end = (
                lines.len() - 1,
                lines.last().expect("a line").chars().count(),
            );
            spans.push((start.expect("a word"), end));
        }
        // A character's index on its page counts the characters of the page's lines before its own.
        let page_of = |line: usize| line / rows;
        let index_of = |line: usize, column: usize| {
            lines[page_of(line) * rows..line]
                .iter()
                .map(|l| l.chars().count())
                .sum::<usize>()
                + column
        };
        let spots = spans
            .iter()
            .map(|&((l0, c0), (l1, c1))| {
                (page_of(l0) == page_of(l1))
                    .then(|| (page_of(l0), index_of(l0, c0), index_of(l1, c1)))
            })
            .collect();
        let layouts = lines
            .chunks(rows)
            .map(|chunk| page(&chunk.iter().map(String::as_str).collect::<Vec<_>>()))
            .collect();
        (layouts, spots)
    }

    #[test]
    fn a_preprint_s_highlights_land_on_the_published_version_95_percent() {
        let words = [
            "data", "model", "error", "signal", "sample", "theory", "method", "result",
        ];
        let sentence = |n: usize, typo: bool| {
            let verb = if typo { "shwos" } else { "shows" };
            format!(
                "Finding {n} {verb} the {} of the {} under {} {}.",
                words[n % 8],
                words[(n / 8) % 8],
                words[(n / 3) % 8],
                words[(n * 7) % 8]
            )
        };
        let preprint: Vec<String> = (0..300).map(|n| sentence(n, n % 10 == 0)).collect();
        let mut published: Vec<String> =
            vec!["A foreword added for the published version.".to_owned(); 12];
        published.extend((0..300).map(|n| sentence(n, false)));
        let (source, spots) = typeset(&preprint, 62, 30);
        let (target, _) = typeset(&published, 80, 44);
        let mut wanted = Vec::new();
        let mut all = Vec::new();
        for (n, spot) in spots.iter().enumerate().step_by(3) {
            let Some((page_index, start, end)) = *spot else {
                continue;
            };
            all.push(highlight(&source[page_index], page_index, start..end));
            wanted.push(sentence(n, false));
        }
        assert!(all.len() >= 90, "{} highlights", all.len());
        let transferred = transfer(&all, source, target.clone());
        let right = transferred
            .iter()
            .zip(&wanted)
            .filter(|(t, want)| t.placed.is_some() && marked(t, &target) == **want)
            .count();
        assert!(
            right * 100 >= all.len() * 95,
            "{right} of {} placed on the right text",
            all.len()
        );
    }
}
