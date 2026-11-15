//! Find in document (plan F.1): matching a query against a page's text (K.13), and the hits found so far.
//!
//! Matching is plain Rust over the text layer, not MuPDF's `fz_search_stext_page_cb`: the canvas already has each
//! page's text as [`TextLayout`], and matching it here gives whole-word and accent-insensitive search, which
//! MuPDF's search does not offer. A line break counts as a space, and a word hyphenated across lines is found whole.
//!
//! shortcut: accents are folded for Latin letters (Latin-1, Latin Extended-A and combining marks), not by full
//! Unicode decomposition; upgrade with K.13's normalization step.

use core::ops::Range;
use std::collections::BTreeMap;

use excellent_mupdf::{Quad, TextLayout};
use excellent_text::PageText;

/// Characters of context kept each side of a hit in its snippet.
const CONTEXT: usize = 40;

/// Whether upper and lower case must match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Case {
    /// "Rust" finds "rust" and "RUST".
    #[default]
    Ignore,
    /// "Rust" finds only "Rust".
    Match,
}

/// Whether a hit may be part of a longer word.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Words {
    /// "read" finds "reader" too.
    #[default]
    Anywhere,
    /// "read" finds only the word "read".
    Whole,
}

/// Whether accents and other marks must match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Marks {
    /// "cafe" finds "café".
    #[default]
    Ignore,
    /// "cafe" doesn't find "café".
    Match,
}

/// How a query matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Upper and lower case.
    pub case: Case,
    /// Whole words.
    pub words: Words,
    /// Accents.
    pub marks: Marks,
}

/// What the reader is looking for, folded for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    needle: Vec<char>,
    options: Options,
}

impl Query {
    /// The query for `text`; `None` when it has nothing but spaces. Runs of spaces match any run of spaces or a line
    /// break.
    pub fn new(text: &str, options: Options) -> Option<Self> {
        let mut needle = Vec::new();
        for c in text.trim().chars() {
            fold(c, options, &mut needle);
        }
        needle.dedup_by(|a, b| *a == ' ' && *b == ' ');
        (!needle.is_empty()).then_some(Self { needle, options })
    }

    /// How it matches.
    pub fn options(&self) -> Options {
        self.options
    }
}

/// Folds `c` as `options` say, onto `out`: spaces become one space, case and marks are dropped when they don't count.
fn fold(c: char, options: Options, out: &mut Vec<char>) {
    if c.is_whitespace() {
        out.push(' ');
        return;
    }
    if options.marks == Marks::Ignore {
        if is_mark(c) {
            return;
        }
        let base = unaccented(c);
        if base != c {
            for b in base_chars(base) {
                fold_case(b, options, out);
            }
            return;
        }
    }
    fold_case(c, options, out);
}

fn fold_case(c: char, options: Options, out: &mut Vec<char>) {
    match options.case {
        Case::Ignore => out.extend(c.to_lowercase()),
        Case::Match => out.push(c),
    }
}

/// A combining mark: what an accent is when it is a character of its own.
fn is_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}')
}

/// Ligatures and letters that unfold to two letters.
fn base_chars(c: char) -> impl Iterator<Item = char> {
    let pair = match c {
        'Æ' => Some(('A', 'E')),
        'æ' => Some(('a', 'e')),
        'Œ' => Some(('O', 'E')),
        'œ' => Some(('o', 'e')),
        'ß' => Some(('s', 's')),
        '\u{fb00}' => Some(('f', 'f')),
        '\u{fb01}' => Some(('f', 'i')),
        '\u{fb02}' => Some(('f', 'l')),
        _ => None,
    };
    match pair {
        Some((a, b)) => [Some(a), Some(b)],
        None => [Some(c), None],
    }
    .into_iter()
    .flatten()
}

/// `c` without its accent, for the Latin letters; anything else as it is. Ligatures come back as themselves, for
/// [`base_chars`].
fn unaccented(c: char) -> char {
    const FROM: &str = "ÀÁÂÃÄÅàáâãäåÇçÈÉÊËèéêëÌÍÎÏìíîïÑñÒÓÔÕÖØòóôõöøÙÚÛÜùúûüÝýÿ\
                        ĀāĂăĄąĆćĈĉĊċČčĎďĐđĒēĔĕĖėĘęĚěĜĝĞğĠġĢģĤĥĦħĨĩĪīĬĭĮįİıĴĵĶķĹĺĻļĽľĿŀŁł\
                        ŃńŅņŇňŌōŎŏŐőŔŕŖŗŘřŚśŜŝŞşŠšŢţŤťŦŧŨũŪūŬŭŮůŰűŲųŴŵŶŷŸŹźŻżŽž";
    const TO: &str = "AAAAAAaaaaaaCcEEEEeeeeIIIIiiiiNnOOOOOOooooooUUUUuuuuYyy\
                      AaAaAaCcCcCcCcDdDdEeEeEeEeEeGgGgGgGgHhHhIiIiIiIiIiJjKkLlLlLlLlLl\
                      NnNnNnOoOoOoRrRrRrSsSsSsSsTtTtTtUuUuUuUuUuUuWwYyYZzZzZz";
    match c {
        'Æ' | 'æ' | 'Œ' | 'œ' | 'ß' | '\u{fb00}'..='\u{fb02}' => c,
        _ if c.is_ascii() => c,
        _ => FROM
            .chars()
            .zip(TO.chars())
            .find_map(|(from, to)| (from == c).then_some(to))
            .unwrap_or(c),
    }
}

