//! Finding the citation under the pointer, and its entry in the references by plain text when the document has no
//! link to follow. Ported from SumatraPDF Enhanced's `src/RefHoverTextDetect.cpp`.

use super::{PageText, Rect, fold, is_space};

/// How far from the pointer, in points, the nearest glyph may be.
const MAX_POINTER_DIST: i64 = 30;
/// Lowercase particles that are part of a surname ("van der Berg", "de la Cruz").
const NAME_PREFIXES: &[&str] = &[
    "van", "von", "de", "der", "den", "la", "le", "el", "al", "da", "du", "di", "do", "bin", "ben",
    "te", "ten", "ter", "op", "'t", "af", "av", "zu", "san", "st", "st.",
];

/// An author-year citation: "(Smith et al., 2020)", "Smith (2020)".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorYear {
    /// The first author's surname as printed, which may be more than one word ("van der Berg").
    pub surname: String,
    /// The year.
    pub year: u16,
    /// Where this citation is on its page, from the surname to the year: tells two citations of one work apart.
    pub source: Rect,
}

/// A numeric citation: "\[12\]", or the number nearest the pointer in "\[3, 7-9\]".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numeric {
    /// The reference number.
    pub number: u32,
    /// Where the bracketed list is on its page (it may cover two lines).
    pub source: Rect,
}

/// Where a reference entry starts: its first glyph's top left, in page points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
}

impl Anchor {
    fn of(r: Rect) -> Self {
        Self {
            x: r.x as f32,
            y: r.y as f32,
        }
    }
}

/// The glyph whose centre is nearest the point, if within 30 pt.
fn nearest_glyph(text: &PageText, x: i32, y: i32) -> Option<usize> {
    let mut best = None;
    let mut best_dist = i64::MAX;
    for (i, r) in text.rects().iter().enumerate() {
        if r.dx <= 0 && r.dy <= 0 {
            continue;
        }
        let ddx = i64::from(r.x + r.dx / 2 - x);
        let ddy = i64::from(r.y + r.dy / 2 - y);
        let dist = ddx * ddx + ddy * ddy;
        if dist < best_dist {
            best_dist = dist;
            best = Some(i);
        }
    }
    best.filter(|_| best_dist <= MAX_POINTER_DIST * MAX_POINTER_DIST)
}

/// Whether the point is on the glyph, give or take a point for rounding (but no further: nearby prose isn't a target).
fn hits_glyph(glyph: Rect, x: i32, y: i32) -> bool {
    !glyph.is_empty() && glyph.inflate(1, 1).contains(x, y)
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.right().min(b.right()) > a.x.max(b.x) && a.bottom().min(b.bottom()) > a.y.max(b.y)
}

fn name_char(c: char) -> bool {
    c.is_alphabetic() || c == '-' || c == '\''
}

fn is_name_prefix(word: &[char]) -> bool {
    word.first().is_some_and(|c| c.is_lowercase())
        && NAME_PREFIXES
            .iter()
            .any(|p| p.chars().map(fold).eq(word.iter().copied().map(fold)))
}

/// The box of glyphs `start..=end` on one line: their left and right, the first one's top and height.
fn glyph_span_bounds(text: &PageText, start: usize, end: usize) -> Rect {
    let mut line = None;
    let (mut x_min, mut x_max) = (i32::MAX, i32::MIN);
    for r in text.rects().iter().take(end + 1).skip(start) {
        if r.dx <= 0 && r.dy <= 0 {
            continue;
        }
        line.get_or_insert((r.y, r.dy));
        x_min = x_min.min(r.x);
        x_max = x_max.max(r.right());
    }
    line.map_or_else(Rect::default, |(y, dy)| {
        Rect::new(x_min, y, x_max - x_min, dy)
    })
}

