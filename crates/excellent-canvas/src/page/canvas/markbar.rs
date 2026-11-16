//! The markup bar (plan A.1): once text is selected in a document that can be edited, a small bar floats under the
//! selection with the four text markups (highlight, underline, squiggly, strikeout) and five colours. A markup
//! button, or its command, sends the selection to the app as `markup`, which makes it one undoable edit; the model
//! is `excellent-features`' `annotate::markup`.

use excellent_core::annot::Markup;
use excellent_text::TextPos;

use super::PageCanvas;
use super::refhover::luminance;
use crate::Rect;
use crate::page::host::{Host, Wash};

/// The colours offered, `0xRRGGBB`: yellow first, the colour a highlight is expected in.
pub(super) const COLORS: [u32; 5] = [0xffd400, 0x7ed957, 0x5ec8f2, 0xff8fb1, 0xffa94d];

/// The markups, in the bar's order.
const KINDS: [Markup; 4] = [
    Markup::Highlight,
    Markup::Underline,
    Markup::Squiggly,
    Markup::StrikeOut,
];

/// Sizes at 96 dpi, in pixels: a button's square, a colour's cell, the bar's padding, the gap under the selection.
const BUTTON: f32 = 32.0;
const DOT_CELL: f32 = 26.0;
const DOT: f32 = 7.0;
const PAD: f32 = 4.0;
const SEPARATOR: f32 = 9.0;
const GAP: f32 = 8.0;
const RADIUS: f32 = 8.0;

/// Characters of context kept either side of a markup's text, to find it again (K.28).
const CONTEXT: usize = 32;

/// What part of the bar is under a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    Kind(Markup),
    Color(usize),
    /// The bar, between its buttons.
    Frame,
}

/// The bar's state.
#[derive(Debug, Default)]
pub(super) struct MarkBar {
    /// The colour chosen, an index into [`COLORS`].
    color: usize,
    /// Where it was last drawn, in client pixels; `None` while it isn't shown.
    frame: Option<Rect>,
    /// What the pointer is over.
    hover: Option<Hit>,
    /// A markup waiting for the text of selected pages that aren't read yet.
    pub(super) pending: Option<Markup>,
}

impl<H: Host> PageCanvas<H> {
    /// Marks the selected text with `kind` in the bar's colour: the canvas sends it to the app as `markup`:
    /// `{kind, color: [r, g, b], pages: [{page, quads: [[ulx, uly, urx, ury, llx, lly, lrx, lry]], quote: {prefix,
    /// exact, suffix, start}}]}` in page points, colour channels 0 to 1. The selection then goes. Pages of the
    /// selection whose text isn't read yet are waited for.
    pub fn mark(&mut self, kind: Markup) {
        let Some(selection) = self.selection.filter(|s| !s.is_collapsed()) else {
            return;
        };
        if !self.can_mark() {
            return;
        }
        if selection.pages().any(|page| self.texts.get(page).is_none()) {
            self.markbar.pending = Some(kind);
            return;
        }
        self.markbar.pending = None;
        let flat = self.flat_text();
        let mut pages = Vec::new();
        for page in selection.pages() {
            let Some(text) = self.texts.get(page) else {
                continue;
            };
            let Some(range) = selection.range_on(page, text) else {
                continue;
            };
            let exact = text.text(range.clone());
            if exact.trim().is_empty() {
                continue;
            }
            let quads: Vec<[f32; 8]> = text
                .highlight(range.clone())
                .iter()
                .map(|q| {
                    [
                        q.ul.x, q.ul.y, q.ur.x, q.ur.y, q.ll.x, q.ll.y, q.lr.x, q.lr.y,
                    ]
                })
                .collect();
            let quote = serde_json::json!({
                "prefix": text.text(range.start.saturating_sub(CONTEXT)..range.start),
                "exact": exact,
                "suffix": text.text(range.end..range.end + CONTEXT),
                "start": flat.offset(TextPos { page, index: range.start }),
            });
            pages.push(serde_json::json!({ "page": page, "quads": quads, "quote": quote }));
        }
        if pages.is_empty() {
            return;
        }
        let rgb = COLORS[self.markbar.color];
        let color = [16, 8, 0].map(|shift| f64::from((rgb >> shift) & 0xff) / 255.0);
        self.emit(
            "markup",
            serde_json::json!({ "kind": kind_name(kind), "color": color, "pages": pages }),
        );
        self.selection = None;
        self.markbar.frame = None;
        self.host.selection_changed();
        self.invalidate();
    }

