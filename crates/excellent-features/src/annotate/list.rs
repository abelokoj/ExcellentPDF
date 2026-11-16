//! The annotation list (A.14): one row per annotation of the document, grouped by page, filtered by kind, colour and
//! category (A.17) and searched in its note, the text it marks and its tags (`tag:method` for one tag, A.18), as Enhanced's annotation search and filter bar do. Pure: the app
//! reads the annotations on a worker, makes the rows once, and filters them as the reader types. Each row keeps its
//! text folded to lower case, so a filter over 10,000 rows is one pass of substring tests.

use excellent_core::annot::{Annotation, Boxed, Markup, Shape};

use super::categories::Category;
use super::notes::Target;
use super::tags;

/// What kind of annotation a row is, as the list names and filters it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A highlight.
    Highlight,
    /// An underline or a squiggly underline.
    Underline,
    /// A strikeout.
    StrikeOut,
    /// A sticky note.
    Note,
    /// A text box, typewriter text or a callout.
    Text,
    /// A line or an arrow.
    Line,
    /// A rectangle, an ellipse, a polygon, a polyline or a cloud.
    Shape,
    /// A freehand drawing.
    Ink,
    /// A stamp or an insertion caret.
    Stamp,
}

impl Kind {
    /// Every kind, in the order the filter offers them.
    pub const ALL: [Self; 9] = [
        Self::Highlight,
        Self::Underline,
        Self::StrikeOut,
        Self::Note,
        Self::Text,
        Self::Line,
        Self::Shape,
        Self::Ink,
        Self::Stamp,
    ];

    /// The kind of `annotation`.
    #[must_use]
    pub fn of(annotation: &Annotation) -> Self {
        match &annotation.shape {
            Shape::Markup(Markup::Highlight, _) => Self::Highlight,
            Shape::Markup(Markup::Underline | Markup::Squiggly, _) => Self::Underline,
            Shape::Markup(Markup::StrikeOut, _) => Self::StrikeOut,
            Shape::Boxed(Boxed::Note, _) => Self::Note,
            Shape::Boxed(Boxed::FreeText | Boxed::Typewriter, _) | Shape::Callout(..) => Self::Text,
            Shape::Line(..) => Self::Line,
            Shape::Boxed(Boxed::Square | Boxed::Circle, _) | Shape::Path(..) => Self::Shape,
            Shape::Ink(_) => Self::Ink,
            Shape::Boxed(Boxed::Stamp | Boxed::Caret, _) => Self::Stamp,
        }
    }

    /// Its name, for its label's key (`annotations.kind.<name>`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::StrikeOut => "strikeout",
            Self::Note => "note",
            Self::Text => "text",
            Self::Line => "line",
            Self::Shape => "shape",
            Self::Ink => "ink",
            Self::Stamp => "stamp",
        }
    }
}

/// One annotation, as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Which annotation it is.
    pub target: Target,
    /// Its kind.
    pub kind: Kind,
    /// Its colour, `0xRRGGBB`; `None` when it has none.
    pub color: Option<u32>,
    /// Its category (A.17); `None` when it has none.
    pub category: Option<Category>,
    /// What the row reads: its note, else the text it marks; empty when it has neither.
    pub text: String,
    /// Who made it.
    pub author: String,
    /// Its note, the text it marks and its tags, folded to lower case, for search.
    folded: String,
    /// Its tags, folded to lower case, for `tag:` (A.18).
    tags: Vec<String>,
}

impl Row {
    /// The row of `annotation`.
    #[must_use]
    pub fn of(annotation: &Annotation) -> Self {
        let marked = annotation
            .anchor
            .as_ref()
            .map_or("", |quote| quote.exact.as_str());
        let note = annotation.contents.trim();
        let text = if note.is_empty() { marked.trim() } else { note };
        Self {
            target: Target {
                page: annotation.page,
                id: annotation.id.clone(),
            },
            kind: Kind::of(annotation),
            color: annotation.style.color.or(annotation.fill).map(rgb),
            category: Category::of(annotation),
            text: one_line(text),
            author: annotation.author.clone(),
            folded: format!("{note}\n{marked}\n{}", annotation.tags.join("\n")).to_lowercase(),
            tags: annotation
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect(),
        }
    }
}

/// The rows of `annotations`, in page order and, on a page, in the order the page draws them.
#[must_use]
pub fn rows(annotations: &[Annotation]) -> Vec<Row> {
    let mut rows: Vec<Row> = annotations.iter().map(Row::of).collect();
    rows.sort_by_key(|row| row.target.page);
    rows
}

/// What the list shows: some kinds (none: all), one colour (none: all), some categories (none: all), and the rows
/// whose note, marked text or tags hold every word of the search (a `tag:` word: that tag).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// The kinds shown; empty for every kind.
    pub kinds: Vec<Kind>,
    /// The colour shown, `0xRRGGBB`; `None` for every colour.
    pub color: Option<u32>,
    /// The categories shown; empty for every row, with a category or not.
    pub categories: Vec<Category>,
    /// The search as typed.
    pub query: String,
}

