//! The annotation list (A.14): one row per annotation of the document, grouped by page, filtered by kind, colour,
//! category (A.17), tag, author and date (A.19) and searched in its note, the text it marks and its tags (`tag:method`
//! for one tag, A.18), as Enhanced's annotation search and filter bar do. The filters combine with AND; the choices
//! within one (two colours' worth of kinds, say) with OR. Pure: the app reads the annotations on a worker, makes the
//! rows once, and filters them as the reader types. Each row keeps its text folded to lower case, so a filter over
//! 10,000 rows is one pass of substring tests.

use ecclesia_core::annot::{Annotation, Boxed, Markup, Shape, Status};
use serde_json::{Value, json};

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

    /// The kind named `name` ([`Kind::name`]).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
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
    /// Its tags, as written.
    pub tags: Vec<String>,
    /// When it last changed, in seconds since 1970 (UTC).
    pub modified: i64,
    /// How many replies its comment thread has (A.21).
    pub replies: usize,
    /// Its review status (A.21); `None` when none is set.
    pub status: Option<Status>,
    /// The layer it is on (A.22); `None` for none.
    pub layer: Option<String>,
    /// Its note, the text it marks, its tags and its replies, folded to lower case, for search.
    folded: String,
    /// Its tags, folded to lower case, for `tag:` (A.18) and the tag filter (A.19).
    folded_tags: Vec<String>,
}

impl Row {
    /// The row of `annotation`, with its thread from `replies` (every reply in the document will do, A.21).
    #[must_use]
    pub fn of(annotation: &Annotation, replies: &[Annotation]) -> Self {
        let thread = super::threads::thread(replies, &annotation.id);
        let answered: Vec<&str> = thread.iter().map(|reply| reply.contents.as_str()).collect();
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
            tags: annotation.tags.clone(),
            modified: annotation.modified,
            replies: thread.len(),
            status: super::threads::status_of(replies, &annotation.id),
            layer: annotation.layer.clone(),
            folded: format!(
                "{note}\n{marked}\n{}\n{}",
                annotation.tags.join("\n"),
                answered.join("\n")
            )
            .to_lowercase(),
            folded_tags: annotation
                .tags
                .iter()
                .map(|tag| tag.to_lowercase())
                .collect(),
        }
    }
}

/// The rows of `annotations`, in page order and, on a page, in the order the page draws them. Replies in threads
/// (A.21) are not rows of their own: each row counts its own and finds by them.
#[must_use]
pub fn rows(annotations: &[Annotation]) -> Vec<Row> {
    let (replies, on_page): (Vec<Annotation>, Vec<Annotation>) =
        annotations.iter().cloned().partition(Annotation::is_reply);
    let mut rows: Vec<Row> = on_page
        .iter()
        .map(|annotation| Row::of(annotation, &replies))
        .collect();
    rows.sort_by_key(|row| row.target.page);
    rows
}

/// How recently the rows shown were changed (A.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Since {
    /// In the last day.
    Day,
    /// In the last seven days.
    Week,
    /// In the last thirty days.
    Month,
}

impl Since {
    /// Every choice, in the order the filter offers them.
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    /// Its name, for its label's key (`annotations.since.<name>`) and the saved filter.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// The choice named `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|since| since.name() == name)
    }

    /// How far back it reaches, in seconds.
    fn seconds(self) -> i64 {
        const DAY: i64 = 24 * 60 * 60;
        match self {
            Self::Day => DAY,
            Self::Week => 7 * DAY,
            Self::Month => 30 * DAY,
        }
    }
}

/// What the list shows: some kinds, one colour, some categories, some tags, some authors (none of each: all), the
/// rows changed since a time (none: all), and the rows whose note, marked text or tags hold every word of the search
/// (a `tag:` word: that tag).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// The kinds shown; empty for every kind.
    pub kinds: Vec<Kind>,
    /// The colour shown, `0xRRGGBB`; `None` for every colour.
    pub color: Option<u32>,
    /// The categories shown; empty for every row, with a category or not.
    pub categories: Vec<Category>,
    /// The tags shown, folded to lower case; empty for every row, tagged or not.
    pub tags: Vec<String>,
    /// The authors shown; empty for every author.
    pub authors: Vec<String>,
    /// How recently the rows shown were changed; `None` for any time.
    pub since: Option<Since>,
    /// The search as typed.
    pub query: String,
    /// Whether the pages, too, leave out the annotations the filter doesn't show ("show only matching").
    pub show_only: bool,
}

