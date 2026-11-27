//! From a hovered citation to the slice of a page worth showing: Enhanced's `RefHoverText.cpp` (finding the entry
//! by plain text, with a lookup cache) and `RefHoverShow.cpp` (choosing the region, and moving to the next page when
//! the entry starts there).

use std::collections::HashMap;

use ecclesia_mupdf::PageRect;

use super::cite::{self, Anchor};
use super::layout::{self, EntryRegion};
use super::{PageText, Rect, RectF};

/// Default maximum popover width as a percentage of the page width (SumatraPDF Enhanced default).
pub const DEFAULT_MAX_WIDTH_PCT: f32 = 90.0;
/// Minimum configurable maximum popover width percentage.
pub const MIN_MAX_WIDTH_PCT: f32 = 40.0;
/// Maximum configurable maximum popover width percentage.
pub const MAX_MAX_WIDTH_PCT: f32 = 100.0;

/// A two-dimensional pixel extent (`width` × `height`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent in device pixels.
    pub width: f32,
    /// Vertical extent in device pixels.
    pub height: f32,
}

impl Size {
    /// Creates a pixel size from `width` and `height`.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// A document's pages as the resolver reads them. The app reads them from MuPDF on a worker; tests make them up.
pub trait Pages {
    /// How many pages the document has.
    fn count(&self) -> usize;

    /// Page `page`'s text (counting from zero), as extracted: not yet [`PageText::prepared`].
    fn text(&mut self, page: usize) -> Option<PageText>;

    /// Page `page`'s box, in points.
    fn mediabox(&mut self, page: usize) -> Option<RectF>;
}

/// A place in the document: page `page` (counting from zero) at (`x`, `y`) in its points. A negative `y` means the
/// link named only the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    /// The page, counting from zero.
    pub page: usize,
    /// Left, in points.
    pub x: f32,
    /// Top, in points; negative when unknown.
    pub y: f32,
}

/// A citation found by its text, with no link: where it is and where its entry is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    /// The citation's own glyphs, which tell two citations on one line apart.
    pub source: Rect,
    /// The start of its entry in the references.
    pub target: Place,
}

/// What a popover shows: one or two slices of one page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    /// The page, counting from zero.
    pub page: usize,
    /// The slice, in its points.
    pub region: RectF,
    /// A second slice stacked below the first, when an entry wraps to the next column.
    pub continuation: Option<RectF>,
}

impl Shown {
    /// The size of both slices stacked, in points: the wider width and the summed height.
    pub fn extent(&self) -> (f32, f32) {
        self.continuation
            .map_or((self.region.dx, self.region.dy), |c| {
                (self.region.dx.max(c.dx), self.region.dy + c.dy)
            })
    }
}

/// What a lookup was for: an author and year, or a reference number. Enhanced keyed numbers as the name `[N]`; an
/// enum keeps the two apart without the trick.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Name(String, u16),
    Number(u32),
}

/// Lookups already made in one document, found or not, so hovering the same citation again costs nothing. Keyed by
/// the citation's own place too: the walk stops at the citing page, which differs between citations.
#[derive(Debug, Clone, Default)]
pub struct Lookups {
    done: HashMap<(Key, usize, Rect), Option<Place>>,
}

impl Lookups {
    /// No lookups yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The citation at (`x`, `y`) on page `page` and its entry, found by text alone: a numeric citation first, as a
    /// pointer inside brackets is the surer sign, then an author and year.
    pub fn find(&mut self, pages: &mut impl Pages, page: usize, x: i32, y: i32) -> Option<Found> {
        if page >= pages.count() {
            return None;
        }
        let text = pages.text(page)?;
        if let Some(c) = cite::detect_numeric(&text, x, y) {
            let key = Key::Number(c.number);
            let target = self.lookup(pages, key, page, c.source, |t, skip| {
                cite::find_numeric_reference(t, c.number, skip)
            });
            if let Some(target) = target {
                return Some(Found {
                    source: c.source,
                    target,
                });
            }
        }
        let c = cite::detect_author_year(&text, x, y)?;
        let target = self.find_name(pages, page, c.source, &c.surname, c.year)?;
        Some(Found {
            source: c.source,
            target,
        })
    }