/// A page's text as one run of folded characters, each with the character it came from (`None` for the space a line
/// break counts as).
fn haystack(layout: &TextLayout, options: Options) -> Vec<(char, Option<usize>)> {
    let mut hay = Vec::with_capacity(layout.chars.len() + layout.lines.len());
    let mut folded = Vec::new();
    for line in &layout.lines {
        for i in line.chars.clone() {
            let c = layout.chars[i].c;
            // A hyphenated word's hyphen isn't part of its text.
            if line.joined && i + 1 == line.chars.end && is_hyphen(c) {
                continue;
            }
            folded.clear();
            fold(c, options, &mut folded);
            hay.extend(folded.iter().map(|&f| (f, Some(i))));
        }
        if !line.joined {
            hay.push((' ', None));
        }
    }
    hay.dedup_by(|a, b| a.0 == ' ' && b.0 == ' ');
    hay
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{00ad}' | '\u{2010}' | '\u{2011}')
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every hit of `query` on a page with text `layout`, as ranges of its characters, in reading order.
pub fn matches(layout: &TextLayout, query: &Query) -> Vec<Range<usize>> {
    let hay = haystack(layout, query.options);
    let needle = &query.needle;
    let mut hits = Vec::new();
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        let end = at + needle.len();
        let found = hay[at..end].iter().zip(needle).all(|((c, _), n)| c == n);
        let whole = query.options.words == Words::Anywhere
            || ((at == 0 || !is_word(hay[at - 1].0))
                && hay.get(end).is_none_or(|(c, _)| !is_word(*c)));
        if found && whole {
            let mut sources = hay[at..end].iter().filter_map(|(_, source)| *source);
            if let Some(first) = sources.next() {
                let last = sources.next_back().unwrap_or(first);
                hits.push(first..last + 1);
            }
            at = end;
        } else {
            at += 1;
        }
    }
    hits
}

/// A hit with the text around it, as the result list shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snippet {
    /// The text before the hit, cut at a word, with "…" when cut.
    pub before: String,
    /// The hit as the page has it.
    pub hit: String,
    /// The text after the hit, cut at a word, with "…" when cut.
    pub after: String,
}

/// The snippet around characters `chars` of a page with text `layout`.
pub fn snippet(layout: &TextLayout, chars: Range<usize>) -> Snippet {
    let plain = haystack(
        layout,
        Options {
            case: Case::Match,
            words: Words::Anywhere,
            marks: Marks::Match,
        },
    );
    let start = plain
        .iter()
        .position(|(_, s)| s.is_some_and(|s| s >= chars.start))
        .unwrap_or(plain.len());
    let end = plain
        .iter()
        .rposition(|(_, s)| s.is_some_and(|s| s < chars.end))
        .map_or(start, |i| i + 1);
    let text = |range: Range<usize>| -> String { plain[range].iter().map(|(c, _)| c).collect() };
    let from = start.saturating_sub(CONTEXT);
    let to = (end + CONTEXT).min(plain.len());
    let mut before = text(from..start);
    if from > 0 && plain[from - 1].0 != ' ' {
        // Drop the cut word's tail.
        before = match before.find(' ') {
            Some(space) => format!("…{}", &before[space + 1..]),
            None => format!("…{before}"),
        };
    } else if from > 0 {
        before = format!("…{before}");
    }
    let mut after = text(end..to);
    if to < plain.len() && plain[to].0 == ' ' {
        after = format!("{}…", after.trim_end());
    } else if to < plain.len() {
        after = match after.rfind(' ') {
            Some(space) if space > 0 => format!("{}…", &after[..space]),
            _ => format!("{after}…"),
        };
    }
    Snippet {
        before: before.trim_start().to_owned(),
        hit: text(start..end),
        after: after.trim_end().to_owned(),
    }
}