impl Filter {
    /// Whether it shows every row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
            && self.color.is_none()
            && self.categories.is_empty()
            && self.query.trim().is_empty()
    }

    /// The indices of the rows of `rows` it shows, in order.
    #[must_use]
    pub fn apply(&self, rows: &[Row]) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let words: Vec<(Option<String>, &str)> = query
            .split_whitespace()
            .map(|word| (tags::asked(word), word))
            .collect();
        rows.iter()
            .enumerate()
            .filter(|(_, row)| {
                (self.kinds.is_empty() || self.kinds.contains(&row.kind))
                    && self.color.is_none_or(|color| row.color == Some(color))
                    && (self.categories.is_empty()
                        || row.category.is_some_and(|c| self.categories.contains(&c)))
                    && words.iter().all(|(tag, word)| match tag {
                        Some(tag) => row.tags.contains(tag),
                        None => row.folded.contains(word),
                    })
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The filter with `kind` shown or, when it was, not.
    #[must_use]
    pub fn toggled_kind(&self, kind: Kind) -> Self {
        let mut next = self.clone();
        if let Some(at) = next.kinds.iter().position(|k| *k == kind) {
            next.kinds.remove(at);
        } else {
            next.kinds.push(kind);
            next.kinds.sort_unstable();
        }
        next
    }
}

impl Filter {
    /// The filter with `category` shown or, when it was, not.
    #[must_use]
    pub fn toggled_category(&self, category: Category) -> Self {
        let mut next = self.clone();
        if let Some(at) = next.categories.iter().position(|c| *c == category) {
            next.categories.remove(at);
        } else {
            next.categories.push(category);
            next.categories.sort_unstable();
        }
        next
    }
}

/// The categories of `rows`, each once and in [`Category::ALL`]'s order, for the filter to offer.
#[must_use]
pub fn categories(rows: &[Row]) -> Vec<Category> {
    Category::ALL
        .into_iter()
        .filter(|category| rows.iter().any(|row| row.category == Some(*category)))
        .collect()
}

/// The kinds and the colours of `rows`, each once and in order, for the filter to offer.
#[must_use]
pub fn offered(rows: &[Row]) -> (Vec<Kind>, Vec<u32>) {
    let mut kinds: Vec<Kind> = rows.iter().map(|row| row.kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut colors: Vec<u32> = Vec::new();
    for color in rows.iter().filter_map(|row| row.color) {
        if !colors.contains(&color) {
            colors.push(color);
        }
    }
    (kinds, colors)
}

/// One line of the list: a page's heading or a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A page's heading.
    Page {
        /// The page, from zero.
        page: usize,
        /// How many of the shown rows are on it.
        count: usize,
    },
    /// The row at this index of the rows.
    Row(usize),
}

/// The lines that show the rows `shown` (indices into `rows`, in order): each page's heading, then its rows.
#[must_use]
pub fn entries(rows: &[Row], shown: &[usize]) -> Vec<Entry> {
    let mut entries = Vec::with_capacity(shown.len() + shown.len() / 4);
    let mut at = 0;
    while at < shown.len() {
        let page = rows[shown[at]].target.page;
        let count = shown[at..]
            .iter()
            .take_while(|&&index| rows[index].target.page == page)
            .count();
        entries.push(Entry::Page { page, count });
        entries.extend(shown[at..at + count].iter().map(|&index| Entry::Row(index)));
        at += count;
    }
    entries
}

/// `color`, RGB from 0 to 1, as `0xRRGGBB`.
fn rgb(color: [f32; 3]) -> u32 {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
    (byte(color[0]) << 16) | (byte(color[1]) << 8) | byte(color[2])
}

/// `text` on one line: runs of white space, line breaks among them, made one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use excellent_core::anchor::Quote;
    use excellent_core::annot::Style;
    use excellent_mupdf::PageRect;

    use super::*;

    fn note(page: usize, contents: &str, color: [f32; 3]) -> Annotation {
        let mut a = Annotation::new(
            page,
            Shape::Boxed(Boxed::Note, PageRect::default()),
            Style {
                color: Some(color),
                ..Style::default()
            },
            "Ada",
        );
        a.contents = contents.to_owned();
        a
    }

    fn highlight(page: usize, marked: &str) -> Annotation {
        let mut a = Annotation::new(
            page,
            Shape::Markup(Markup::Highlight, Vec::new()),
            Style {
                color: Some([1.0, 1.0, 0.0]),
                ..Style::default()
            },
            "Ada",
        );
        a.anchor = Quote::of(marked, 0..marked.chars().count());
        a
    }

    #[test]
    fn rows_read_the_note_else_the_marked_text_and_group_by_page() {
        let all = [
            note(3, "Check\n  this", [1.0, 0.0, 0.0]),
            highlight(1, "The Quick fox"),
        ];
        let rows = rows(&all);
        assert_eq!(rows[0].target.page, 1, "in page order");
        assert_eq!(rows[0].text, "The Quick fox");
        assert_eq!(rows[0].color, Some(0xffff00));
        assert_eq!(rows[1].text, "Check this", "one line");
        assert_eq!(rows[1].kind, Kind::Note);
        let shown = Filter::default().apply(&rows);
        assert_eq!(
            entries(&rows, &shown),
            vec![
                Entry::Page { page: 1, count: 1 },
                Entry::Row(0),
                Entry::Page { page: 3, count: 1 },
                Entry::Row(1),
            ]
        );
        assert_eq!(
            offered(&rows),
            (vec![Kind::Highlight, Kind::Note], vec![0xffff00, 0xff0000])
        );
    }

    #[test]
    fn the_filter_takes_kinds_a_colour_and_every_word_of_the_search_in_any_case() {
        let rows = rows(&[
            note(0, "Ask Grace about this", [1.0, 0.0, 0.0]),
            highlight(0, "the QUICK brown fox"),
            note(1, "quick note", [0.0, 0.0, 1.0]),
        ]);
        let search = |query: &str| {
            Filter {
                query: query.to_owned(),
                ..Filter::default()
            }
            .apply(&rows)
        };
        assert_eq!(search("quick"), vec![1, 2]);
        assert_eq!(search("  fox   QUICK "), vec![1], "every word, any order");
        assert_eq!(search("grace"), vec![0]);
        assert!(search("zebra").is_empty());
        let notes = Filter::default().toggled_kind(Kind::Note);
        assert_eq!(notes.apply(&rows), vec![0, 2]);
        assert!(notes.toggled_kind(Kind::Note).is_empty());
        let blue = Filter {
            color: Some(0x0000ff),
            ..notes
        };
        assert_eq!(blue.apply(&rows), vec![2]);
    }

    #[test]
    fn a_tag_word_finds_that_tag_and_plain_words_find_tags_too() {
        let mut method = highlight(0, "we sampled ten sites");
        method.tags = vec!["Method".to_owned(), "to read".to_owned()];
        let mut methods = note(1, "methods compared", [1.0, 0.0, 0.0]);
        methods.tags = vec!["review".to_owned()];
        let rows = rows(&[method, methods]);
        let search = |query: &str| {
            Filter {
                query: query.to_owned(),
                ..Filter::default()
            }
            .apply(&rows)
        };
        assert_eq!(
            search("tag:method"),
            vec![0],
            "the tag, not a word in a note"
        );
        assert_eq!(search("TAG:Method sampled"), vec![0]);
        assert_eq!(
            search("method"),
            vec![0, 1],
            "a plain word finds tags and notes"
        );
        assert_eq!(search("review"), vec![1]);
        assert!(search("tag:meth").is_empty(), "a whole tag");
    }

    #[test]
    fn the_filter_shows_the_chosen_categories_only() {
        let mut claim = highlight(0, "the earth is round");
        claim.category = Some("claim".to_owned());
        let mut todo = note(1, "check the source", [1.0, 0.0, 0.0]);
        todo.category = Some("todo".to_owned());
        let rows = rows(&[claim, todo, note(2, "plain", [1.0, 0.0, 0.0])]);
        assert_eq!(rows[0].category, Some(Category::Claim));
        assert_eq!(categories(&rows), vec![Category::Claim, Category::ToDo]);
        let claims = Filter::default().toggled_category(Category::Claim);
        assert!(!claims.is_empty());
        assert_eq!(claims.apply(&rows), vec![0]);
        let both = claims.toggled_category(Category::ToDo);
        assert_eq!(both.apply(&rows), vec![0, 1], "no category, not shown");
        assert_eq!(
            both.toggled_category(Category::Claim)
                .toggled_category(Category::ToDo),
            Filter::default()
        );
    }

    #[test]
    fn ten_thousand_annotations_filter_as_you_type_within_a_frame() {
        let words = [
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "theta",
        ];
        let all: Vec<Annotation> = (0..10_000)
            .map(|n| {
                let text = format!(
                    "{} {} remark number {n} about the chapter on {}",
                    words[n % 7],
                    words[(n / 7) % 7],
                    words[(n / 49) % 7]
                );
                if n % 2 == 0 {
                    note(n / 20, &text, [1.0, 0.0, 0.0])
                } else {
                    highlight(n / 20, &text)
                }
            })
            .collect();
        let rows = rows(&all);
        let filter = Filter {
            query: "gamma chapter 9".to_owned(),
            ..Filter::default()
        };
        // The fastest of a few runs, so a busy machine's one stall doesn't decide it.
        let best = (0..5)
            .map(|_| {
                let started = Instant::now();
                let shown = filter.apply(&rows);
                let entries = entries(&rows, &shown);
                assert!(!entries.is_empty());
                started.elapsed()
            })
            .min()
            .expect("five runs");
        // 16 ms is a frame at 60 Hz; an unoptimized test build gets ten times that.
        let limit = if cfg!(debug_assertions) { 160 } else { 16 };
        assert!(best.as_millis() < limit, "filtering took {best:?}");
    }
}