    /// The entry of `surname` and `year`, or, for a name of several words, of its longest word that has one: the
    /// references may list "Oude Vrielink" under "Vrielink", and extraction may split "Bashab" into "Bash b".
    fn find_name(
        &mut self,
        pages: &mut impl Pages,
        page: usize,
        source: Rect,
        surname: &str,
        year: u16,
    ) -> Option<Place> {
        let mut lookup = |this: &mut Self, name: &str| {
            let key = Key::Name(name.to_owned(), year);
            this.lookup(pages, key, page, source, |t, skip| {
                cite::find_surname(t, name, year, skip)
            })
        };
        if let Some(found) = lookup(self, surname) {
            return Some(found);
        }
        if !surname.contains(' ') {
            return None;
        }
        let mut parts: Vec<&str> = surname
            .split(' ')
            .filter(|p| p.chars().count() >= 2)
            .take(8)
            .collect();
        // Longest first; a stable sort keeps equal lengths in reading order, as Enhanced's selection sort does.
        parts.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
        parts.into_iter().find_map(|p| lookup(self, p))
    }

    /// A cached lookup, or a fresh walk from the last page back to the citing page, skipping the citation itself.
    fn lookup(
        &mut self,
        pages: &mut impl Pages,
        key: Key,
        page: usize,
        source: Rect,
        find: impl Fn(&PageText, Option<Rect>) -> Option<Anchor>,
    ) -> Option<Place> {
        let entry = (key, page, source);
        if let Some(done) = self.done.get(&entry) {
            return *done;
        }
        let found = (page..pages.count()).rev().find_map(|p| {
            let text = pages.text(p)?;
            let skip = (p == page).then_some(source);
            find(&text, skip).map(|a| Place {
                page: p,
                x: a.x,
                y: a.y,
            })
        });
        self.done.insert(entry, found);
        found
    }
}

/// How many words of the citation to try on the target page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// Every candidate word, best first, until one is found.
    Any,
    /// Only the best: a common word found elsewhere would mislead.
    Best,
}

/// Where on `dest` the citation at `source` on `src` points, found by its text: the top of the line that starts with
/// its best word (a parenthesized label such as "(14)" first, then the longest), or of any line that has it.
pub fn dest_y_from_source(
    src: &PageText,
    source: RectF,
    dest: &PageText,
    matching: Match,
) -> Option<f32> {
    dest_point_from_source(src, source, dest, matching).map(|(_, y)| y)
}

/// Like [`dest_y_from_source`], returning both `(x, y)` in page points so multi-column targets
/// keep their exact column coordinate instead of falling back to the left column.
fn dest_point_from_source(
    src: &PageText,
    source: RectF,
    dest: &PageText,
    matching: Match,
) -> Option<(f32, f32)> {
    if source.is_empty() || src.is_empty() || dest.is_empty() {
        return None;
    }
    let words = source_words(src, source);
    let best = |word: &[char]| {
        let mut at_start: Option<Rect> = None;
        let mut anywhere: Option<Rect> = None;
        for i in (0..dest.len()).filter(|i| word_at(dest, *i, word)) {
            let r = dest.rect(i);
            let slot = if starts_line(dest, i) {
                &mut at_start
            } else {
                &mut anywhere
            };
            if slot.is_none_or(|s| r.x < s.x) {
                *slot = Some(r);
            }
        }
        at_start.or(anywhere).map(|r| (r.x as f32, r.y as f32))
    };
    match matching {
        Match::Any => words.iter().find_map(|w| best(w)),
        Match::Best => words.first().and_then(|w| best(w)),
    }
}

/// Raw characters under `source` on `src` (grown by 2 pt), capped at 511 glyphs.
fn source_raw_chars(src: &PageText, source: RectF) -> Vec<char> {
    const MAX_GLYPHS: usize = 511;
    // Enhanced truncates the float box to whole points before growing it.
    let (l, t) = (source.x as i32 - 2, source.y as i32 - 2);
    let (r, b) = (source.right() as i32 + 2, source.bottom() as i32 + 2);
    src.chars()
        .iter()
        .zip(src.rects())
        .filter(|(_, g)| g.right() >= l && g.x <= r && g.bottom() >= t && g.y <= b)
        .map(|(c, _)| *c)
        .take(MAX_GLYPHS)
        .collect()
}

