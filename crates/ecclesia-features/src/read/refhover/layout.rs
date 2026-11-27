//! What to show of the target page: one bibliography entry, a labelled equation's line, or a landscape strip below
//! the target. Ported from SumatraPDF Enhanced's `src/RefHoverDetect.cpp`; the constants are its constants.
//!
//! Every detector takes the page's glyphs (after [`PageText::prepared`]), its media box and the target point in page
//! space, and returns a region in page space clipped to the media box.

use super::{PageText, Rect, RectF, fold, is_space};

/// Room left above the target when a strip starts there.
const ANCHOR_TOP_MARGIN: f32 = 6.0;
/// Padding around a detected entry.
const ENTRY_PAD: f32 = 6.0;
/// A target this far down its page may well be an entry that starts on the next page.
const LATE_PAGE_START_RATIO: f32 = 0.70;

/// Words that start a caption, lowercase: "Figure 2.1", "Tableau 3". Add a language by adding its words.
const CAPTION_WORDS: &[&str] = &[
    "figure",
    "table",
    "listing",
    "algorithm", // en
    "abbildung",
    "tabelle",
    "algorithmus", // de
    "figura",
    "algoritmo", // es, it, pt
    "tabla",     // es
    "tabella",   // it
    "tabela",    // pt
    "tableau",
    "algorithme", // fr
];

/// Words that start a heading or caption, lowercase: a superset of [`CAPTION_WORDS`].
const HEADING_WORDS: &[&str] = &[
    "figure",
    "table",
    "listing",
    "section",
    "chapter",
    "algorithm", // en
    "abbildung",
    "tabelle",
    "abschnitt",
    "kapitel",
    "algorithmus", // de
    "figura",
    "algoritmo",
    "capítulo", // es, it, pt
    "tabla",
    "sección", // es
    "tabella",
    "sezione",
    "capitolo", // it
    "tabela",
    "seção",
    "secção", // pt
    "tableau",
    "chapitre",
    "algorithme", // fr
];

/// Whether a target this far down its page should also be looked for on the next page.
pub fn should_search_next_page(mediabox: RectF, dest_y: f32) -> bool {
    mediabox.dy > 0.0 && dest_y >= mediabox.dy * LATE_PAGE_START_RATIO
}

/// The part of the page from an explicit target point to the bottom right, with a target outside the page or not a
/// number brought onto it. `None` for a page with no area.
pub fn destination_region(mediabox: RectF, dest_x: f32, dest_y: f32) -> Option<RectF> {
    if mediabox.dx <= 0.0 || mediabox.dy <= 0.0 {
        return None;
    }
    let x = if dest_x.is_finite() {
        dest_x
    } else {
        mediabox.x
    };
    let y = if dest_y.is_finite() {
        dest_y
    } else {
        mediabox.y
    };
    let x = limit(x, mediabox.x, mediabox.right() - mediabox.dx.min(1.0));
    let y = limit(y, mediabox.y, mediabox.bottom() - mediabox.dy.min(1.0));
    Some(RectF::new(
        x,
        y,
        mediabox.right() - x,
        mediabox.bottom() - y,
    ))
}

/// Whether a link's target height is unusable (not a number, or off the page), so the entry must be found by its
/// text instead.
pub fn should_resolve_dest_y(mediabox: RectF, dest_y: f32) -> bool {
    !dest_y.is_finite() || dest_y < mediabox.y || dest_y >= mediabox.bottom()
}

/// Where on a popover's frame a point is, for resizing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    /// Outside the popover.
    Outside,
    /// Inside, away from the edges.
    Inside,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
}

/// Which edge of `window` the point is on, with edges `grip` wide (at most half the window, at least 1).
pub fn edge_hit(window: Rect, x: i32, y: i32, grip: i32) -> Edge {
    if !window.contains(x, y) {
        return Edge::Outside;
    }
    let grip = grip.min(window.dx.min(window.dy) / 2).max(1);
    let left = x < window.x + grip;
    let right = x >= window.right() - grip;
    let top = y < window.y + grip;
    let bottom = y >= window.bottom() - grip;
    match (top, bottom, left, right) {
        (true, _, true, _) => Edge::TopLeft,
        (true, _, _, true) => Edge::TopRight,
        (true, ..) => Edge::Top,
        (_, true, true, _) => Edge::BottomLeft,
        (_, true, _, true) => Edge::BottomRight,
        (_, true, ..) => Edge::Bottom,
        (.., true, _) => Edge::Left,
        (.., true) => Edge::Right,
        _ => Edge::Inside,
    }
}

