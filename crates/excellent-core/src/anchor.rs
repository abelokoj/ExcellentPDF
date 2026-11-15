//! Annotation anchors (plan K.28, Appendix D.1.5): where an annotation sits, told by the text under it, so it can be
//! found again in another version of the document (re-typeset, corrected, reflowed).
//!
//! An anchor is a W3C Web Annotation text quote: the exact text, a little text either side, and the approximate
//! position, all in characters of the document's flat text. [`locate`] finds it again: the exact text near the old
//! position first, then an approximate match (Myers' bit-parallel edit distance), else nothing ("orphaned", for the
//! reader to place).
//!
//! Both sides are compared after [`normalize`]: re-typesetting changes line breaks, hyphenation and ligatures, not the
//! words. Pure: no MuPDF, no UI. Where the anchor is kept (the annotation's `/NM` and a private key, the sidecar)
//! belongs to the annotation model.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// Characters of context kept either side of the exact text.
const CONTEXT: usize = 32;

/// The longest piece searched approximately in one pass: one machine word of pattern bits.
const WORD: usize = 64;

/// Quotes shorter than this (normalized) are only ever placed exactly: a few letters match too much.
const FUZZY_MIN: usize = 8;

/// A text-quote anchor (W3C `TextQuoteSelector` with a `TextPositionSelector` hint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    /// Up to [`CONTEXT`] characters before the exact text.
    pub prefix: String,
    /// The text under the annotation.
    pub exact: String,
    /// Up to [`CONTEXT`] characters after it.
    pub suffix: String,
    /// Where the exact text started, in characters of the flat text it was taken from.
    pub start: usize,
}

impl Quote {
    /// The quote of characters `range` of `text`. `None` when the range holds no text.
    pub fn of(text: &str, range: Range<usize>) -> Option<Self> {
        let chars: Vec<char> = text.chars().collect();
        let end = range.end.min(chars.len());
        let start = range.start.min(end);
        let exact: String = chars[start..end].iter().collect();
        if exact.trim().is_empty() {
            return None;
        }
        Some(Self {
            prefix: chars[start.saturating_sub(CONTEXT)..start].iter().collect(),
            exact,
            suffix: chars[end..(end + CONTEXT).min(chars.len())]
                .iter()
                .collect(),
            start,
        })
    }
}

/// Where a quote was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The exact text, at these characters of the text searched.
    Exact(Range<usize>),
    /// Text close to it, `errors` edits away (normalized).
    Fuzzy {
        /// The characters found.
        range: Range<usize>,
        /// Characters inserted, deleted or changed against the quote.
        errors: usize,
    },
    /// Nothing close enough: the reader places it.
    Orphaned,
}

impl Placement {
    /// The characters found, if any.
    pub fn range(&self) -> Option<Range<usize>> {
        match self {
            Self::Exact(range) | Self::Fuzzy { range, .. } => Some(range.clone()),
            Self::Orphaned => None,
        }
    }
}

/// Finds `quote` in `text` (the flat text of the document as it is now).
pub fn locate(quote: &Quote, text: &str) -> Placement {
    let hay = normalize(text);
    let exact = normalize(&quote.exact).trimmed();
    if exact.chars.is_empty() {
        return Placement::Orphaned;
    }
    let prefix = normalize(&quote.prefix);
    let suffix = normalize(&quote.suffix);
    let expected = hay.near(quote.start);
    let rank = |start: usize, end: usize| -> f64 {
        let before = common_suffix(&prefix.chars, &hay.chars[..start]);
        let after = common_prefix(&suffix.chars, &hay.chars[end..]);
        let context = |found: usize, of: usize| {
            if of == 0 {
                1.0
            } else {
                found as f64 / of as f64
            }
        };
        let distance = start.abs_diff(expected) as f64 / hay.chars.len().max(1) as f64;
        20.0 * context(before, prefix.chars.len())
            + 20.0 * context(after, suffix.chars.len())
            + 2.0 * (1.0 - distance)
    };
    let best_exact = find_all(&hay.chars, &exact.chars)
        .map(|start| (start, start + exact.chars.len()))
        .max_by(|a, b| rank(a.0, a.1).total_cmp(&rank(b.0, b.1)));
    if let Some((start, end)) = best_exact {
        return Placement::Exact(hay.original(start..end));
    }
    let m = exact.chars.len();
    if m < FUZZY_MIN {
        return Placement::Orphaned;
    }
    let allowed = m / 4;
    let found = if m <= WORD {
        approximate(&hay.chars, &exact.chars, allowed)
    } else {
        head_and_tail(&hay.chars, &exact.chars, allowed)
    };
    let best = found.into_iter().min_by(|a, b| {
        a.2.cmp(&b.2)
            .then_with(|| rank(b.0, b.1).total_cmp(&rank(a.0, a.1)))
    });
    match best {
        Some((start, end, _)) => {
            let (start, end) = whole_words(&hay.chars, start, end);
            Placement::Fuzzy {
                range: hay.original(start..end),
                errors: distance(&exact.chars, &hay.chars[start..end]),
            }
        }
        None => Placement::Orphaned,
    }
}