/// The author-year citation under the point: the year nearest the pointer (preferring one after it, so the pointer on
/// "Gu" in "(Bashab et al., 2023; Gu et al., 2025)" gets 2025), and the surname before it, through "et al." and
/// citation punctuation. The point must be on the citation's glyphs.
pub fn detect_author_year(text: &PageText, x: i32, y: i32) -> Option<AuthorYear> {
    let cursor = nearest_glyph(text, x, y)?;

    // Three lines around the pointer's, as one string: line breaks and runs of spaces (kerning in Word PDFs puts
    // spaces inside words, "et  al .") become one space. Each character remembers its glyph.
    let c = text.rect(cursor);
    let line_h = if c.dy + 4 < 12 { 14 } else { c.dy + 4 };
    let (y_min, y_max) = (c.y - line_h - 2, c.y + line_h * 2 + 2);
    let mut s: Vec<char> = Vec::new();
    let mut glyphs: Vec<Option<usize>> = Vec::new();
    let mut cursor_pos = None;
    let mut prev_y = None;
    let mut last_was_space = true;
    for (i, (&ch, r)) in text.chars().iter().zip(text.rects()).enumerate() {
        if r.y < y_min || r.y > y_max {
            continue;
        }
        let line_break = prev_y.is_some_and(|p| r.y > p + 2);
        if i == cursor {
            cursor_pos = Some(s.len());
        }
        if line_break || is_space(ch) {
            if !last_was_space {
                s.push(' ');
                glyphs.push(None);
                last_was_space = true;
            }
        } else {
            s.push(ch);
            glyphs.push(Some(i));
            last_was_space = false;
        }
        prev_y = Some(r.y);
    }
    let cursor_pos = cursor_pos?;

    let year_at = |i: usize| -> Option<u16> {
        let digits = s.get(i..i + 4)?;
        if !digits.iter().all(char::is_ascii_digit)
            || s.get(i + 4).is_some_and(char::is_ascii_digit)
            || (i > 0 && s[i - 1].is_ascii_digit())
        {
            return None;
        }
        let year = digits
            .iter()
            .fold(0u16, |v, d| v * 10 + d.to_digit(10).unwrap_or(0) as u16);
        (1900..=2050).contains(&year).then_some(year)
    };
    let after = (cursor_pos..s.len().saturating_sub(3))
        .take_while(|i| i - cursor_pos <= 60)
        .find(|i| year_at(*i).is_some());
    let year_pos = match after {
        Some(p) => p,
        None => {
            let nearest = (0..s.len().saturating_sub(3))
                .filter(|i| year_at(*i).is_some())
                .min_by_key(|i| i.abs_diff(cursor_pos))?;
            if nearest.abs_diff(cursor_pos) > 80 {
                return None;
            }
            nearest
        }
    };
    let year = year_at(year_pos)?;

    // Back from the year over spaces and citation punctuation.
    let at = |p: isize| -> char { if p >= 0 { s[p as usize] } else { '\0' } };
    let mut p = year_pos as isize - 1;
    let mut punctuated = false;
    while p >= 0 && matches!(at(p), ' ' | '\t' | ',' | '(' | '\n' | ';') {
        punctuated |= matches!(at(p), ',' | '(');
        p -= 1;
    }
    // Over "et al." in its broken forms: the full stop and the "a" may be missing, spaces may come between.
    {
        let mut q = p;
        if at(q) == '.' {
            q -= 1;
            while at(q) == ' ' {
                q -= 1;
            }
        }
        if q >= 4 && matches!(at(q), 'l' | 'L') {
            let mut r = q - 1;
            if matches!(at(r), 'a' | 'A') {
                r -= 1;
            }
            while at(r) == ' ' {
                r -= 1;
            }
            if r >= 1 && matches!(at(r), 't' | 'T') && matches!(at(r - 1), 'e' | 'E') {
                punctuated = true;
                p = r - 2;
                while at(p) == ' ' {
                    p -= 1;
                }
            }
        }
    }

    // Back over the surname: name characters, lowercase particles and more capitalised words ("Oude Vrielink",
    // "van der Berg", "El Mansouri").
    let surname_end = p + 1;
    while p >= 0 {
        let ch = at(p);
        if name_char(ch) {
            p -= 1;
            continue;
        }
        if ch != ' ' {
            break;
        }
        let mut word_end = p - 1;
        while at(word_end) == ' ' {
            word_end -= 1;
        }
        let mut word_start = word_end;
        while word_start >= 0 && name_char(at(word_start)) {
            word_start -= 1;
        }
        word_start += 1;
        if word_end < word_start {
            break;
        }
        let word = &s[word_start as usize..=word_end as usize];
        let first = word[0];
        // "and", "&" and other words end it, except a one- or two-letter lowercase scrap between capitalised words
        // (extraction broke "Oude" into "O de").
        if first.is_lowercase() && !is_name_prefix(word) {
            let mut peek = word_start - 1;
            while at(peek) == ' ' {
                peek -= 1;
            }
            let mut peek_end = peek;
            while peek_end >= 0 && name_char(at(peek_end)) {
                peek_end -= 1;
            }
            let capital_before =
                word.len() <= 2 && peek > peek_end && at(peek_end + 1).is_uppercase();
            if !capital_before {
                break;
            }
        }
        if !first.is_lowercase() && !first.is_uppercase() {
            break;
        }
        p = word_start - 1;
    }
    let mut surname_start = p + 1;
    if surname_end <= surname_start {
        return None;
    }
    while surname_start < surname_end && matches!(at(surname_start), ' ' | '.') {
        surname_start += 1;
    }
    if surname_end - surname_start < 2 || !at(surname_start).is_uppercase() {
        return None;
    }
    if !punctuated && at(surname_start - 1) != '(' {
        return None;
    }
    let (start, end) = (surname_start as usize, surname_end as usize);
    let span = start..year_pos + 4;
    if !glyphs[span.clone()]
        .iter()
        .flatten()
        .any(|g| hits_glyph(text.rect(*g), x, y))
    {
        return None;
    }
    let surname: String = s[start..end]
        .iter()
        .collect::<String>()
        .trim_end_matches([' ', '.', ','])
        .to_owned();
    if surname.chars().count() < 2 {
        return None;
    }
    let mut on_page = glyphs[span].iter().flatten();
    let source = match (on_page.next(), on_page.next_back()) {
        (Some(a), Some(b)) => glyph_span_bounds(text, *a, *b),
        (Some(a), None) => glyph_span_bounds(text, *a, *a),
        _ => Rect::default(),
    };
    Some(AuthorYear {
        surname,
        year,
        source,
    })
}