/// The citation's words (runs of two or more ASCII letters and digits under `source`, grown by 2 pt), best first:
/// parenthesized ones, then longer ones. At most 16, from the first 511 glyphs, as in Enhanced.
fn source_words(src: &PageText, source: RectF) -> Vec<Vec<char>> {
    const MAX_WORDS: usize = 16;
    let raw = source_raw_chars(src, source);

    let mut words: Vec<(Vec<char>, bool)> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if !raw[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < raw.len() && raw[i].is_ascii_alphanumeric() {
            i += 1;
        }
        if i - start >= 2 && words.len() < MAX_WORDS {
            let flanked = start > 0 && raw[start - 1] == '(' && raw.get(i) == Some(&')');
            words.push((raw[start..i].to_vec(), flanked));
        }
    }
    words.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.len().cmp(&a.0.len())));
    words.into_iter().map(|(w, _)| w).collect()
}

/// Whether `word` is on `text` at `i` as a whole word, ignoring ASCII case.
fn word_at(text: &PageText, i: usize, word: &[char]) -> bool {
    let chars = text.chars();
    let end = i + word.len();
    end <= chars.len()
        && chars[i..end]
            .iter()
            .zip(word)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        && (i == 0 || !chars[i - 1].is_ascii_alphanumeric())
        && chars.get(end).is_none_or(|c| !c.is_ascii_alphanumeric())
}

/// Whether nothing is inked immediately left of glyph `i` on its line (the page margin or a
/// multi-column gutter, so right-column lines also qualify even with a 10 pt LaTeX `\columnsep`).
fn starts_line(text: &PageText, i: usize) -> bool {
    const COL_GAP: i32 = 9;
    let r = text.rect(i);
    let y_tol = (r.dy / 2).max(3);
    !text.ink().any(|(j, _, g)| {
        j != i && (g.y - r.y).abs() <= y_tol && g.x < r.x && g.right() > r.x - COL_GAP
    })
}

/// Extracts all candidate `(x, y)` anchors on `dest` near `target_y` (within `NEAR_Y_TOL` pt)
/// matching the hovered citation or cross-reference under `source` on `src`. Handles:
/// - bracketed or bare numeric references (`[5]`, `[26]`, `26` -> `[26]` at a column start),
/// - equation numbers (`(3)`, `(2.1)` -> `(3)` / `(2.1)` near `target_y`),
/// - words/labels (`Smith`, `Figure`, `Table`, `Section`, `Foo10`) at a column start.
fn anchors_near_y(
    src: &PageText,
    source: RectF,
    dest: &PageText,
    target_y: f32,
) -> Vec<(f32, f32)> {
    const NEAR_Y_TOL: i32 = 55;
    if source.is_empty() || src.is_empty() || dest.is_empty() || !target_y.is_finite() {
        return Vec::new();
    }
    let ty = target_y as i32;
    let raw = source_raw_chars(src, source);
    if raw.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(f32, f32)> = Vec::new();

    // 1. Numeric reference "[N]" (including single-digit references "[1]".."[9]"): match a
    //    column-starting "[N]" on `dest` near `target_y`.
    let trimmed: String = raw
        .iter()
        .copied()
        .collect::<String>()
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | ',' | '.' | ';'))
        .to_owned();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        let needle: Vec<char> = format!("[{trimmed}]").chars().collect();
        for i in 0..dest.len() {
            let r = dest.rect(i);
            if (r.y - ty).abs() > NEAR_Y_TOL || !starts_line(dest, i) {
                continue;
            }
            let end = i + needle.len();
            if end <= dest.len() && dest.chars()[i..end] == needle[..] {
                hits.push((r.x as f32, r.y as f32));
            }
        }
        if !hits.is_empty() {
            return hits;
        }
    }

    // 2. Equation label "(N)" or "(N.M)" in source text (e.g. "(3)", "Eq. (2.1)"): match "(N)"
    //    on `dest` near `target_y`.
    let raw_str: String = raw.iter().copied().collect();
    if let Some(open) = raw_str.find('(')
        && let Some(rel_close) = raw_str[open + 1..].find(')')
    {
        let inner = &raw_str[open + 1..open + 1 + rel_close];
        if !inner.is_empty()
            && inner.len() <= 6
            && inner.chars().all(|c| c.is_ascii_digit() || c == '.')
            && inner.chars().any(|c| c.is_ascii_digit())
        {
            let needle: Vec<char> = format!("({inner})").chars().collect();
            for i in 0..dest.len() {
                let r = dest.rect(i);
                if (r.y - ty).abs() > NEAR_Y_TOL {
                    continue;
                }
                let end = i + needle.len();
                if end <= dest.len() && dest.chars()[i..end] == needle[..] {
                    hits.push((r.x as f32, r.y as f32));
                }
            }
            if !hits.is_empty() {
                return hits;
            }
        }
    }

    // 3. Best word(s) from `source_words` matching at a line/column start near `target_y`
    //    (e.g. author surname, `[Foo10]`, `Figure`, `Table`, `Section`).
    for word in source_words(src, source) {
        for i in 0..dest.len() {
            let r = dest.rect(i);
            if (r.y - ty).abs() > NEAR_Y_TOL || !word_at(dest, i, &word) {
                continue;
            }
            if starts_line(dest, i)
                || (i > 0 && dest.chars()[i - 1] == '[' && starts_line(dest, i - 1))
            {
                let anchor_r = if i > 0 && dest.chars()[i - 1] == '[' {
                    dest.rect(i - 1)
                } else {
                    r
                };
                hits.push((anchor_r.x as f32, anchor_r.y as f32));
            }
        }
        if !hits.is_empty() {
            return hits;
        }
    }
    hits
}