impl Filter {
    /// Whether it shows every row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
            && self.color.is_none()
            && self.categories.is_empty()
            && self.tags.is_empty()
            && self.authors.is_empty()
            && self.since.is_none()
            && self.query.trim().is_empty()
    }

    /// The indices of the rows of `rows` it shows, in order, as of now.
    #[must_use]
    pub fn apply(&self, rows: &[Row]) -> Vec<usize> {
        self.apply_at(rows, ecclesia_core::annot::now())
    }

    /// The ids of the annotations of `rows` the pages leave out, as of now: those it doesn't show, when it shows only
    /// matching ones; none otherwise.
    #[must_use]
    pub fn hidden(&self, rows: &[Row]) -> Vec<String> {
        if !self.show_only || self.is_empty() {
            return Vec::new();
        }
        let shown = self.apply(rows);
        let mut shown = shown.iter().peekable();
        let mut hidden = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if shown.next_if(|&&at| at == index).is_none() {
                hidden.push(row.target.id.clone());
            }
        }
        hidden
    }

    /// The indices of the rows of `rows` it shows, in order, at `now` (seconds since 1970, UTC).
    #[must_use]
    pub fn apply_at(&self, rows: &[Row], now: i64) -> Vec<usize> {
        let after = self.since.map(|since| now - since.seconds());
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
                    && (self.tags.is_empty()
                        || row.folded_tags.iter().any(|tag| self.tags.contains(tag)))
                    && (self.authors.is_empty() || self.authors.contains(&row.author))
                    && after.is_none_or(|after| row.modified >= after)
                    && words.iter().all(|(tag, word)| match tag {
                        Some(tag) => row.folded_tags.contains(tag),
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
        toggle(&mut next.kinds, kind);
        next
    }

    /// The filter with `category` shown or, when it was, not.
    #[must_use]
    pub fn toggled_category(&self, category: Category) -> Self {
        let mut next = self.clone();
        toggle(&mut next.categories, category);
        next
    }

    /// The filter with the tag `tag` (any case) shown or, when it was, not.
    #[must_use]
    pub fn toggled_tag(&self, tag: &str) -> Self {
        let mut next = self.clone();
        toggle(&mut next.tags, tag.to_lowercase());
        next
    }

    /// The filter with `author`'s annotations shown or, when they were, not.
    #[must_use]
    pub fn toggled_author(&self, author: &str) -> Self {
        let mut next = self.clone();
        toggle(&mut next.authors, author.to_owned());
        next
    }

    /// The filter as kept for its file (`library.db`, A.19).
    #[must_use]
    pub fn to_json(&self) -> String {
        json!({
            "kinds": self.kinds.iter().map(|kind| kind.name()).collect::<Vec<_>>(),
            "color": self.color,
            "categories": self.categories.iter().map(|category| category.key()).collect::<Vec<_>>(),
            "tags": self.tags,
            "authors": self.authors,
            "since": self.since.map(Since::name),
            "query": self.query,
            "show_only": self.show_only,
        })
        .to_string()
    }

    /// The filter kept as `text` by [`Filter::to_json`]; what it can't read is left out, and text that isn't JSON
    /// gives the empty filter.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        let value: Value = serde_json::from_str(text).unwrap_or_default();
        let names = |key: &str| -> Vec<String> {
            value[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| Some(name.as_str()?.to_owned()))
                .collect()
        };
        let mut kinds: Vec<Kind> = names("kinds")
            .iter()
            .filter_map(|n| Kind::from_name(n))
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        let mut categories: Vec<Category> = names("categories")
            .iter()
            .filter_map(|key| Category::from_key(key))
            .collect();
        categories.sort_unstable();
        categories.dedup();
        let mut tags: Vec<String> = names("tags").iter().map(|tag| tag.to_lowercase()).collect();
        tags.sort_unstable();
        tags.dedup();
        let mut authors = names("authors");
        authors.sort_unstable();
        authors.dedup();
        Self {
            kinds,
            color: value["color"]
                .as_u64()
                .and_then(|color| u32::try_from(color).ok())
                .filter(|color| *color <= 0x00ff_ffff),
            categories,
            tags,
            authors,
            since: value["since"].as_str().and_then(Since::from_name),
            query: value["query"].as_str().unwrap_or_default().to_owned(),
            show_only: value["show_only"].as_bool().unwrap_or_default(),
        }
    }
}

/// `list` with `item` taken out or, when it wasn't there, put in its sorted place.
fn toggle<T: Ord>(list: &mut Vec<T>, item: T) {
    match list.binary_search(&item) {
        Ok(at) => {
            list.remove(at);
        }
        Err(at) => list.insert(at, item),
    }
}

/// The tags of `rows`, each once whatever its case (as first written) and sorted, for the filter to offer.
#[must_use]
pub fn tags(rows: &[Row]) -> Vec<String> {
    let mut tags: Vec<(String, String)> = Vec::new();
    for row in rows {
        for (tag, folded) in row.tags.iter().zip(&row.folded_tags) {
            if !tags.iter().any(|(_, seen)| seen == folded) {
                tags.push((tag.clone(), folded.clone()));
            }
        }
    }
    tags.sort_unstable_by(|a, b| a.1.cmp(&b.1));
    tags.into_iter().map(|(tag, _)| tag).collect()
}

