//! From a hovered citation to the slice of a page worth showing: Enhanced's `RefHoverText.cpp` (finding the entry
//! by plain text, with a lookup cache) and `RefHoverShow.cpp` (choosing the region, and moving to the next page when
//! the entry starts there).

use std::collections::HashMap;

use super::cite::{self, Anchor};
use super::layout::{self, EntryRegion};
use super::{PageText, Rect, RectF};

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
        at_start.or(anywhere).map(|r| r.y as f32)
    };
    match matching {
        Match::Any => words.iter().find_map(|w| best(w)),
        Match::Best => words.first().and_then(|w| best(w)),
    }
}

/// The citation's words (runs of two or more ASCII letters and digits under `source`, grown by 2 pt), best first:
/// parenthesized ones, then longer ones. At most 16, from the first 511 glyphs, as in Enhanced.
fn source_words(src: &PageText, source: RectF) -> Vec<Vec<char>> {
    const MAX_GLYPHS: usize = 511;
    const MAX_WORDS: usize = 16;
    // Enhanced truncates the float box to whole points before growing it.
    let (l, t) = (source.x as i32 - 2, source.y as i32 - 2);
    let (r, b) = (source.right() as i32 + 2, source.bottom() as i32 + 2);
    let raw: Vec<char> = src
        .chars()
        .iter()
        .zip(src.rects())
        .filter(|(_, g)| g.right() >= l && g.x <= r && g.bottom() >= t && g.y <= b)
        .map(|(c, _)| *c)
        .take(MAX_GLYPHS)
        .collect();

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

/// Whether nothing is inked left of glyph `i` on its line (the glyphs with its top).
fn starts_line(text: &PageText, i: usize) -> bool {
    let r = text.rect(i);
    !text
        .ink()
        .any(|(j, _, g)| j != i && g.y == r.y && g.x < r.x)
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
        |dest: &PageText, matching: Match| dest_y_from_source(&src, source, dest, matching);

    if layout::should_resolve_dest_y(mediabox, y) {
        y = 0.0;
        if let Some(found) = resolve(&dest, Match::Any) {
            y = found;
            x = x.max(0.0);
        }
    }

    let prepared = dest.prepared();
    let first = detect_region(&prepared, mediabox, x, y, Missing::Empty);
    let search_next = match first {
        None => true,
        Some(_) => {
            layout::should_search_next_page(mediabox, y) && resolve(&dest, Match::Best).is_none()
        }
    };
    if search_next && page + 1 < pages.count() {
        let next = pages.text(page + 1).unwrap_or_default();
        if let Some(next_y) = resolve(&next, Match::Best) {
            let mediabox = pages.mediabox(page + 1).filter(|m| !m.is_empty())?;
            let found = detect_region(
                &next.prepared(),
                mediabox,
                x.max(0.0),
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
    if let Some(region) = layout::detect_equation_box(text, mediabox, y).filter(|r| !r.is_empty()) {
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
        region: layout::landscape_box(mediabox, y, text),
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
}

/// The zoom (device pixels per point) to draw `shown` at: the page's own zoom, shrunk until both slices fit the
/// popover's room, and never below [`size::MIN_ZOOM`].
pub fn zoom(shown: &Shown, page_zoom: f32, room: Room) -> f32 {
    let (dx, dy) = shown.extent();
    let s = room.scale;
    let width_cap = {
        let wide = (room.screen_width * 0.95).max(size::MAX_WIDTH * s);
        if room.page.dx > 0.0 {
            wide.min(room.page.dx * 0.95)
        } else {
            wide
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
    let (avail_w, avail_h) = (width_cap - border, height_cap - border);
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