/// Refines `(x, y)` on a multi-column destination page when `x` is unspecified/at the page origin
/// (e.g. `/FitH`, `/XYZ null y`, `/XYZ 0 y`) OR when `x` lands in the wrong column while the
/// opposite column holds the sole matching anchor near `y`.
fn refine_two_column_target(
    src: &PageText,
    source: RectF,
    dest: &PageText,
    mediabox: RectF,
    x: f32,
    y: f32,
) -> (f32, f32) {
    let Some(cols) = layout::detect_page_columns(dest, mediabox) else {
        return (x, y);
    };
    let hits = anchors_near_y(src, source, dest, y);
    if hits.is_empty() {
        return (x, y);
    }
    let unspecified_x = !x.is_finite() || x <= mediabox.x + 1.0;
    if unspecified_x {
        if let Some(&(hx, hy)) = hits
            .iter()
            .min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs()))
        {
            return (hx, hy);
        }
        return (x, y);
    }

    let in_left = (x as i32) < cols.gutter_mid;
    let same_col_hit = hits
        .iter()
        .any(|&(hx, _)| ((hx as i32) < cols.gutter_mid) == in_left);
    if same_col_hit {
        return (x, y);
    }
    // No matching anchor in `x`'s column near `y`, while the opposite column has an exact match:
    // snap to the closest opposite-column match (rescues PDFs that emit a left-margin x for
    // right-column anchors).
    if let Some(&(hx, hy)) = hits
        .iter()
        .min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs()))
    {
        return (hx, hy);
    }
    (x, y)
}

/// What to show for `target`, cited at `source` on page `src_page`: an equation, a reference entry or a strip of the
/// page. When the entry starts on the next page (the link points at the foot of a page), it moves there.
pub fn shown(
    pages: &mut impl Pages,
    src_page: usize,
    source: RectF,
    target: Place,
) -> Option<Shown> {
    let Place { page, mut x, mut y } = target;
    let mediabox = pages.mediabox(page).filter(|m| !m.is_empty())?;
    let src = pages.text(src_page).unwrap_or_default();
    let dest = pages.text(page).unwrap_or_default();
    let resolve =
        |dest: &PageText, matching: Match| dest_point_from_source(&src, source, dest, matching);

    if layout::should_resolve_dest_y(mediabox, y) {
        y = 0.0;
        if let Some((found_x, found_y)) = resolve(&dest, Match::Any) {
            x = found_x.max(0.0);
            y = found_y;
        }
    }

    let prepared = dest.prepared();
    let (rx, ry) = refine_two_column_target(&src, source, &prepared, mediabox, x, y);
    x = rx;
    y = ry;

    let first = detect_region(&prepared, mediabox, x, y, Missing::Empty);
    let search_next = match first {
        None => true,
        Some(_) => {
            layout::should_search_next_page(mediabox, y) && resolve(&dest, Match::Best).is_none()
        }
    };
    if search_next && page + 1 < pages.count() {
        let next = pages.text(page + 1).unwrap_or_default();
        if let Some((next_x, next_y)) = resolve(&next, Match::Best) {
            let mediabox = pages.mediabox(page + 1).filter(|m| !m.is_empty())?;
            let found = detect_region(
                &next.prepared(),
                mediabox,
                next_x.max(0.0),
                next_y,
                Missing::Landscape,
            )?;
            return Some(Shown {
                page: page + 1,
                region: found.region,
                continuation: found.continuation,
            });
        }
    }
    let found = first.or_else(|| detect_region(&prepared, mediabox, x, y, Missing::Landscape))?;
    Some(Shown {
        page,
        region: found.region,
        continuation: found.continuation,
    })
}