/// `start..end` of `text` without spaces at its ends, then widened to whole words: an approximate match may stop
/// inside a changed word or take the space before it.
fn whole_words(text: &[char], mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end && text[start] == ' ' {
        start += 1;
    }
    while end > start && text[end - 1] == ' ' {
        end -= 1;
    }
    while start > 0 && text[start - 1] != ' ' {
        start -= 1;
    }
    while end < text.len() && text[end] != ' ' {
        end += 1;
    }
    (start, end)
}

/// A new annotation id for `/NM`: a random (version 4) UUID, from the OS-seeded hasher std already has.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let word = || {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.finish()
    };
    let (high, low) = (word(), word());
    let bytes: Vec<u8> = high
        .to_be_bytes()
        .into_iter()
        .chain(low.to_be_bytes())
        .collect();
    let mut b: [u8; 16] = bytes.try_into().expect("two u64s are 16 bytes");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Text as compared, with each character's place in the original.
#[derive(Debug, Default)]
struct Normalized {
    chars: Vec<char>,
    /// For each character, the original character it came from.
    origin: Vec<usize>,
    /// The original's length in characters.
    len: usize,
}

impl Normalized {
    /// The original characters behind normalized `range`.
    fn original(&self, range: Range<usize>) -> Range<usize> {
        let start = self.origin.get(range.start).copied().unwrap_or(self.len);
        let end = match range.end.checked_sub(1) {
            Some(last) if range.end > range.start => self.origin[last] + 1,
            _ => start,
        };
        start..end
    }

    /// The normalized character nearest original character `at`.
    fn near(&self, at: usize) -> usize {
        self.origin.partition_point(|&origin| origin < at)
    }

    /// Without a space at either end.
    fn trimmed(mut self) -> Self {
        if self.chars.last() == Some(&' ') {
            self.chars.pop();
            self.origin.pop();
        }
        if self.chars.first() == Some(&' ') {
            self.chars.remove(0);
            self.origin.remove(0);
        }
        self
    }

    fn push(&mut self, c: char, from: usize) {
        self.chars.push(c);
        self.origin.push(from);
    }
}

/// `text` as re-typesetting leaves it: whitespace runs as one space; soft hyphens gone; a hyphen ending a line joins
/// the word; ligatures spelled out; curly quotes straight; dashes plain. Not trimmed: the context's edge
/// spaces count.
fn normalize(text: &str) -> Normalized {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Normalized {
        len: chars.len(),
        ..Normalized::default()
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            let start = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push(' ', start);
            continue;
        }
        if matches!(c, '-' | '\u{2010}')
            && chars.get(i + 1).is_some_and(|&n| n == '\n' || n == '\r')
        {
            // A word broken at the line's end: join it.
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        match c {
            '\u{00AD}' => {}
            '\u{FB00}' => "ff".chars().for_each(|l| out.push(l, i)),
            '\u{FB01}' => "fi".chars().for_each(|l| out.push(l, i)),
            '\u{FB02}' => "fl".chars().for_each(|l| out.push(l, i)),
            '\u{FB03}' => "ffi".chars().for_each(|l| out.push(l, i)),
            '\u{FB04}' => "ffl".chars().for_each(|l| out.push(l, i)),
            '\u{FB05}' | '\u{FB06}' => "st".chars().for_each(|l| out.push(l, i)),
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => out.push('\'', i),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => out.push('"', i),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-', i),
            _ => out.push(c, i),
        }
        i += 1;
    }
    out
}