/// The reference entry for an author-year citation: a line starting at the page's left text edge with the surname
/// (or, for a surname of three letters or more, with it within the line's first 30 characters, for a surname the
/// extraction broke up), whose next 600 characters hold the year. A match in brackets ("(Smith") or at `source`
/// (the citation itself) doesn't count.
pub fn find_surname(
    text: &PageText,
    surname: &str,
    year: u16,
    source: Option<Rect>,
) -> Option<Anchor> {
    let name: Vec<char> = surname.chars().map(fold).collect();
    if name.is_empty() {
        return None;
    }
    let chars = text.chars();
    let left_x = text.ink().map(|(_, _, r)| r.x).min()?;
    let year: Vec<char> = format!("{:04}", year % 10000).chars().collect();
    let starts_with = |k: usize| {
        k + name.len() <= chars.len()
            && chars[k..k + name.len()]
                .iter()
                .map(|c| fold(*c))
                .eq(name.iter().copied())
    };

    let mut prev_y: Option<i32> = None;
    let mut line_first = None;
    for (i, _, r) in text.ink() {
        if prev_y.is_none_or(|p| r.y > p + 2) {
            line_first = Some(i);
        }
        prev_y = Some(r.y);
        if line_first != Some(i) || r.x > left_x + 20 {
            continue;
        }
        let found = if starts_with(i) {
            Some(i)
        } else if name.len() >= 3 {
            let line_end = (i + 30).min(chars.len());
            (i + 1..line_end.saturating_sub(name.len()) + 1)
                .find(|k| starts_with(*k) && !chars[k - 1].is_alphabetic())
        } else {
            None
        };
        let Some(at) = found else { continue };
        if at > i && chars[at - 1] == '(' {
            continue;
        }
        if source.is_some_and(|s| intersects(text.rect(at), s)) {
            continue;
        }
        let scan_end = (at + 600).min(chars.len());
        let has_year = (at + name.len()..scan_end.saturating_sub(3)).any(|j| {
            chars[j..j + 4] == year[..]
                && !(j > 0 && chars[j - 1].is_ascii_digit())
                && !(j + 4 < scan_end && chars[j + 4].is_ascii_digit())
        });
        if has_year {
            return Some(Anchor::of(r));
        }
    }
    None
}

/// A character that can be inside a numeric citation: digits, separators, and the dashes of a range ("9–14").
fn list_char(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(
            c,
            ' ' | '\t' | ',' | '-' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}'
        )
}

/// Glyphs further apart than this are in different columns.
const COL_GAP: i32 = 16;

/// One line's part in one column: the glyphs whose baseline is within `tol` of `baseline`, left to right, cut at
/// gaps wider than a gutter, keeping the run that overlaps `lo..=hi`. With its left and right. Keeps the other column
/// and a watermark's scattered glyphs out.
fn segment(
    text: &PageText,
    baseline: i32,
    tol: i32,
    lo: i32,
    hi: i32,
) -> Option<(Vec<usize>, i32, i32)> {
    let mut line: Vec<usize> = (0..text.len())
        .filter(|i| (text.rect(*i).bottom() - baseline).abs() <= tol)
        .collect();
    line.sort_by_key(|i| text.rect(*i).x);
    let mut s = 0;
    while s < line.len() {
        let mut e = s;
        while e + 1 < line.len() && text.rect(line[e + 1]).x - text.rect(line[e]).right() <= COL_GAP
        {
            e += 1;
        }
        let (run_lo, run_hi) = (text.rect(line[s]).x, text.rect(line[e]).right());
        if run_hi >= lo && run_lo <= hi {
            return Some((line[s..=e].to_vec(), run_lo, run_hi));
        }
        s = e + 1;
    }
    None
}