/// What to fall back to when the target is neither an equation nor an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// Nothing: the caller still has the next page to try.
    Empty,
    /// A strip of the page below the target.
    Landscape,
}

fn detect_region(
    text: &PageText,
    mediabox: RectF,
    x: f32,
    y: f32,
    missing: Missing,
) -> Option<EntryRegion> {
    if let Some(region) =
        layout::detect_equation_box_at(text, mediabox, x, y).filter(|r| !r.is_empty())
    {
        return Some(EntryRegion {
            region,
            continuation: None,
        });
    }
    if let Some(entry) =
        layout::detect_entry_box(text, mediabox, x, y).filter(|e| !e.region.is_empty())
    {
        return Some(entry);
    }
    (missing == Missing::Landscape).then(|| EntryRegion {
        region: layout::landscape_box_at(mediabox, x, y, text),
        continuation: None,
    })
}

/// Popover sizes, in device pixels, from Enhanced (`RefHover.h`), before the screen's scale.
pub mod size {
    /// The zoom a popover starts at when the page's is unknown.
    pub const RENDER_ZOOM: f32 = 1.5;
    /// The widest a popover gets, unless the screen is wider.
    pub const MAX_WIDTH: f32 = 1200.0;
    /// The tallest a short popover gets.
    pub const MAX_HEIGHT: f32 = 600.0;
    /// The frame around the slice.
    pub const BORDER: f32 = 4.0;
    /// The gap kept between the pointer and the popover.
    pub const POINTER_GAP: f32 = 30.0;
    /// The smallest zoom a popover shows its slice at.
    pub const MIN_ZOOM: f32 = 0.4;
    /// The largest zoom the reader can scroll a popover to.
    pub const MAX_ZOOM: f32 = 3.0;
    /// One zoom step.
    pub const ZOOM_STEP: f32 = 1.15;
}

/// Where a popover goes: the screen (or window) it must fit in and the page it was raised over, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Room {
    /// The usable width of the screen or window.
    pub screen_width: f32,
    /// The citing page on screen: left, top, width, height. Zero sizes when unknown.
    pub page: RectF,
    /// The pointer's height on screen.
    pub pointer_y: f32,
    /// Device pixels per layout pixel.
    pub scale: f32,
    /// Maximum popover width as a percentage of the page width (`40.0..=100.0`, default `90.0`).
    pub max_width_pct: f32,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            screen_width: 0.0,
            page: RectF::default(),
            pointer_y: 0.0,
            scale: 1.0,
            max_width_pct: DEFAULT_MAX_WIDTH_PCT,
        }
    }
}

impl Room {
    /// Returns the clamped maximum width fraction in `0.40..=1.00`, falling back to
    /// [`DEFAULT_MAX_WIDTH_PCT`] when `max_width_pct` is non-finite or non-positive.
    #[must_use]
    pub fn max_width_fraction(&self) -> f32 {
        let pct = if self.max_width_pct.is_finite() && self.max_width_pct > 0.0 {
            self.max_width_pct
        } else {
            DEFAULT_MAX_WIDTH_PCT
        };
        pct.clamp(MIN_MAX_WIDTH_PCT, MAX_MAX_WIDTH_PCT) / 100.0
    }
}