/// One hit on a page: its characters, where it is drawn and its snippet.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Its characters on the page.
    pub chars: Range<usize>,
    /// Its area, a quad per line it is on, in page points.
    pub quads: Vec<Quad>,
    /// It with its context.
    pub snippet: Snippet,
}

/// Every hit of `query` on a page with text `text`.
pub fn find_on_page(text: &PageText, query: &Query) -> Vec<Found> {
    matches(text.layout(), query)
        .into_iter()
        .map(|chars| Found {
            quads: text.highlight(chars.clone()),
            snippet: snippet(text.layout(), chars.clone()),
            chars,
        })
        .collect()
}

/// The order pages are searched in: from the reader's page `start` to the end, then from the start of the document.
pub fn page_order(start: usize, count: usize) -> impl Iterator<Item = usize> {
    let start = start.min(count);
    (start..count).chain(0..start)
}

/// A hit's place: its page and its place among that page's hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HitId {
    /// Its page.
    pub page: usize,
    /// Its place on the page, from 0.
    pub nth: usize,
}

/// Forward or back through the hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The next hit (F3).
    Next,
    /// The previous hit (Shift+F3).
    Previous,
}

/// The hits found so far, page by page, and the one the reader is on.
#[derive(Debug, Clone, Default)]
pub struct Hits<T = Found> {
    pages: BTreeMap<usize, Vec<T>>,
    total: usize,
    current: Option<HitId>,
}

impl<T> Hits<T> {
    /// No hits.
    pub fn new() -> Self {
        Self {
            pages: BTreeMap::new(),
            total: 0,
            current: None,
        }
    }

    /// Adds page `page`'s hits.
    pub fn add(&mut self, page: usize, hits: Vec<T>) {
        if hits.is_empty() {
            return;
        }
        self.total += hits.len();
        if let Some(old) = self.pages.insert(page, hits) {
            self.total -= old.len();
        }
    }