/// The authors of `rows`, each once and sorted, for the filter to offer.
#[must_use]
pub fn authors(rows: &[Row]) -> Vec<String> {
    let mut authors: Vec<String> = rows.iter().map(|row| row.author.clone()).collect();
    authors.sort_unstable();
    authors.dedup();
    authors
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

    use ecclesia_core::anchor::Quote;
    use ecclesia_core::annot::Style;
    use ecclesia_mupdf::PageRect;

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
    fn filters_combine_with_and_and_choices_with_or() {
        const DAY: i64 = 24 * 60 * 60;
        let now = 100 * DAY;
        let mut a = note(0, "one", [1.0, 0.0, 0.0]);
        a.author = "Ada".to_owned();
        a.tags = vec!["Method".to_owned()];
        a.modified = now - DAY / 2;
        let mut b = note(1, "two", [1.0, 0.0, 0.0]);
        b.author = "Grace".to_owned();
        b.tags = vec!["review".to_owned()];
        b.modified = now - 3 * DAY;
        let mut c = highlight(2, "three");
        c.author = "Grace".to_owned();
        c.modified = now - 60 * DAY;
        let rows = rows(&[a, b, c]);
        assert_eq!(tags(&rows), ["Method", "review"]);
        assert_eq!(authors(&rows), ["Ada", "Grace"]);
        let grace = Filter::default().toggled_author("Grace");
        assert_eq!(grace.apply_at(&rows, now), vec![1, 2]);
        let week = Filter {
            since: Some(Since::Week),
            ..grace.clone()
        };
        assert_eq!(week.apply_at(&rows, now), vec![1], "Grace's, and this week");
        let today = Filter {
            since: Some(Since::Day),
            ..Filter::default()
        };
        assert_eq!(today.apply_at(&rows, now), vec![0]);
        let tagged = Filter::default()
            .toggled_tag("METHOD")
            .toggled_tag("Review");
        assert_eq!(tagged.apply_at(&rows, now), vec![0, 1], "either tag");
        assert_eq!(
            tagged.toggled_author("Ada").apply_at(&rows, now),
            vec![0],
            "a tag and an author"
        );
        assert!(
            tagged
                .toggled_tag("method")
                .toggled_tag("review")
                .is_empty()
        );
    }

    #[test]
    fn show_only_matching_hides_the_rest_on_the_page() {
        let rows = rows(&[
            note(0, "keep", [1.0, 0.0, 0.0]),
            note(0, "drop", [0.0, 0.0, 1.0]),
            highlight(1, "keep this"),
        ]);
        let keep = Filter {
            query: "keep".to_owned(),
            ..Filter::default()
        };
        assert!(keep.hidden(&rows).is_empty(), "the list only");
        let only = Filter {
            show_only: true,
            ..keep
        };
        assert_eq!(only.hidden(&rows), vec![rows[1].target.id.clone()]);
        let nothing = Filter {
            show_only: true,
            ..Filter::default()
        };
        assert!(
            nothing.hidden(&rows).is_empty(),
            "an empty filter hides nothing"
        );
    }

    #[test]
    fn a_filter_is_kept_as_json_and_read_back() {
        let filter = Filter {
            kinds: vec![Kind::Highlight, Kind::Note],
            color: Some(0xff_ff00),
            categories: vec![Category::Claim],
            tags: vec!["method".to_owned()],
            authors: vec!["Ada".to_owned()],
            since: Some(Since::Month),
            query: "fox".to_owned(),
            show_only: true,
        };
        assert_eq!(Filter::from_json(&filter.to_json()), filter);
        assert_eq!(Filter::from_json("not json"), Filter::default());
        let odd = Filter::from_json(
            r#"{"kinds":["highlight","nope"],"color":-4,"since":"year","tags":["A"]}"#,
        );
        assert_eq!(odd.kinds, [Kind::Highlight]);
        assert_eq!((odd.color, odd.since), (None, None));
        assert_eq!(odd.tags, ["a"]);
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

    #[test]
    fn replies_are_no_rows_but_counted_and_found_on_their_parent_s() {
        use ecclesia_core::annot::Status;
        use ecclesia_core::edit::Change;

        use crate::annotate::threads;

        let parent = note(0, "plain", [1.0, 0.0, 0.0]);
        let made = |change: Option<Change>| match change {
            Some(Change::CreateAnnotation(annotation)) => annotation,
            other => panic!("expected a new annotation, got {other:?}"),
        };
        let answer = made(threads::reply(&parent, "Needs a source", "Grace"));
        let status = made(threads::set_status(
            &parent,
            &[],
            Status::Completed,
            "Grace",
            "Completed",
        ));
        let rows = rows(&[parent.clone(), answer, status]);
        assert_eq!(rows.len(), 1, "replies aren't rows");
        assert_eq!(rows[0].target.id, parent.id);
        assert_eq!(rows[0].replies, 1, "a status isn't a reply");
        assert_eq!(rows[0].status, Some(Status::Completed));
        let search = Filter {
            query: "source".to_owned(),
            ..Filter::default()
        };
        assert_eq!(
            search.apply(&rows),
            vec![0],
            "a reply's words find its parent"
        );
    }
}