/// The zoom (device pixels per point) to draw `shown` at: the page's own zoom, shrunk until both slices fit the
/// popover's room (never wider than `room.max_width_fraction()` of either the page or the screen), and never below
/// [`size::MIN_ZOOM`].
pub fn zoom(shown: &Shown, page_zoom: f32, room: Room) -> f32 {
    let (dx, dy) = shown.extent();
    let s = room.scale;
    let frac = room.max_width_fraction();
    let width_cap = {
        let screen_cap = if room.screen_width > 0.0 {
            room.screen_width * frac
        } else {
            size::MAX_WIDTH * s
        };
        if room.page.dx > 0.0 {
            screen_cap.min(room.page.dx * frac)
        } else {
            screen_cap
        }
    };
    let height_cap = if dy > 250.0 && room.page.dy > 0.0 {
        let gap = size::POINTER_GAP * s;
        let above = room.pointer_y - room.page.y - gap;
        let below = room.page.bottom() - room.pointer_y - gap;
        (room.page.dy * 0.75).max(above.max(below).max(0.0))
    } else if room.page.dy > 0.0 {
        (size::MAX_HEIGHT * s).min(room.page.dy * 0.45)
    } else {
        size::MAX_HEIGHT * s
    };
    let border = 2.0 * size::BORDER * s;
    let (avail_w, avail_h) = (
        (width_cap - border).max(1.0),
        (height_cap - border).max(1.0),
    );
    let mut zoom = if page_zoom > 0.0 {
        page_zoom
    } else {
        size::RENDER_ZOOM
    };
    if dy > 0.0 && dy * zoom > avail_h {
        zoom = avail_h / dy;
    }
    if dx > 0.0 && dx * zoom > avail_w {
        zoom = avail_w / dx;
    }
    zoom.max(size::MIN_ZOOM)
}