    /// Whether the document shown can take markups: a PDF, which edits save into.
    fn can_mark(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.is_editable())
    }

    /// Whether client pixel (`x`, `y`) is over the bar.
    pub(super) fn over_markbar(&self, x: i32, y: i32) -> bool {
        self.markbar_hit(x, y).is_some()
    }

    /// The pointer moved: the button under it is lit.
    pub(super) fn markbar_hover(&mut self, at: Option<(i32, i32)>) {
        let hover = at.and_then(|(x, y)| self.markbar_hit(x, y));
        if hover != self.markbar.hover {
            self.markbar.hover = hover;
            self.invalidate();
        }
    }

    /// The left button went down at client pixel (`x`, `y`): on a markup it marks the selection, on a colour it
    /// chooses it. Returns whether the bar took the press.
    pub(super) fn markbar_down(&mut self, x: i32, y: i32) -> bool {
        match self.markbar_hit(x, y) {
            None => false,
            Some(Hit::Kind(kind)) => {
                self.mark(kind);
                true
            }
            Some(Hit::Color(index)) => {
                self.markbar.color = index;
                self.invalidate();
                true
            }
            Some(Hit::Frame) => true,
        }
    }

    fn markbar_hit(&self, x: i32, y: i32) -> Option<Hit> {
        let frame = self.markbar.frame?;
        let (x, y) = (x as f32, y as f32);
        if !(frame.x0 <= x && x < frame.x1 && frame.y0 <= y && y < frame.y1) {
            return None;
        }
        let cells = cells(frame, self.dpi_scale());
        let hit = cells
            .kinds
            .iter()
            .zip(KINDS)
            .find(|(cell, _)| cell.x0 <= x && x < cell.x1)
            .map(|(_, kind)| Hit::Kind(kind))
            .or_else(|| {
                cells
                    .colors
                    .iter()
                    .position(|cell| cell.x0 <= x && x < cell.x1)
                    .map(Hit::Color)
            });
        Some(hit.unwrap_or(Hit::Frame))
    }

    /// Draws the bar under the selection's last line, or over its first when there is no room below; it shows
    /// only while text is selected, not being dragged, in a document that can be edited.
    pub(super) fn draw_markbar(&mut self) -> Result<(), H::Error> {
        self.markbar.frame = None;
        let shown = self.selection.is_some_and(|s| !s.is_collapsed()) && !self.selecting;
        if !shown || !self.can_mark() {
            return Ok(());
        }
        let quads = self.selection_quads();
        let (Some(first), Some(last)) = (quads.first(), quads.last()) else {
            return Ok(());
        };
        let s = self.dpi_scale();
        let width = 2.0 * PAD + 4.0 * BUTTON + SEPARATOR + 5.0 * DOT_CELL;
        let (width, height) = (width * s, (BUTTON + 2.0 * PAD) * s);
        let (view_w, view_h) = (self.viewport.width(), self.viewport.height());
        let below = last.iter().map(|p| p.1).fold(f32::MIN, f32::max) + GAP * s;
        let above = first.iter().map(|p| p.1).fold(f32::MAX, f32::min) - GAP * s - height;
        let top = if below + height <= view_h || above < 0.0 {
            below
        } else {
            above
        };
        if top >= view_h || top + height <= 0.0 {
            // The selection is off the screen.
            return Ok(());
        }
        let middle = last.iter().map(|p| p.0).sum::<f32>() / 4.0;
        let left = (middle - width / 2.0).clamp(PAD * s, (view_w - width - PAD * s).max(PAD * s));
        let frame = Rect::new(left.round(), top.round(), width.round(), height.round());
        self.markbar.frame = Some(frame);

        let dark = luminance(self.background) < 0.5;
        let (edge, shadow, ink, lit) = if dark {
            (0x5c_6068, 0x00_0000, 0xe6_e8ec, 0x3a_3e46)
        } else {
            (0xc4_c8cf, 0x1a_1d23, 0x3a_3f47, 0xe8_ebf0)
        };
        let paper = if dark { 0x26_282d } else { 0xff_ffff };
        let color = COLORS[self.markbar.color];
        let hover = self.markbar.hover;
        let chosen = self.markbar.color;
        let host = &mut self.host;
        let radius = RADIUS * s;
        host.band(
            &Rect::new(frame.x0 + s, frame.y0 + 3.0 * s, width, height),
            radius,
            shadow,
            Wash::Translucent,
        )?;
        host.band(&frame, radius, edge, Wash::Darkest)?;
        let hair = s.max(1.0).round();
        let inner = Rect::new(
            frame.x0 + hair,
            frame.y0 + hair,
            frame.width() - 2.0 * hair,
            frame.height() - 2.0 * hair,
        );
        host.band(&inner, radius - hair, paper, Wash::Darkest)?;
        let cells = cells(frame, s);
        for (cell, kind) in cells.kinds.iter().zip(KINDS) {
            if hover == Some(Hit::Kind(kind)) {
                host.band(&inset(cell, 2.0 * s), 6.0 * s, lit, Wash::Darkest)?;
            }
            glyph(host, kind, cell, s, ink, color)?;
        }
        let rule = cells.separator;
        host.fill(&Rect::new(rule.0, rule.1, hair, rule.2), edge)?;
        for (index, cell) in cells.colors.iter().enumerate() {
            let (cx, cy) = center(cell);
            if index == chosen || hover == Some(Hit::Color(index)) {
                let ring = if index == chosen { ink } else { edge };
                host.band(
                    &circle(cx, cy, (DOT + 3.0) * s),
                    (DOT + 3.0) * s,
                    ring,
                    Wash::Darkest,
                )?;
                host.band(
                    &circle(cx, cy, (DOT + 1.5) * s),
                    (DOT + 1.5) * s,
                    paper,
                    Wash::Darkest,
                )?;
            }
            host.band(
                &circle(cx, cy, DOT * s),
                DOT * s,
                COLORS[index],
                Wash::Darkest,
            )?;
        }
        Ok(())
    }
}