    /// How many hits there are.
    pub fn len(&self) -> usize {
        self.total
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Page `page`'s hits.
    pub fn on_page(&self, page: usize) -> &[T] {
        self.pages.get(&page).map_or(&[], Vec::as_slice)
    }

    /// The hit `id`, if there is one.
    pub fn get(&self, id: HitId) -> Option<&T> {
        self.pages.get(&id.page)?.get(id.nth)
    }

    /// The hit the reader is on.
    pub fn current(&self) -> Option<HitId> {
        self.current
    }

    /// Makes `id` the reader's hit, if it is one.
    pub fn set_current(&mut self, id: HitId) -> bool {
        let ok = self.get(id).is_some();
        if ok {
            self.current = Some(id);
        }
        ok
    }

    /// The current hit's place in document order, from 0.
    pub fn index_of_current(&self) -> Option<usize> {
        let id = self.current?;
        let before: usize = self
            .pages
            .range(..id.page)
            .map(|(_, hits)| hits.len())
            .sum();
        Some(before + id.nth)
    }

    /// The first hit on page `page` or after it, round the end of the document.
    pub fn first_from(&self, page: usize) -> Option<HitId> {
        let (&page, _) = self
            .pages
            .range(page..)
            .next()
            .or_else(|| self.pages.iter().next())?;
        Some(HitId { page, nth: 0 })
    }

    /// Moves to the next or previous hit, round the ends; from no current hit, to the first one on page `page` or
    /// after it (`Next`) or the last one before it (`Previous`). The new current hit, if there are any.
    pub fn step(&mut self, direction: Direction, page: usize) -> Option<HitId> {
        let next = match (self.current, direction) {
            (None, Direction::Next) => self.first_from(page),
            (None, Direction::Previous) => self.last_before(page),
            (Some(id), Direction::Next) => {
                if id.nth + 1 < self.on_page(id.page).len() {
                    Some(HitId {
                        nth: id.nth + 1,
                        ..id
                    })
                } else {
                    self.first_from(id.page + 1)
                }
            }
            (Some(id), Direction::Previous) => {
                if id.nth > 0 {
                    Some(HitId {
                        nth: id.nth - 1,
                        ..id
                    })
                } else {
                    self.last_before(id.page)
                }
            }
        };
        if next.is_some() {
            self.current = next;
        }
        next
    }

    /// The last hit before page `page`, round the start of the document.
    fn last_before(&self, page: usize) -> Option<HitId> {
        let (&page, hits) = self
            .pages
            .range(..page)
            .next_back()
            .or_else(|| self.pages.iter().next_back())?;
        Some(HitId {
            page,
            nth: hits.len() - 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{PagePoint, PageRect, TextBlock, TextChar, TextLine};

    use super::*;

    /// One hit, as `find` returns it.
    fn one(hit: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
        vec![hit]
    }

    /// A page with one line per item of `lines`; a line ending in `-` and followed by another is hyphenated.
    fn page(lines: &[&str]) -> TextLayout {
        let mut layout = TextLayout::default();
        let quad = Quad {
            ul: PagePoint { x: 0.0, y: 0.0 },
            ur: PagePoint { x: 1.0, y: 0.0 },
            ll: PagePoint { x: 0.0, y: 1.0 },
            lr: PagePoint { x: 1.0, y: 1.0 },
        };
        let rect = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
        };
        for (n, text) in lines.iter().enumerate() {
            let start = layout.chars.len();
            layout.chars.extend(text.chars().map(|c| TextChar {
                c,
                quad,
                size: 10.0,
            }));
            layout.lines.push(TextLine {
                bbox: rect,
                dir: PagePoint { x: 1.0, y: 0.0 },
                vertical: false,
                joined: text.ends_with('-') && n + 1 < lines.len(),
                block: 0,
                chars: start..layout.chars.len(),
            });
        }
        layout.blocks.push(TextBlock {
            bbox: rect,
            lines: 0..layout.lines.len(),
        });
        layout
    }

    fn find(layout: &TextLayout, text: &str, options: Options) -> Vec<Range<usize>> {
        matches(layout, &Query::new(text, options).expect("a query"))
    }

    #[test]
    fn case_whole_words_and_accents_match_as_chosen() {
        let layout = page(&["The Café reader read", "CAFE readers"]);
        let any = Options::default();
        assert_eq!(find(&layout, "cafe", any), [4..8, 20..24]);
        let marks = Options {
            marks: Marks::Match,
            ..any
        };
        assert_eq!(find(&layout, "cafe", marks), one(20..24));
        let case = Options {
            case: Case::Match,
            ..any
        };
        assert_eq!(find(&layout, "Cafe", case), one(4..8));
        assert_eq!(find(&layout, "read", any), [9..13, 16..20, 25..29]);
        let whole = Options {
            words: Words::Whole,
            ..any
        };
        assert_eq!(find(&layout, "read", whole), one(16..20));
        assert!(Query::new("   ", any).is_none());
    }

    #[test]
    fn a_line_break_is_a_space_and_a_hyphenated_word_is_whole() {
        let layout = page(&["a well-", "known fact about", "the  page"]);
        let any = Options::default();
        assert_eq!(find(&layout, "wellknown", any), one(2..12));
        assert_eq!(find(&layout, "fact about the", any), one(13..26));
        assert_eq!(find(&layout, "the page", any), one(23..32));
    }

    #[test]
    fn a_snippet_cuts_at_words_round_the_hit() {
        let line = "aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo ppp target qqq rrr sss ttt uuu vvv                     www xxx yyy zzz end";
        let layout = page(&[line]);
        let hit = line.find("target").expect("in the line");
        let snip = snippet(&layout, hit..hit + 6);
        assert_eq!(snip.hit, "target");
        assert_eq!(snip.before, "…ggg hhh iii jjj kkk lll mmm nnn ooo ppp ");
        assert_eq!(snip.after, " qqq rrr sss ttt uuu vvv www xxx yyy zzz…");
        let cut = snippet(&layout, hit + 1..hit + 6);
        assert_eq!(
            cut.before, "…hhh iii jjj kkk lll mmm nnn ooo ppp t",
            "a cut word goes"
        );
        let short = page(&["short page"]);
        assert_eq!(
            snippet(&short, 6..10),
            Snippet {
                before: "short ".to_owned(),
                hit: "page".to_owned(),
                after: String::new()
            }
        );
    }

    #[test]
    fn pages_are_searched_from_the_reader_s_page_round_the_end() {
        assert_eq!(page_order(3, 5).collect::<Vec<_>>(), [3, 4, 0, 1, 2]);
        assert_eq!(page_order(9, 2).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn hits_step_round_the_ends_in_document_order() {
        let mut hits = Hits::new();
        hits.add(5, vec!['a', 'b']);
        hits.add(1, vec!['c']);
        assert_eq!(hits.len(), 3);
        let at = |page, nth| Some(HitId { page, nth });
        assert_eq!(hits.step(Direction::Next, 3), at(5, 0));
        assert_eq!(hits.index_of_current(), Some(1));
        assert_eq!(hits.step(Direction::Next, 0), at(5, 1));
        assert_eq!(hits.step(Direction::Next, 0), at(1, 0), "round the end");
        assert_eq!(
            hits.step(Direction::Previous, 0),
            at(5, 1),
            "round the start"
        );
        let mut fresh: Hits<char> = Hits::new();
        assert_eq!(fresh.step(Direction::Next, 0), None);
        fresh.add(2, vec!['x']);
        assert_eq!(fresh.step(Direction::Previous, 1), at(2, 0));
    }
}