/// Computes the PDF-point region for a user-resized popover of `content_size_px` rendered at
/// `target_zoom` (device pixels per point), anchored at `origin`'s top-left and clamped to
/// `page_bounds`.
///
/// Modeled on SumatraPDF Enhanced's `RefHoverSizedRegion`: invalid or non-positive inputs return
/// an empty [`PageRect`].
#[must_use]
pub fn resized_region(
    origin: PageRect,
    target_zoom: f32,
    content_size_px: Size,
    page_bounds: PageRect,
) -> PageRect {
    let page_w = page_bounds.x1 - page_bounds.x0;
    let page_h = page_bounds.y1 - page_bounds.y0;
    if !target_zoom.is_finite()
        || target_zoom <= 0.0
        || !content_size_px.width.is_finite()
        || !content_size_px.height.is_finite()
        || content_size_px.width <= 0.0
        || content_size_px.height <= 0.0
        || !page_w.is_finite()
        || !page_h.is_finite()
        || page_w <= 0.0
        || page_h <= 0.0
    {
        return PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        };
    }
    let min_span_x = 1.0_f32.min(page_w);
    let min_span_y = 1.0_f32.min(page_h);
    let x0 = origin.x0.clamp(page_bounds.x0, page_bounds.x1 - min_span_x);
    let y0 = origin.y0.clamp(page_bounds.y0, page_bounds.y1 - min_span_y);
    let dx = (content_size_px.width / target_zoom).min(page_bounds.x1 - x0);
    let dy = (content_size_px.height / target_zoom).min(page_bounds.y1 - y0);
    PageRect {
        x0,
        y0,
        x1: x0 + dx,
        y1: y0 + dy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_shown(dx: f32, dy: f32) -> Shown {
        Shown {
            page: 0,
            region: RectF::new(0.0, 0.0, dx, dy),
            continuation: None,
        }
    }

    #[test]
    fn max_width_fraction_clamps_and_defaults() {
        let with_pct = |pct| Room {
            max_width_pct: pct,
            ..Room::default()
        };
        assert!((Room::default().max_width_fraction() - 0.90).abs() < 1e-6);
        assert!((with_pct(90.0).max_width_fraction() - 0.90).abs() < 1e-6);
        assert!((with_pct(60.0).max_width_fraction() - 0.60).abs() < 1e-6);
        assert!((with_pct(100.0).max_width_fraction() - 1.00).abs() < 1e-6);
        assert!((with_pct(15.0).max_width_fraction() - 0.40).abs() < 1e-6);
        assert!((with_pct(180.0).max_width_fraction() - 1.00).abs() < 1e-6);
        assert!((with_pct(0.0).max_width_fraction() - 0.90).abs() < 1e-6);
        assert!((with_pct(-10.0).max_width_fraction() - 0.90).abs() < 1e-6);
        assert!((with_pct(f32::NAN).max_width_fraction() - 0.90).abs() < 1e-6);
    }

    #[test]
    fn zoom_caps_at_default_90_percent_of_page_width() {
        let shown = sample_shown(400.0, 100.0);
        let room = Room {
            screen_width: 1920.0,
            page: RectF::new(0.0, 0.0, 900.0, 1200.0),
            pointer_y: 600.0,
            scale: 1.0,
            max_width_pct: DEFAULT_MAX_WIDTH_PCT,
        };
        let z = zoom(&shown, 3.0, room);
        let expected = (900.0 * 0.90 - 8.0) / 400.0;
        assert!((z - expected).abs() < 1e-4, "got {z}, expected {expected}");
    }

    #[test]
    fn zoom_respects_custom_max_width_pct_and_screen_width() {
        let shown = sample_shown(400.0, 100.0);
        let room_60 = Room {
            screen_width: 1920.0,
            page: RectF::new(0.0, 0.0, 900.0, 1200.0),
            pointer_y: 600.0,
            scale: 1.0,
            max_width_pct: 60.0,
        };
        let z60 = zoom(&shown, 3.0, room_60);
        assert!((z60 - (900.0 * 0.60 - 8.0) / 400.0).abs() < 1e-4);

        let room_100 = Room {
            max_width_pct: 100.0,
            ..room_60
        };
        let z100 = zoom(&shown, 3.0, room_100);
        assert!((z100 - (900.0 - 8.0) / 400.0).abs() < 1e-4);

        // When screen is narrower than page, screen_width * frac caps width too.
        let narrow_screen = Room {
            screen_width: 600.0,
            page: RectF::new(0.0, 0.0, 1200.0, 1400.0),
            pointer_y: 600.0,
            scale: 1.0,
            max_width_pct: 90.0,
        };
        let z_narrow = zoom(&shown, 3.0, narrow_screen);
        assert!((z_narrow - (600.0 * 0.90 - 8.0) / 400.0).abs() < 1e-4);
    }

    #[test]
    fn resized_region_scales_content_and_clamps_to_page_bounds() {
        let page = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 600.0,
            y1: 800.0,
        };
        let origin = PageRect {
            x0: 50.0,
            y0: 100.0,
            x1: 250.0,
            y1: 200.0,
        };
        // Normal resize at 2x zoom: 400x300 px -> 200x150 pt from (50, 100).
        let grown = resized_region(origin, 2.0, Size::new(400.0, 300.0), page);
        assert_eq!(
            grown,
            PageRect {
                x0: 50.0,
                y0: 100.0,
                x1: 250.0,
                y1: 250.0,
            }
        );

        // Resize past the right/bottom page edge clamps to page_bounds.
        let clamped = resized_region(origin, 1.0, Size::new(900.0, 1000.0), page);
        assert_eq!(
            clamped,
            PageRect {
                x0: 50.0,
                y0: 100.0,
                x1: 600.0,
                y1: 800.0,
            }
        );

        // Origin outside page_bounds clamps inside with at least 1 pt remaining.
        let outside_origin = PageRect {
            x0: -40.0,
            y0: 950.0,
            x1: 10.0,
            y1: 990.0,
        };
        let shifted = resized_region(outside_origin, 1.0, Size::new(100.0, 100.0), page);
        assert_eq!(shifted.x0, 0.0);
        assert_eq!(shifted.y0, 799.0);
        assert_eq!(shifted.x1, 100.0);
        assert_eq!(shifted.y1, 800.0);

        // Invalid inputs return an empty rect.
        let invalid = resized_region(origin, 0.0, Size::new(200.0, 100.0), page);
        assert_eq!(invalid.x1 - invalid.x0, 0.0);
        assert_eq!(invalid.y1 - invalid.y0, 0.0);
    }
}