/// Every start of `needle` in `hay`.
fn find_all<'a>(hay: &'a [char], needle: &'a [char]) -> impl Iterator<Item = usize> + 'a {
    hay.windows(needle.len())
        .enumerate()
        .filter(move |(_, window)| *window == needle)
        .map(|(start, _)| start)
}

fn common_suffix(a: &[char], b: &[char]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

fn common_prefix(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// The places `pattern` (at most [`WORD`] characters) is at most `allowed` edits from a stretch of `hay`: start, end
/// and errors, the best of each run of neighbouring ends. Myers (1999) as Hyyrö gives it, one bit per pattern
/// character.
fn approximate(hay: &[char], pattern: &[char], allowed: usize) -> Vec<(usize, usize, usize)> {
    let m = pattern.len();
    debug_assert!((1..=WORD).contains(&m), "a pattern of {m} characters");
    let mut peq: HashMap<char, u64> = HashMap::new();
    for (bit, &c) in pattern.iter().enumerate() {
        *peq.entry(c).or_default() |= 1 << bit;
    }
    let high = 1u64 << (m - 1);
    let (mut pv, mut mv, mut score) = (!0u64, 0u64, m);
    // (end, errors), the best of the current run of matches.
    let mut run: Option<(usize, usize)> = None;
    let mut ends = Vec::new();
    for (j, c) in hay.iter().enumerate() {
        let eq = peq.get(c).copied().unwrap_or(0);
        let xv = eq | mv;
        let xh = ((eq & pv).wrapping_add(pv) ^ pv) | eq;
        let mut ph = mv | !(xh | pv);
        let mut mh = pv & xh;
        if ph & high != 0 {
            score += 1;
        } else if mh & high != 0 {
            score -= 1;
        }
        ph <<= 1;
        mh <<= 1;
        pv = mh | !(xv | ph);
        mv = ph & xv;
        if score <= allowed {
            match run {
                Some((_, errors)) if errors <= score => {}
                _ => run = Some((j + 1, score)),
            }
        } else if let Some(best) = run.take() {
            ends.push(best);
        }
    }
    ends.extend(run);
    ends.into_iter()
        .map(|(end, errors)| {
            let start = start_of(hay, pattern, end, errors);
            (start, end, errors)
        })
        .collect()
}

/// Where the match of `pattern` ending at `end` with `errors` edits starts: the start nearest `end - m` among those
/// that keep the errors.
fn start_of(hay: &[char], pattern: &[char], end: usize, errors: usize) -> usize {
    let m = pattern.len();
    let reach = (m + errors).min(end);
    // Edit distance of the reversed pattern against the text read back from `end`: column per text length.
    let mut row: Vec<usize> = (0..=reach).collect();
    for (i, &p) in pattern.iter().rev().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for len in 1..=reach {
            let c = hay[end - len];
            let next = (diagonal + usize::from(c != p))
                .min(row[len] + 1)
                .min(row[len - 1] + 1);
            diagonal = row[len];
            row[len] = next;
        }
    }
    let best = row.iter().copied().min().unwrap_or(errors);
    let len = (0..=reach)
        .filter(|&len| row[len] == best)
        .min_by_key(|&len| len.abs_diff(m))
        .unwrap_or(m);
    end - len
}

/// A long quote placed by its first and last [`WORD`] / 2 characters, found approximately and about the quote's
/// length apart: start, end and the errors of the whole stretch.
fn head_and_tail(hay: &[char], exact: &[char], allowed: usize) -> Vec<(usize, usize, usize)> {
    let piece = WORD / 2;
    let m = exact.len();
    let heads = approximate(hay, &exact[..piece], piece / 4);
    let tails = approximate(hay, &exact[m - piece..], piece / 4);
    let (shortest, longest) = (m - m / 4, m + m / 4);
    let mut found = Vec::new();
    for &(start, _, _) in &heads {
        for &(_, end, _) in &tails {
            if end > start && (shortest..=longest).contains(&(end - start)) {
                let errors = distance(exact, &hay[start..end]);
                if errors <= allowed {
                    found.push((start, end, errors));
                }
            }
        }
    }
    found
}

/// Levenshtein distance, one row at a time.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &y) in b.iter().enumerate() {
            let next = (diagonal + usize::from(x != y))
                .min(row[j + 1] + 1)
                .min(row[j] + 1);
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAPER: &str = "Deflate streams are made of blocks. Each block is compressed with its own Huffman \
trees, which are sent ahead of the data they describe. A block may also be stored, its bytes copied \
as they are, when compression would make it larger. The format leaves the choice of block size to the \
encoder; a good encoder ends a block where the statistics of the input change.";

    fn chars(text: &str, range: Range<usize>) -> String {
        text.chars().skip(range.start).take(range.len()).collect()
    }

    fn quote_of(text: &str, needle: &str) -> Quote {
        let byte = text.find(needle).expect("the needle is in the text");
        let start = text[..byte].chars().count();
        Quote::of(text, start..start + needle.chars().count()).expect("a quote")
    }

    #[test]
    fn the_same_text_places_the_quote_where_it_was() {
        let quote = quote_of(PAPER, "Huffman trees");
        let placed = locate(&quote, PAPER);
        assert_eq!(placed, Placement::Exact(quote.start..quote.start + 13));
    }

    #[test]
    fn context_chooses_between_repeats_of_the_exact_text() {
        let text = "first a block here. Then: the block. And a block there.";
        let byte = text.rfind("block").expect("in the text");
        let start = text[..byte].chars().count();
        let quote = Quote::of(text, start..start + 5).expect("a quote");
        // Moved: the same words with more text in front.
        let moved = format!("Preface. {text}");
        let placed = locate(&quote, &moved).range().expect("placed");
        assert_eq!(placed.start, start + 9);
        assert_eq!(chars(&moved, placed.clone()), "block");
        assert!(chars(&moved, placed.end..placed.end + 6).starts_with(" there"));
    }

    #[test]
    fn re_typesetting_keeps_the_quote_exact() {
        let quote = quote_of(PAPER, "the statistics of the input change");
        // Broken into new lines, a word hyphenated at a line's end, a ligature for "fi" elsewhere.
        let reset = PAPER
            .replace("statistics of the", "statis-\ntics of\nthe")
            .replace("format", "\u{FB01}rst format");
        let placed = locate(&quote, &reset);
        let Placement::Exact(range) = placed else {
            panic!("expected an exact placement, got {placed:?}");
        };
        assert_eq!(
            chars(&reset, range),
            "the statis-\ntics of\nthe input change"
        );
    }

    #[test]
    fn a_corrected_typo_is_placed_approximately() {
        let typo = PAPER.replace("compressed", "compresed");
        let quote = quote_of(&typo, "Each block is compresed with its own");
        let placed = locate(&quote, PAPER);
        let Placement::Fuzzy { range, errors } = placed else {
            panic!("expected a fuzzy placement, got {placed:?}");
        };
        assert_eq!(errors, 1);
        assert_eq!(chars(PAPER, range), "Each block is compressed with its own");
    }

    #[test]
    fn a_long_quote_with_edits_is_placed_by_its_ends() {
        let long = "A block may also be stored, its bytes copied as they are, when compression would make it larger.";
        let quote = quote_of(PAPER, long);
        let edited = PAPER.replace("its bytes copied", "with its bytes copied");
        let placed = locate(&quote, &edited);
        let range = placed.range().expect("placed");
        assert!(
            matches!(placed, Placement::Fuzzy { errors: 5, .. }),
            "{placed:?}"
        );
        assert_eq!(
            chars(&edited, range),
            "A block may also be stored, with its bytes copied as they are, when compression would make it larger."
        );
    }

    #[test]
    fn text_that_is_gone_is_orphaned() {
        let quote = quote_of(PAPER, "sent ahead of the data they describe");
        let cut = PAPER.replace(
            "which are sent ahead of the data they describe",
            "kept elsewhere",
        );
        assert_eq!(locate(&quote, &cut), Placement::Orphaned);
        // A short quote is never placed approximately.
        let short = quote_of(PAPER, "trees");
        assert_eq!(
            locate(&short, &PAPER.replace("trees", "tress")),
            Placement::Orphaned
        );
    }

    #[test]
    fn ids_are_version_4_uuids_and_differ() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"), "{a}");
    }

    /// The text of an imaginary paper, typeset at `width` characters a line: long words broken with hyphens, some
    /// "fi" and "fl" as ligatures.
    fn typeset(words: &[&str], width: usize, ligatures: bool) -> String {
        let mut out = String::new();
        let mut line = 0;
        for word in words {
            let word = if ligatures {
                word.replace("fi", "\u{FB01}").replace("fl", "\u{FB02}")
            } else {
                (*word).to_owned()
            };
            let len = word.chars().count();
            if line > 0 && line + 1 + len > width {
                if len > 6 && width > line + 4 {
                    let cut = width - line - 2;
                    let head: String = word.chars().take(cut).collect();
                    let tail: String = word.chars().skip(cut).collect();
                    out.push(' ');
                    out.push_str(&head);
                    out.push_str("-\n");
                    out.push_str(&tail);
                    line = tail.chars().count();
                    continue;
                }
                out.push('\n');
                line = 0;
            } else if line > 0 {
                out.push(' ');
                line += 1;
            }
            out.push_str(&word);
            line += len;
        }
        out
    }

    /// Done when (K.28): quotes taken from one typesetting are placed on the right words of another, with a few
    /// corrections made between them, at least 95 % of the time.
    #[test]
    fn quotes_survive_a_re_typeset_version_95_percent() {
        // Words drawn from the paper's in a fixed pseudo-random order, so five-word phrases don't repeat.
        let vocabulary: Vec<&str> = PAPER.split_whitespace().collect();
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let words: Vec<&str> = (0..1200)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                vocabulary[(seed >> 33) as usize % vocabulary.len()]
            })
            .collect();
        let before = typeset(&words, 72, false);
        // Re-typeset narrower with ligatures, and every 50th word corrected, as an erratum does: a letter added.
        let changed: Vec<String> = words
            .iter()
            .enumerate()
            .map(|(i, word)| {
                if i % 50 == 25 {
                    format!("{word}s")
                } else {
                    (*word).to_owned()
                }
            })
            .collect();
        let after = typeset(
            &changed.iter().map(String::as_str).collect::<Vec<_>>(),
            58,
            true,
        );
        let flat = |text: &str| -> String { normalize(text).chars.iter().collect() };
        let (mut placed, mut total, mut missed) = (0, 0, Vec::new());
        for at in (0..words.len() - 6).step_by(11) {
            let phrase = words[at..at + 5].join(" ");
            let target = changed[at..at + 5].join(" ");
            // The phrase's occurrence in the before text: count the words before it.
            let start = flat(&words[..at].join(" ")).chars().count() + usize::from(at > 0);
            let normal: Vec<char> = normalize(&before).chars;
            let quote = Quote {
                prefix: normal[start.saturating_sub(CONTEXT)..start]
                    .iter()
                    .collect(),
                exact: phrase.clone(),
                suffix: normal[(start + phrase.chars().count()).min(normal.len())
                    ..(start + phrase.chars().count() + CONTEXT).min(normal.len())]
                    .iter()
                    .collect(),
                start: normalize(&before).origin.get(start).copied().unwrap_or(0),
            };
            total += 1;
            let Some(range) = locate(&quote, &after).range() else {
                missed.push(format!("orphaned at word {at}: {target:?}"));
                continue;
            };
            let found: String = flat(&chars(&after, range.clone()));
            // Right words, and the right occurrence: as many words before it as before the target.
            let before_it = normalize(&chars(&after, 0..range.start)).chars;
            let words_before = before_it
                .split(|&c| c == ' ')
                .filter(|word| !word.is_empty())
                .count();
            if flat(&target) == found && words_before == at {
                placed += 1;
            } else {
                missed.push(format!(
                    "at word {at}: {target:?} found as {found:?} after {words_before} words"
                ));
            }
        }
        assert!(
            placed * 100 >= total * 95,
            "{placed} of {total} placed: {missed:#?}"
        );
    }
}