/// The page region a popover of `content` pixels shows at `zoom`, starting at `region`'s top left (brought onto the
/// page) and cut at the page's edges. `None` for a bad zoom, size or page.
pub fn sized_region(page: RectF, region: RectF, content: (i32, i32), zoom: f32) -> Option<RectF> {
    let (width, height) = content;
    if !zoom.is_finite()
        || zoom <= 0.0
        || width <= 0
        || height <= 0
        || page.dx <= 0.0
        || page.dy <= 0.0
    {
        return None;
    }
    let x = limit(region.x, page.x, page.right() - page.dx.min(1.0));
    let y = limit(region.y, page.y, page.bottom() - page.dy.min(1.0));
    Some(RectF::new(
        x,
        y,
        (width as f32 / zoom).min(page.right() - x),
        (height as f32 / zoom).min(page.bottom() - y),
    ))
}

fn limit(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Whether the text at `idx` is the lowercase word `word`, case aside. With [`After::Digit`], spaces then a digit
/// must follow ("Figure 1.2"); with [`After::WordEnd`], no letter may follow ("Sections" isn't "section").
fn word_at(text: &PageText, idx: usize, word: &str, after: After) -> bool {
    let chars = text.chars();
    let n = word.chars().count();
    if idx + n > chars.len() || (after == After::Digit && idx + n + 1 >= chars.len()) {
        return false;
    }
    if !chars[idx..idx + n]
        .iter()
        .map(|c| fold(*c))
        .eq(word.chars())
    {
        return false;
    }
    let mut k = idx + n;
    match after {
        After::WordEnd => !chars.get(k).is_some_and(char::is_ascii_alphabetic),
        After::Digit => {
            while k < chars.len() && matches!(chars[k], ' ' | '\t') {
                k += 1;
            }
            chars.get(k).is_some_and(char::is_ascii_digit)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Digit,
    WordEnd,
}

/// Whether a caption label ("Figure 1.2", "Tableau 2") starts at `idx`, at a word boundary.
fn caption_label_at(text: &PageText, idx: usize) -> bool {
    if idx > 0 && text.ch(idx - 1).is_ascii_alphanumeric() {
        return false;
    }
    CAPTION_WORDS
        .iter()
        .any(|w| word_at(text, idx, w, After::Digit))
}

/// Cuts a box to the media box: a negative left or top moves to 0 (shrinking the box), and any overhang goes.
fn clip_to_mediabox(mut b: RectF, mediabox: RectF) -> RectF {
    if b.x < 0.0 {
        b.dx += b.x;
        b.x = 0.0;
    }
    if b.y < 0.0 {
        b.dy += b.y;
        b.y = 0.0;
    }
    if b.x + b.dx > mediabox.dx {
        b.dx = mediabox.dx - b.x;
    }
    if b.y + b.dy > mediabox.dy {
        b.dy = mediabox.dy - b.y;
    }
    b
}

fn padded(min_x: i32, min_y: i32, max_x: i32, max_y: i32, mediabox: RectF) -> RectF {
    clip_to_mediabox(
        RectF::new(
            min_x as f32 - ENTRY_PAD,
            min_y as f32 - ENTRY_PAD,
            (max_x - min_x) as f32 + 2.0 * ENTRY_PAD,
            (max_y - min_y) as f32 + 2.0 * ENTRY_PAD,
        ),
        mediabox,
    )
}

/// Gives every glyph its line's top and height. Glyph boxes are tight to the ink, so one line's glyphs have
/// different tops (a full stop sits far below a capital), and a stray low glyph can land in the next line's band.
/// The baseline (bottom) is steady across a line, so glyphs are grouped by baseline, within 4 pt.
pub fn normalize_glyph_lines(rects: &[Rect]) -> Vec<Rect> {
    const BASELINE_TOL: i32 = 4;
    const MAX_LINES: usize = 4096;
    struct Line {
        baseline: i32,
        top: i32,
        bottom: i32,
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut ids = Vec::with_capacity(rects.len());
    for r in rects {
        let baseline = r.bottom();
        let mut best = None;
        let mut best_dist = BASELINE_TOL + 1;
        for (l, line) in lines.iter().enumerate() {
            let dist = (baseline - line.baseline).abs();
            if dist < best_dist {
                best_dist = dist;
                best = Some(l);
            }
        }
        let id = match best {
            Some(l) => {
                lines[l].top = lines[l].top.min(r.y);
                lines[l].bottom = lines[l].bottom.max(baseline);
                l
            }
            None if lines.len() < MAX_LINES => {
                lines.push(Line {
                    baseline,
                    top: r.y,
                    bottom: baseline,
                });
                lines.len() - 1
            }
            // A page with more lines than this is not text; fold the rest into the first.
            None => 0,
        };
        ids.push(id);
    }
    rects
        .iter()
        .zip(ids)
        .map(|(r, l)| Rect {
            y: lines[l].top,
            dy: lines[l].bottom - lines[l].top,
            ..*r
        })
        .collect()
}

/// Drops a diagonal "draft" watermark's glyphs: much taller than body text (1.5 times the commonest height) and each
/// nearly alone on its baseline, where a heading is a row of glyphs of one size. Run before
/// [`normalize_glyph_lines`], which could fold a watermark glyph into a body line.
pub fn strip_watermark_glyphs(text: &PageText) -> PageText {
    const MAX_HISTOGRAM_HEIGHT: i32 = 4096;
    const MIN_BODY_DY: i32 = 4;
    const BASELINE_TOL: i32 = 4;
    const MIN_ROW_GLYPHS: usize = 3;

    let max_dy = text
        .rects()
        .iter()
        .map(|r| r.dy)
        .filter(|d| *d <= MAX_HISTOGRAM_HEIGHT)
        .max()
        .unwrap_or(0)
        .max(0);
    let mut mode_dy = 0;
    if max_dy > 0 {
        let mut hist = vec![0usize; max_dy as usize + 1];
        for (_, _, r) in text.ink() {
            if r.dy > 0 && r.dy <= max_dy {
                hist[r.dy as usize] += 1;
            }
        }
        let mut mode_count = 0;
        for (d, count) in hist.iter().enumerate().skip(1) {
            if *count > mode_count {
                mode_count = *count;
                mode_dy = d as i32;
            }
        }
    }
    let can_strip = mode_dy >= MIN_BODY_DY;
    let tall = mode_dy + mode_dy / 2;

    let mut out = PageText::new();
    for (&c, &r) in text.chars().iter().zip(text.rects()) {
        // A tall glyph with too few glyphs of its height on its baseline.
        let watermark = can_strip && !is_space(c) && r.dy > tall && {
            let row = text
                .ink()
                .filter(|(_, _, o)| {
                    (o.bottom() - r.bottom()).abs() <= BASELINE_TOL
                        && (o.dy - r.dy).abs() * 2 <= r.dy
                })
                .take(MIN_ROW_GLYPHS)
                .count();
            row < MIN_ROW_GLYPHS
        };
        if !watermark {
            out.push(c, r);
        }
    }
    out
}

/// The fallback view: the page's full width from just above the target down to the last text, at most 200 pt (360
/// at a caption). A target on a caption line takes in the figure above it; a caption below the strip is taken in
/// whole.
pub fn landscape_box(mediabox: RectF, dest_y: f32, text: &PageText) -> RectF {
    const FIGURE_BODY_EXTEND: f32 = 250.0;
    const MAX_LANDSCAPE: f32 = 200.0;
    const MAX_LANDSCAPE_CAPTION: f32 = 360.0;

    let mut ty = if dest_y >= 0.0 {
        dest_y - ANCHOR_TOP_MARGIN
    } else {
        0.0
    }
    .max(0.0);
    let dest_at_caption = !text.is_empty() && dest_y > 0.0 && {
        let d_y = dest_y as i32;
        (0..text.len()).any(|i| {
            let gy = text.rect(i).y;
            gy >= d_y - 5 && gy <= d_y + 15 && caption_label_at(text, i)
        })
    };
    if dest_at_caption {
        ty = (ty - FIGURE_BODY_EXTEND).max(0.0);
    }
    let mut h = mediabox.dy - ty;
    if h <= 0.0 {
        h = mediabox.dy;
        ty = 0.0;
    }
    h = h.min(if dest_at_caption {
        MAX_LANDSCAPE_CAPTION
    } else {
        MAX_LANDSCAPE
    });

    if !text.is_empty() {
        if let Some(caption_end) = caption_block_end(text, (ty + h) as i32, mediabox.dy as i32) {
            h = h.max(caption_end as f32 + ANCHOR_TOP_MARGIN - ty);
        }

        // Trim the blank margin after the last text in the strip.
        let (top, bottom) = (ty as i32, (ty + h) as i32);
        let last = text
            .ink()
            .filter(|(_, _, r)| r.y >= top && r.y < bottom)
            .map(|(_, _, r)| r.bottom())
            .fold(top, i32::max);
        let trimmed = last as f32 + ANCHOR_TOP_MARGIN - ty;
        if trimmed > 20.0 && trimmed < h {
            h = trimmed;
        }
    }
    RectF::new(0.0, ty, mediabox.dx, h)
}

/// The bottom of the topmost caption block starting between `top` and `bottom`: its label line and up to two more
/// lines, stopping at a paragraph gap or where a short line is followed by a full one (the caption ended and the
/// body began).
fn caption_block_end(text: &PageText, top: i32, bottom: i32) -> Option<i32> {
    let mut start = None;
    let mut best_y = i32::MAX;
    for i in 0..text.len() {
        let gy = text.rect(i).y;
        if gy < top || gy > bottom || gy >= best_y {
            continue;
        }
        if caption_label_at(text, i) {
            start = Some(i);
            best_y = gy;
        }
    }
    let start = text.rect(start?);
    let line_h = if start.dy < 10 { 12 } else { start.dy };
    // A line reaching within 30 pt of the page's text margin is full width.
    let page_right = text.rects().iter().map(|r| r.right()).fold(0, i32::max);

    let mut end = start.y + line_h;
    let mut prev_bottom = start.y + line_h - 1;
    let mut seen_short = false;
    for line in 0..3 {
        let (band_top, band_bot) = if line == 0 {
            (start.y - 3, start.y + 3)
        } else {
            (prev_bottom + 1, prev_bottom + line_h * 18 / 10)
        };
        let mut found = false;
        let (mut line_top, mut line_bottom, mut line_right) = (i32::MAX, -1, 0);
        for r in text.rects() {
            if r.y < band_top || r.y > band_bot {
                continue;
            }
            found = true;
            line_top = line_top.min(r.y);
            line_bottom = line_bottom.max(r.bottom());
            line_right = line_right.max(r.right());
        }
        if !found {
            break;
        }
        let short = line_right < page_right - 30;
        if line >= 1 && (line_top - prev_bottom > line_h * 7 / 10 || (!short && seen_short)) {
            break;
        }
        end = line_bottom;
        prev_bottom = line_bottom;
        seen_short |= short;
    }
    Some(end)
}

/// A labelled display equation near the target: a "(N)" or "(N.M)" at the end of its line, in the right half of the
/// page, within 40 pt of the target. Its line, the page's full width, padded. `None` when there is none.
pub fn detect_equation_box(text: &PageText, mediabox: RectF, dest_y: f32) -> Option<RectF> {
    if dest_y <= 0.0 || text.is_empty() {
        return None;
    }
    let d_y = dest_y as i32;
    let chars = text.chars();
    let mut best: Option<(i32, i32)> = None;
    let mut best_dist = i32::MAX;
    for i in 0..chars.len() {
        if chars[i] != ')' {
            continue;
        }
        let ly = text.rect(i).y;
        if ly < d_y - 40 || ly > d_y + 40 {
            continue;
        }
        let same_line_digit = |p: usize| chars[p].is_ascii_digit() && text.rect(p).y == ly;
        // Walk back over the number, then an optional ".M" form's first part.
        let mut p = i;
        let mut digits = 0;
        while p > 0 && same_line_digit(p - 1) {
            p -= 1;
            digits += 1;
        }
        if digits == 0 {
            continue;
        }
        let mut had_dot = false;
        if p > 0 && chars[p - 1] == '.' && text.rect(p - 1).y == ly {
            had_dot = true;
            p -= 1;
            let mut more = 0;
            while p > 0 && same_line_digit(p - 1) {
                p -= 1;
                more += 1;
            }
            if more == 0 {
                continue;
            }
        }
        if p == 0 || chars[p - 1] != '(' || text.rect(p - 1).y != ly {
            continue;
        }
        let open = p - 1;
        // "(2013)" ending a reference line is a year, not an equation number.
        if !had_dot && digits >= 4 {
            continue;
        }
        let label_left = text.rect(open).x;
        let label_right = text.rect(i).right();
        let right_of = (0..chars.len()).any(|j| {
            !(open..=i).contains(&j)
                && !chars[j].is_whitespace()
                && text.rect(j).y == ly
                && text.rect(j).right() > label_right
        });
        // A label is the last thing on its line, and a "(N)" in the left half is a body-text marker.
        if right_of || label_left < (mediabox.dx * 0.5) as i32 {
            continue;
        }
        let dist = (ly - d_y).abs();
        if dist < best_dist {
            best_dist = dist;
            best = Some((ly, text.rect(i).dy));
        }
    }
    let (label_y, label_dy) = best?;
    let label_dy = if label_dy <= 0 { 12 } else { label_dy };
    let pad = label_dy as f32 + 6.0;
    Some(clip_to_mediabox(
        RectF::new(
            0.0,
            label_y as f32 - pad,
            mediabox.dx,
            label_dy as f32 + 2.0 * pad,
        ),
        mediabox,
    ))
}

/// The run of glyphs on `anchor`'s line (within 3 pt) reachable without crossing a gap wider than 20 pt, which keeps
/// it within one column. Its leftmost glyph, left edge and right edge.
fn line_run_extent(text: &PageText, anchor: usize) -> (usize, i32, i32) {
    const MAX_LINE_GAP: i32 = 20;
    let a = text.rect(anchor);
    let (mut left_idx, mut left_x, mut right_x) = (anchor, a.x, a.right());
    let mut extended = true;
    while extended {
        extended = false;
        for (i, _, r) in text.ink() {
            if r.y < a.y - 3 || r.y > a.y + 3 {
                continue;
            }
            if r.x < left_x && r.right() >= left_x - MAX_LINE_GAP {
                left_x = r.x;
                left_idx = i;
                extended = true;
            }
            if r.right() > right_x && r.x <= right_x + MAX_LINE_GAP {
                right_x = r.right();
                extended = true;
            }
        }
    }
    (left_idx, left_x, right_x)
}

/// The right edge of the column whose text starts at `scan_start`: scanning right through the glyphs between `top`
/// and `bottom`, the end of the last ink before a strip at least 8 pt wide with no ink on any of those lines (a
/// gutter). `None` when the page leaves no room to scan.
fn column_right(
    text: &PageText,
    mediabox: RectF,
    top: i32,
    bottom: i32,
    scan_start: i32,
) -> Option<i32> {
    const GUTTER_W: i32 = 8;
    let x_lo = (scan_start - 5).max(0);
    let x_hi = mediabox.dx as i32;
    if x_hi <= x_lo + 2 {
        return None;
    }
    let n = x_hi - x_lo;
    let mut occupied = vec![false; n as usize];
    for (_, _, r) in text.ink() {
        if r.y < top || r.y > bottom {
            continue;
        }
        let a = r.x - x_lo;
        let b = r.right() - x_lo;
        if b <= 0 || a >= n {
            continue;
        }
        occupied[a.max(0) as usize..b.min(n) as usize].fill(true);
    }
    let mut last = scan_start;
    for x in scan_start..x_hi {
        let idx = x - x_lo;
        if idx >= 0 && idx < n && occupied[idx as usize] {
            last = x + 1;
        } else if x - last >= GUTTER_W {
            break;
        }
    }
    Some(last)
}

/// A bracket entry that runs off the bottom of its column may go on at the top of the next one. That block, if it is
/// short (closed by the next "[" label within 60 pt, or the last text in the column) and doesn't itself start with a
/// label.
fn column_wrap_continuation(
    text: &PageText,
    mediabox: RectF,
    old_column_right: i32,
) -> Option<RectF> {
    const GUTTER_W: i32 = 8;
    const COL_WIDTH_MAX: i32 = 280;
    const MIN_TOP_MARGIN: i32 = 30;
    const MAX_CONTINUATION: i32 = 60;

    let next_col_left = text
        .ink()
        .map(|(_, _, r)| r.x)
        .filter(|x| *x > old_column_right + GUTTER_W)
        .min()?;

    // The next column's top line, leaving out lines across the whole page (a running title) and the top margin (a
    // running header in pieces).
    let top_y = text
        .ink()
        .filter(|(_, _, r)| r.x >= next_col_left - 5 && r.y >= MIN_TOP_MARGIN)
        .filter(|(i, _, _)| {
            let (_, left, right) = line_run_extent(text, *i);
            right - left <= COL_WIDTH_MAX && left >= next_col_left - 20
        })
        .map(|(_, _, r)| r.y)
        .min()?;
    let mut top_left = i32::MAX;
    let mut top_dy = 12;
    for (_, _, r) in text.ink() {
        if r.x < next_col_left - 5 || r.y < top_y - 3 || r.y > top_y + 3 {
            continue;
        }
        if r.x < top_left {
            top_left = r.x;
            top_dy = if r.dy > 0 { r.dy } else { 12 };
        }
    }

    // A top line that starts with "[" is the next entry, not a continuation.
    let starts_entry = (0..text.len()).any(|i| {
        let r = text.rect(i);
        text.ch(i) == '['
            && (top_y - 3..=top_y + 3).contains(&r.y)
            && (top_left - 3..=top_left + 3).contains(&r.x)
    });
    if starts_entry {
        return None;
    }

    let col_right = match column_right(text, mediabox, top_y - 2, top_y + 6 * top_dy, next_col_left)
    {
        Some(last) if last > next_col_left => last,
        _ => next_col_left + 250,
    };

    let cap_y = top_y + MAX_CONTINUATION;
    let sibling = (0..text.len())
        .filter(|i| text.ch(*i) == '[')
        .map(|i| text.rect(i))
        .filter(|r| r.x >= next_col_left - 5 && r.x <= next_col_left + 30)
        .filter(|r| r.y > top_y + top_dy / 2 && r.y < cap_y)
        .map(|r| r.y)
        .min();
    let in_column = |r: &Rect| r.x >= next_col_left - 20 && r.x <= col_right;
    if sibling.is_none()
        && text
            .ink()
            .any(|(_, _, r)| in_column(&r) && r.y >= cap_y - top_dy)
    {
        // Text runs on past the cap: not a short wrapped tail.
        return None;
    }
    let boundary = sibling.unwrap_or(cap_y);

    let (min_x, min_y, max_x, max_y) = text
        .ink()
        .filter(|(_, _, r)| in_column(r) && r.y >= top_y - 5 && r.y < boundary)
        .fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, (_, _, r)| {
            (
                b.0.min(r.x),
                b.1.min(r.y),
                b.2.max(r.right()),
                b.3.max(r.bottom()),
            )
        });
    if min_x == i32::MAX || max_x - min_x < 30 || max_y - min_y < 8 {
        return None;
    }
    Some(padded(min_x, min_y, max_x, max_y, mediabox))
}

/// What to show for a target on a references page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryRegion {
    /// The region.
    pub region: RectF,
    /// The rest of a bracket entry that went on at the top of the next column, if it did.
    pub continuation: Option<RectF>,
}

impl EntryRegion {
    fn whole(region: RectF) -> Self {
        Self {
            region,
            continuation: None,
        }
    }
}

/// The bibliography, glossary or abbreviation entry at the target (bracket "[Foo+09]" labels, hanging-indent
/// author-year entries, one-line description lists), or the landscape view when the target isn't such an entry (a
/// heading, a caption, a table, a code listing). A page with hardly any text is shown whole. `None` when there is no
/// text near the target, so the caller can look again (the anchor may be stale).
pub fn detect_entry_box(
    text: &PageText,
    mediabox: RectF,
    dest_x: f32,
    dest_y: f32,
) -> Option<EntryRegion> {
    const SPARSE_PAGE_TEXT_LEN: usize = 50;
    if text.len() < SPARSE_PAGE_TEXT_LEN {
        return Some(EntryRegion::whole(RectF::new(
            0.0,
            0.0,
            mediabox.dx,
            mediabox.dy,
        )));
    }
    let landscape = || Some(EntryRegion::whole(landscape_box(mediabox, dest_y, text)));
    if dest_y < 0.0 {
        return landscape();
    }

    let d_y = dest_y as i32;
    // Keep to the target's column, with a little room for a "[" starting just left of it.
    let column_left = if dest_x >= 0.0 {
        dest_x as i32 - 15
    } else {
        i32::MIN
    };

    // 1. The entry's first glyph: on the line nearest the target (not the topmost in the band, which in two columns
    // may be the other column's), leftmost.
    let mut start = None;
    let (mut best_dist, mut best_x) = (i32::MAX, i32::MAX);
    for (i, _, r) in text.ink() {
        if r.y < d_y - 5 || r.y > d_y + 30 || r.x < column_left {
            continue;
        }
        let dist = (r.y - d_y).abs();
        if dist < best_dist || (dist == best_dist && r.x < best_x) {
            best_dist = dist;
            best_x = r.x;
            start = Some(i);
        }
    }
    let mut start = start?;

    // Links often carry the source page's x; walk to the start of the line, unless already on the "[" label (walking
    // left from it can cross a narrow gutter).
    let (left_idx, _, line_run_right) = line_run_extent(text, start);
    if text.ch(start) != '[' {
        start = left_idx;
    }
    // A "[VB25]" label on a slightly different baseline from its body: look for it within a line height, at most a
    // hanging indent to the left.
    if text.ch(start) != '[' {
        const MAX_HANGING_INDENT: i32 = 60;
        let s = text.rect(start);
        let y_tol = s.dy.max(10);
        let mut bracket_x = s.x;
        for i in 0..text.len() {
            let r = text.rect(i);
            if text.ch(i) != '['
                || r.y < s.y - y_tol
                || r.y > s.y + y_tol
                || r.x >= bracket_x
                || r.x < s.x - MAX_HANGING_INDENT
            {
                continue;
            }
            bracket_x = r.x;
            start = i;
        }
    }

    let bracket = text.ch(start) == '[';
    let first = text.rect(start);
    let (first_left, first_y) = (first.x, first.y);
    let first_dy = if first.dy <= 0 { 12 } else { first.dy };
    let mut column_right_x = line_run_right + 40;
    // Where a bracket entry's body starts after the gap following its label; the column scan starts there so it
    // doesn't take that gap for a gutter.
    let mut body_left = None;

    if bracket {
        let y_tol = if first_dy > 6 { first_dy } else { 8 };
        if let Some(close) = (start + 1..text.len())
            .find(|i| (text.rect(*i).y - first_y).abs() <= y_tol && text.ch(*i) == ']')
            && let Some(j) = (close + 1..text.len()).find(|j| !is_space(text.ch(*j)))
        {
            let r = text.rect(j);
            if (r.y - first_y).abs() <= y_tol && r.x > text.rect(close).x {
                body_left = Some(r.x);
            }
        }
    }

    // A narrow label in a label column sized for the widest one: bridge the gap to the body (at most 50 pt, under a
    // gutter) and take the body's run, so the box isn't cut to the label's width.
    const MAX_LABEL_WIDTH: i32 = 70;
    if bracket && line_run_right - first_left < MAX_LABEL_WIDTH {
        const MAX_LABEL_SEP: i32 = 50;
        let band_bot = first_y + first_dy.max(10);
        let body = text
            .ink()
            .filter(|(_, _, r)| r.y >= first_y - 3 && r.y <= band_bot)
            .filter(|(_, _, r)| r.x > line_run_right + 3 && r.x <= line_run_right + MAX_LABEL_SEP)
            .fold(None, |best: Option<(usize, i32)>, (i, _, r)| match best {
                Some((_, x)) if x <= r.x => best,
                _ => Some((i, r.x)),
            });
        if let Some((b, _)) = body {
            let (_, left, right) = line_run_extent(text, b);
            body_left = Some(left);
            column_right_x = column_right_x.max(right + 40);
        }
    }

    // The line pitch, from the first two lines' tops, looking about one column wide.
    let mut line_pitch = first_dy;
    {
        const COL_WIDTH_MAX: i32 = 250;
        let next_top = text
            .ink()
            .filter(|(_, _, r)| {
                r.x >= first_left - 20 && r.x <= first_left + COL_WIDTH_MAX && r.y > first_y + 2
            })
            .map(|(_, _, r)| r.y)
            .min();
        if let Some(next) = next_top {
            let p = next - first_y;
            if p >= 4 && p < line_pitch * 2 {
                line_pitch = p;
            }
        }
    }

    // The column's right edge from a band of lines near the entry (not the whole page: a centred page number lower
    // down could bridge the gutter).
    let scan_start = body_left.unwrap_or(first_left);
    if let Some(last) = column_right(
        text,
        mediabox,
        first_y - 2 * line_pitch,
        first_y + 6 * line_pitch,
        scan_start,
    )
    .filter(|last| *last > first_left)
    {
        column_right_x = last;
    }
    let in_column = |r: &Rect| r.x >= first_left - 20 && r.x <= column_right_x;

    // A degenerate box falls through to the line scan.
    if bracket
        && let Some(found) = bracket_entry(
            text,
            mediabox,
            start,
            first_left,
            first_y,
            first_dy,
            line_pitch,
            column_right_x,
        )
    {
        return Some(found);
    }

    // 2. Scan forward for the entry's end.
    let mut end = text.len();
    let mut current_line_y = first_y;
    let mut current_line_bottom = first_y + first_dy;
    let mut prev_bottom = first_y + first_dy;
    let mut line_height = first_dy;
    let mut current_line_left = first_left;
    let mut prev_line_left = i32::MAX;
    // The x of the entry's second line, if seen.
    let mut indent_x: Option<i32> = None;
    // Seen another entry start at the first line's x with no indent between: a description list.
    let mut desc_list_sibling = false;
    let first_char = text.ch(start);
    let marked_entry = first_char == '[' || first_char == '(' || first_char.is_ascii_digit();

    for i in start + 1..text.len() {
        let c = text.ch(i);
        if is_space(c) {
            continue;
        }
        let r = text.rect(i);
        // The text went back up: a column wrap.
        if r.y < first_y - 5 {
            end = i;
            break;
        }
        if !in_column(&r) {
            continue;
        }
        // Only letter-height glyphs start a line: punctuation sits low and would fire too early.
        let major = r.dy * 2 >= first_dy;
        let new_line = major && r.y > current_line_bottom - 2;
        if new_line {
            prev_line_left = current_line_left;
            current_line_left = r.x;
            prev_bottom = current_line_bottom;
        } else if r.x < current_line_left {
            current_line_left = r.x;
        }
        let past_first_line = r.y > first_y + first_dy * 3 / 4 + 2;
        let at_first_left = r.x >= first_left - 5 && r.x <= first_left + 5;
        if new_line && past_first_line && indent_x.is_none() && !at_first_left {
            indent_x = Some(r.x);
        }

        // (a) A "[" at the first line's x is the next entry.
        if c == '[' && at_first_left {
            desc_list_sibling = true;
            end = i;
            break;
        }
        // (b) Back at the first line's x after an indented line: the next author-year entry.
        if new_line
            && at_first_left
            && past_first_line
            && prev_line_left != i32::MAX
            && (prev_line_left < first_left - 5 || prev_line_left > first_left + 5)
        {
            desc_list_sibling = true;
            end = i;
            break;
        }
        // (c) A paragraph gap; back at the first line's x it separates list siblings.
        if r.y > prev_bottom + line_height * 5 / 4 {
            desc_list_sibling |= at_first_left;
            end = i;
            break;
        }
        // (d) Back at the first line's x before any indent: the entry was one line.
        if new_line
            && past_first_line
            && at_first_left
            && indent_x.is_none()
            && prev_line_left != i32::MAX
        {
            desc_list_sibling = true;
            end = i;
            break;
        }
        // (e) An unmarked entry with no hanging indent stops after six lines.
        if !marked_entry && new_line && indent_x.is_none() {
            let lines = if line_height > 0 {
                (r.y - first_y + line_height - 1) / line_height
            } else {
                0
            };
            if lines >= 6 {
                end = i;
                break;
            }
        }
        if new_line {
            let dy = r.y - current_line_y;
            if dy > 4 && dy < 60 {
                line_height = dy;
            }
            current_line_y = r.y;
            current_line_bottom = r.bottom();
        } else if major {
            current_line_y = current_line_y.min(r.y);
            current_line_bottom = current_line_bottom.max(r.bottom());
        }
    }

    // 3. The box of the entry's glyphs.
    let (min_x, min_y, max_x, max_y) = (start..end)
        .filter(|i| !is_space(text.ch(*i)))
        .map(|i| text.rect(i))
        .filter(|r| in_column(r) && r.y >= first_y - 5)
        .fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, r| {
            (
                b.0.min(r.x),
                b.1.min(r.y),
                b.2.max(r.right()),
                b.3.max(r.bottom()),
            )
        });
    if min_x == i32::MAX {
        return landscape();
    }
    let region = padded(min_x, min_y, max_x, max_y, mediabox);
    if region.dx < 50.0 || region.dy < 20.0 {
        return landscape();
    }
    // A caption anywhere below: the target is a figure, table or listing (even one whose lines start with "[TAG]").
    let box_bottom = region.bottom() as i32;
    if (0..text.len()).any(|i| text.rect(i).y > box_bottom && caption_label_at(text, i)) {
        return landscape();
    }
    if bracket {
        return Some(EntryRegion::whole(region));
    }
    // A continuation far right of the first line is a table's column, not a hanging indent.
    if indent_x.is_some_and(|x| x > 0 && x - first_left > 80) {
        return landscape();
    }
    // A heading or caption ("6.2 Foo", "Figure 2.2: ..."), not a reference.
    if first_char.is_ascii_digit()
        || HEADING_WORDS
            .iter()
            .any(|w| word_at(text, start, w, After::WordEnd))
    {
        return landscape();
    }
    // Dense braces, semicolons and brackets: a code listing.
    let (code, total) = (start..end)
        .map(|i| text.ch(i))
        .filter(|c| !is_space(*c))
        .fold((0, 0), |(code, total), c| {
            (
                code + usize::from(matches!(c, '{' | '}' | ';' | '(' | ')')),
                total + 1,
            )
        });
    if total > 50 && code * 12 > total {
        return landscape();
    }
    if desc_list_sibling {
        return Some(EntryRegion::whole(region));
    }
    // One line, no indent, no sibling: a caption, heading or in-text target.
    if region.dy < 30.0 && indent_x.is_none() {
        return landscape();
    }
    // A multi-line author-year entry.
    Some(EntryRegion::whole(region))
}

/// A bracket-labelled entry, bounded below by the next "[" at its x (whatever order the page draws them in), or by
/// the first gap wider than 1.5 lines when it is the last one. `None` for a degenerate box.
#[expect(
    clippy::too_many_arguments,
    reason = "the entry's measured shape, passed through once"
)]
fn bracket_entry(
    text: &PageText,
    mediabox: RectF,
    start: usize,
    first_left: i32,
    first_y: i32,
    first_dy: i32,
    line_pitch: i32,
    column_right_x: i32,
) -> Option<EntryRegion> {
    const MAX_BRACKET_ENTRY: i32 = 250;
    const MIN_BODY_LINE_WIDTH: i32 = 30;
    let in_column = |r: &Rect| r.x >= first_left - 20 && r.x <= column_right_x;

    // The next label: up to 30 pt right of this one (a page or section number may come first), half a line lower.
    let sibling = (0..text.len())
        .filter(|i| *i != start && text.ch(*i) == '[')
        .map(|i| text.rect(i))
        .filter(|r| r.x >= first_left - 5 && r.x <= first_left + 30 && r.y > first_y + first_dy / 2)
        .map(|r| r.y)
        .filter(|y| *y < mediabox.dy as i32)
        .min();
    let mut boundary = sibling
        .unwrap_or(mediabox.dy as i32)
        .min(first_y + MAX_BRACKET_ENTRY)
        - 6;

    // Trim at the first gap wider than 1.5 lines, so a last entry doesn't take in the footer.
    let gap = (line_pitch * 3 / 2).max(12);
    let mut prev_bottom = first_y + line_pitch;
    let mut block_bottom = prev_bottom;
    loop {
        let next_bottom = text
            .ink()
            .filter(|(_, _, r)| in_column(r) && r.y > prev_bottom - 2 && r.y <= prev_bottom + gap)
            .map(|(_, _, r)| r.bottom())
            .max();
        // A glyph that doesn't reach below the block can't grow it; stop rather than loop on it.
        match next_bottom {
            Some(b) if b > prev_bottom => {
                block_bottom = b;
                prev_bottom = b;
            }
            _ => break,
        }
    }
    boundary = boundary.min(block_bottom + 1);

    let (min_x, min_y, max_x, max_y) = text
        .ink()
        .filter(|(_, _, r)| in_column(r) && r.y >= first_y - 5 && r.y < boundary)
        .fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, (_, _, r)| {
            (
                b.0.min(r.x),
                b.1.min(r.y),
                b.2.max(r.right()),
                b.3.max(r.bottom()),
            )
        });
    if min_x == i32::MAX || max_x - min_x < 50 || max_y - min_y < 12 {
        return None;
    }
    let region = padded(min_x, min_y, max_x, max_y, mediabox);
    if region.dx < 50.0 || region.dy < 20.0 {
        return None;
    }
    // No label closed it and no body line follows in the column (a page number or a page-wide footer doesn't
    // count): it may go on at the top of the next column.
    let continuation = if sibling.is_some() {
        None
    } else {
        let more_below = text.ink().any(|(i, _, r)| {
            if !in_column(&r) || r.y <= max_y {
                return false;
            }
            let (_, left, right) = line_run_extent(text, i);
            right - left >= MIN_BODY_LINE_WIDTH && right <= column_right_x + 10
        });
        if more_below {
            None
        } else {
            column_wrap_continuation(text, mediabox, column_right_x)
        }
    };
    Some(EntryRegion {
        region,
        continuation,
    })
}