/// The bar's parts in client pixels.
struct Cells {
    kinds: [Rect; 4],
    /// The rule between the markups and the colours: its left, top and height.
    separator: (f32, f32, f32),
    colors: [Rect; 5],
}

fn cells(frame: Rect, s: f32) -> Cells {
    let (top, size) = (frame.y0 + PAD * s, BUTTON * s);
    let mut x = frame.x0 + PAD * s;
    let kinds = [0; 4].map(|_| {
        let cell = Rect::new(x, top, size, size);
        x += size;
        cell
    });
    let separator = (
        x + (SEPARATOR * s / 2.0).floor(),
        top + 6.0 * s,
        size - 12.0 * s,
    );
    x += SEPARATOR * s;
    let colors = [0; 5].map(|_| {
        let cell = Rect::new(x, top, DOT_CELL * s, size);
        x += DOT_CELL * s;
        cell
    });
    Cells {
        kinds,
        separator,
        colors,
    }
}

/// A markup's icon, drawn from the host's shapes: grey lines of "text" and the markup on them in `color`.
fn glyph<H: Host>(
    host: &mut H,
    kind: Markup,
    cell: &Rect,
    s: f32,
    ink: u32,
    color: u32,
) -> Result<(), H::Error> {
    let (cx, cy) = center(cell);
    let text = |dy: f32, w: f32| Rect::new(cx - 8.0 * s, cy + dy * s, w * s, 2.0 * s);
    match kind {
        Markup::Highlight => {
            host.band(
                &Rect::new(cx - 10.0 * s, cy - 6.0 * s, 20.0 * s, 12.0 * s),
                2.0 * s,
                color,
                Wash::Darkest,
            )?;
            host.fill(&text(-1.0, 16.0), ink)?;
        }
        Markup::Underline => {
            host.fill(&text(-4.0, 16.0), ink)?;
            host.fill(
                &Rect::new(cx - 9.0 * s, cy + 3.0 * s, 18.0 * s, 2.5 * s),
                color,
            )?;
        }
        Markup::Squiggly => {
            host.fill(&text(-4.0, 16.0), ink)?;
            let points: Vec<(f32, f32, f32)> = (0..=6)
                .map(|step| {
                    let x = cx - 9.0 * s + step as f32 * 3.0 * s;
                    let y = cy + if step % 2 == 0 { 3.0 } else { 6.0 } * s;
                    (x, y, 1.8 * s)
                })
                .collect();
            host.polyline(&points, color)?;
        }
        Markup::StrikeOut => {
            host.fill(&text(-5.0, 16.0), ink)?;
            host.fill(&text(3.0, 12.0), ink)?;
            host.fill(
                &Rect::new(cx - 10.0 * s, cy - 1.5 * s, 20.0 * s, 2.5 * s),
                color,
            )?;
        }
    }
    Ok(())
}

/// The name a markup goes to the app by.
fn kind_name(kind: Markup) -> &'static str {
    match kind {
        Markup::Highlight => "highlight",
        Markup::Underline => "underline",
        Markup::Squiggly => "squiggly",
        Markup::StrikeOut => "strikeOut",
    }
}

fn center(r: &Rect) -> (f32, f32) {
    ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
}

fn circle(cx: f32, cy: f32, radius: f32) -> Rect {
    Rect::new(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius)
}

fn inset(r: &Rect, by: f32) -> Rect {
    Rect::new(
        r.x0 + by,
        r.y0 + by,
        r.width() - 2.0 * by,
        r.height() - 2.0 * by,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_s_cells_sit_side_by_side_inside_it() {
        let frame = Rect::new(
            100.0,
            50.0,
            2.0 * PAD + 4.0 * BUTTON + SEPARATOR + 5.0 * DOT_CELL,
            BUTTON + 2.0 * PAD,
        );
        let cells = cells(frame, 1.0);
        assert!((cells.kinds[0].x0 - (100.0 + PAD)).abs() < f32::EPSILON);
        assert!(cells.kinds[3].x1 <= cells.separator.0);
        assert!(cells.separator.0 < cells.colors[0].x0);
        assert!((cells.colors[4].x1 - (frame.x1 - PAD)).abs() < 0.01);
    }
}