/// The numeric citation under the point: the bracketed list around it, read in local reading order (the line above,
/// the pointer's line and the line below, each left to right in its column), so a list that wraps ("[8, 17,\n18]")
/// is whole even when the document draws its parts far apart. The number nearest the pointer; the point must be on
/// the brackets' glyphs.
pub fn detect_numeric(text: &PageText, x: i32, y: i32) -> Option<Numeric> {
    let cursor = nearest_glyph(text, x, y)?;
    let c = text.rect(cursor);
    let line_tol = if c.dy + 4 < 12 { 14 } else { c.dy + 4 };
    let tol = (line_tol / 2).max(3);
    let baseline = c.bottom();

    let (current, lo, hi) = segment(text, baseline, tol, c.x, c.x)?;
    // The nearest lines above and below in this column.
    let (mut prev_bl, mut next_bl) = (None, None);
    for r in text.rects() {
        if r.right() < lo - COL_GAP || r.x > hi + COL_GAP {
            continue;
        }
        let bl = r.bottom();
        if bl < baseline - tol && prev_bl.is_none_or(|p| bl > p) {
            prev_bl = Some(bl);
        }
        if bl > baseline + tol && next_bl.is_none_or(|n| bl < n) {
            next_bl = Some(bl);
        }
    }
    let around = |bl: Option<i32>| {
        bl.and_then(|bl| segment(text, bl, tol, lo, hi))
            .map(|s| s.0)
            .unwrap_or_default()
    };
    let seq: Vec<usize> = around(prev_bl)
        .into_iter()
        .chain(current)
        .chain(around(next_bl))
        .collect();
    let ch = |k: usize| text.ch(seq[k]);
    let cursor_pos = seq.iter().position(|g| *g == cursor)?;

    // Out to "[" and "]" through list characters only.
    let mut open = None;
    for k in (0..=cursor_pos).rev() {
        match ch(k) {
            '[' => {
                open = Some(k);
                break;
            }
            ']' if k == cursor_pos => {}
            c if !list_char(c) => break,
            _ => {}
        }
    }
    let open = open?;
    let close = (open + 1..seq.len())
        .find(|k| ch(*k) == ']' || !list_char(ch(*k)))
        .filter(|k| ch(*k) == ']')?;
    if close <= open + 1 {
        return None;
    }
    if !(open..=close).any(|k| hits_glyph(text.rect(seq[k]), x, y)) {
        return None;
    }

    // The number nearest the pointer.
    let mut best = None;
    let mut best_dist = usize::MAX;
    let mut k = open + 1;
    while k < close {
        if !ch(k).is_ascii_digit() {
            k += 1;
            continue;
        }
        let start = k;
        let mut value = 0u32;
        while k < close && ch(k).is_ascii_digit() {
            value = (value * 10 + ch(k).to_digit(10).unwrap_or(0)).min(99_999);
            k += 1;
        }
        let end = k - 1;
        let dist = if cursor_pos < start {
            start - cursor_pos
        } else {
            cursor_pos.saturating_sub(end)
        };
        if (1..=9999).contains(&value) && dist < best_dist {
            best_dist = dist;
            best = Some(value);
        }
    }
    let number = best?;
    let (x0, y0, x1, y1) = seq[open..=close]
        .iter()
        .map(|g| text.rect(*g))
        .filter(|r| r.dx > 0 || r.dy > 0)
        .fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, r| {
            (
                b.0.min(r.x),
                b.1.min(r.y),
                b.2.max(r.right()),
                b.3.max(r.bottom()),
            )
        });
    let source = if x0 == i32::MAX {
        Rect::default()
    } else {
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    };
    Some(Numeric { number, source })
}

/// The reference entry "\[number\]": a "\[number\]" with clear space to its left on its line (the margin or a gutter, so
/// any column of a two-column list), not at `source`.
pub fn find_numeric_reference(
    text: &PageText,
    number: u32,
    source: Option<Rect>,
) -> Option<Anchor> {
    /// Wider than a space between words, narrower than a gutter or a hanging indent.
    const GAP: i32 = 12;
    if number == 0 {
        return None;
    }
    let chars = text.chars();
    for i in 0..chars.len() {
        if chars[i] != '[' {
            continue;
        }
        let digits = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let after = i + 1 + digits;
        if digits == 0 || chars.get(after) != Some(&']') {
            continue;
        }
        let value = chars[i + 1..after].iter().fold(0u32, |v, d| {
            v.saturating_mul(10)
                .saturating_add(d.to_digit(10).unwrap_or(0))
        });
        let r = text.rect(i);
        if value != number || source.is_some_and(|s| intersects(r, s)) {
            continue;
        }
        // A "[n]" in the middle of a line is a citation in the body.
        let y_tol = if r.dy > 6 { r.dy } else { 8 };
        let crowded = text
            .ink()
            .any(|(_, _, o)| (o.y - r.y).abs() <= y_tol && o.x < r.x && o.right() > r.x - GAP);
        if !crowded {
            return Some(Anchor::of(r));
        }
    }
    None
}
