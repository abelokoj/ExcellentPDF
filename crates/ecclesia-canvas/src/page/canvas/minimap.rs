//! The mini-map on the canvas (plan R.17): the strip down the right edge, its marks, and scrolling by pressing or
//! dragging on it. The model is `ecclesia-features`' `read::minimap`; this reads the annotations, keeps them up to
//! date as edits change pages, and draws the strip over the pages.
//!
//! The whole document's annotations are read once on a worker. After that an edit, an undo or a redo bumps the
//! revisions of the pages it changed, and those pages are read again on this thread before the next frame is drawn,
//! so the marks follow an annotation change in the frame that shows it.

use ecclesia_core::annot::{self, Mark as AnnotMark};
use ecclesia_core::render::{Job, Priority};
use ecclesia_core::shared::pool;
use ecclesia_features::read::minimap::{self, Lane, Mark, MarkKind, Order, Palette, Scale};
use ecclesia_mupdf::{AnnotKind, PagePoint, PageRect};

use super::PageCanvas;
use crate::Rect;
use crate::layout::{Axis, Direction, Flow};
use crate::page::host::{Host, TriangleDirection, Wash};
use crate::page::placement::Placement;

/// Search hits: an amber that shows on light and dark strips.
const HIT: u32 = 0xf29900;
/// The reader's hit: a deeper orange.
const CURRENT_HIT: u32 = 0xe8710a;
/// An annotation without a colour of its own.
const NO_COLOR: u32 = 0xf9ab00;
/// Room between the strip and the view's edges, in pixels at 96 dpi.
const INSET: f32 = 2.0;
/// A view narrower than this many strips draws none: the pages need the room.
const MIN_VIEW: f32 = 8.0;
/// Least height of the scrollable track between the top and bottom arrows, in pixels at 96 dpi.
const MIN_TRACK_HEIGHT: f32 = 24.0;
/// Horizontal inset on each side of the scrollbar thumb pill, in pixels at 96 dpi.
const THUMB_INSET_X: f32 = 3.0;
/// Least width of the scrollbar thumb pill, in pixels at 96 dpi.
const MIN_THUMB_WIDTH: f32 = 4.0;
/// Width of each scrollbar arrow triangle (75° apex at 30% corner curve), in pixels at 96 dpi.
const ARROW_TRIANGLE_WIDTH: f32 = 9.5;
/// Height of each scrollbar arrow triangle (75° apex: `9.5 / (2 * tan(37.5°)) ≈ 6.2`), in pixels at 96 dpi.
const ARROW_TRIANGLE_HEIGHT: f32 = 6.2;
/// Corner rounding of each scrollbar arrow triangle (30% curve), in pixels at 96 dpi.
const ARROW_CORNER_RADIUS: f32 = 1.9;
/// Share of the viewport's height scrolled per press on a scrollbar arrow button.
const ARROW_SCROLL_FRACTION: f32 = 0.15;
/// Least scroll step per press on a scrollbar arrow button, in pixels at 96 dpi.
const MIN_ARROW_SCROLL: f32 = 80.0;

/// The canvas's mini-map state.
#[derive(Default)]
pub(super) struct MiniMap {
    /// Each page's annotation marks, once read.
    marks: Vec<Vec<AnnotMark>>,
    /// Each page's revision when its marks were read.
    revisions: Vec<u64>,
    /// The whole document's marks being read.
    loading: Option<Job<Vec<Vec<AnnotMark>>>>,
    /// The document's edit count when its marks were last brought up to date.
    edits: Option<u64>,
    /// The bookmarks: page, place on it in page points, colour.
    bookmarks: Vec<(usize, PagePoint, u32)>,
    /// While the strip is held: how far below the thumb's middle the pointer is, in pixels.
    held: Option<f32>,
}

impl MiniMap {
    /// Forgets the document's marks: it closed, or its pages were laid out again. Bookmarks are the app's, sent with
    /// each change of file, and stay.
    pub(super) fn forget(&mut self) {
        self.marks.clear();
        self.revisions.clear();
        self.loading = None;
        self.edits = None;
        self.held = None;
    }
}

/// The scrollbar strip's regions and scale.
struct Strip {
    /// Full strip rectangle including both arrow buttons and the track.
    rect: Rect,
    /// Top scrollbar arrow button area.
    top_arrow: Rect,
    /// Bottom scrollbar arrow button area.
    bottom_arrow: Rect,
    /// Scrollable track between the top and bottom arrows.
    track: Rect,
    /// Layout-to-track coordinate mapping.
    scale: Scale,
}

impl<H: Host> PageCanvas<H> {
    /// The bookmarks to mark (B.6): page, place on the page in page points, colour `0xRRGGBB`.
    pub(crate) fn set_bookmarks(&mut self, bookmarks: Vec<(usize, PagePoint, u32)>) {
        if self.minimap.bookmarks != bookmarks {
            self.minimap.bookmarks = bookmarks;
            self.invalidate();
        }
    }

    /// Brings the annotation marks up to date: reads the whole document's on a worker the first time, and after an
    /// edit reads again the pages whose revision moved. Called before a frame is drawn.
    pub(super) fn update_marks(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let pages = self.layout.len();
        if pages == 0 || self.minimap.loading.is_some() {
            return;
        }
        let edits = document.edits();
        if self.minimap.marks.len() != pages {
            let mupdf = match document.mupdf() {
                Ok(mupdf) => mupdf,
                Err(err) => {
                    log::warn!("cannot read the annotations for the mini-map: {err}");
                    return;
                }
            };
            self.minimap.revisions = (0..pages).map(|page| document.revision(page)).collect();
            self.minimap.edits = Some(edits);
            let job = pool().load_marks(&mupdf, 0..pages, Priority::Background);
            let waker = std::sync::Arc::clone(&self.waker);
            job.when_done(move || waker.wake());
            self.minimap.loading = Some(job);
            return;
        }
        if self.minimap.edits == Some(edits) {
            return;
        }
        self.minimap.edits = Some(edits);
        let changed: Vec<usize> = (0..pages)
            .filter(|&page| document.revision(page) != self.minimap.revisions[page])
            .collect();
        if changed.is_empty() {
            return;
        }
        let mupdf = match document.mupdf() {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::warn!("cannot read the annotations for the mini-map: {err}");
                return;
            }
        };
        let mupdf = mupdf.lock().expect("document lock poisoned");
        for page in changed {
            self.minimap.revisions[page] = document.revision(page);
            self.minimap.marks[page] = annot::page_marks(&mupdf, page).unwrap_or_else(|err| {
                log::warn!("cannot read the annotations of page {page}: {err}");
                Vec::new()
            });
        }
    }

    /// The whole document's marks were read, if they were.
    pub(super) fn take_marks(&mut self) -> bool {
        let Some(job) = &mut self.minimap.loading else {
            return false;
        };
        let Some(result) = job.try_take() else {
            return false;
        };
        self.minimap.loading = None;
        match result {
            Ok(marks) if marks.len() == self.layout.len() => self.minimap.marks = marks,
            Ok(_) => log::debug!("the page count changed while the mini-map read the annotations"),
            Err(err) => {
                if !err.is_aborted() {
                    log::warn!("cannot read the annotations for the mini-map: {err}");
                }
                // Not read again in a loop: the marks stay empty for this document.
                self.minimap.marks = vec![Vec::new(); self.layout.len()];
            }
        }
        true
    }

    /// The strip, if the view is large enough for one and a document is shown.
    fn strip(&self) -> Option<Strip> {
        let first = self.layout.bands().first()?;
        let last = self.layout.bands().last()?;
        let scale = self.dpi_scale();
        let (width, height) = self.viewport_size();
        let strip_width = minimap::WIDTH * scale;
        if width < MIN_VIEW * strip_width {
            return None;
        }
        let inset = INSET * scale;
        let arrow_h = minimap::ARROW_HEIGHT * scale;
        if height < 2.0 * (inset + arrow_h) + MIN_TRACK_HEIGHT * scale {
            return None;
        }
        let x = width - inset - strip_width;
        let rect = Rect::new(x, inset, strip_width, height - 2.0 * inset);
        let top_arrow = Rect::new(x, rect.y0, strip_width, arrow_h);
        let bottom_arrow = Rect::new(x, rect.y1 - arrow_h, strip_width, arrow_h);
        let track = Rect::new(x, top_arrow.y1, strip_width, bottom_arrow.y0 - top_arrow.y1);
        let order = match (self.layout.axis(), self.layout.spec().direction) {
            (Axis::Horizontal, Direction::RightToLeft) => Order::Reversed,
            _ => Order::Forward,
        };
        let scale = Scale::new(
            first.start,
            last.end,
            order,
            track.y0,
            track.height(),
            scale,
        )?;
        Some(Strip {
            rect,
            top_arrow,
            bottom_arrow,
            track,
            scale,
        })
    }

    /// The part on screen along the layout.
    fn shown(&self) -> (f32, f32) {
        self.layout.axis().span(&self.viewport.visible())
    }

    /// Where page `page`'s rectangle `rect` (page points) lies along the layout.
    fn along(&self, page: usize, rect: &PageRect) -> Option<(f32, f32)> {
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        let a = placement.to_layout(PagePoint {
            x: rect.x0,
            y: rect.y0,
        });
        let b = placement.to_layout(PagePoint {
            x: rect.x1,
            y: rect.y1,
        });
        let (a, b) = match self.layout.axis() {
            Axis::Vertical => (a.1, b.1),
            Axis::Horizontal => (a.0, b.0),
        };
        Some((a.min(b), a.max(b)))
    }

    /// Everything to mark: annotations, bookmarks and search hits.
    fn marks(&self) -> Vec<Mark> {
        let annotations = self
            .minimap
            .marks
            .iter()
            .enumerate()
            .flat_map(|(page, marks)| marks.iter().map(move |mark| (page, mark)))
            .filter_map(|(page, mark)| {
                let (from, to) = self.along(page, &mark.rect)?;
                let kind = match mark.kind {
                    AnnotKind::Highlight
                    | AnnotKind::Underline
                    | AnnotKind::Squiggly
                    | AnnotKind::StrikeOut => MarkKind::Highlight,
                    _ => MarkKind::Note,
                };
                Some(Mark {
                    from,
                    to,
                    kind,
                    color: mark.color.unwrap_or(NO_COLOR),
                })
            });
        let bookmarks = self
            .minimap
            .bookmarks
            .iter()
            .filter_map(|&(page, at, color)| {
                let point = PageRect {
                    x0: at.x,
                    y0: at.y,
                    x1: at.x,
                    y1: at.y,
                };
                let (from, to) = self.along(page, &point)?;
                Some(Mark {
                    from,
                    to,
                    kind: MarkKind::Bookmark,
                    color,
                })
            });
        let hits = self
            .hit_boxes()
            .into_iter()
            .filter_map(|(page, rect, mine)| {
                let (from, to) = self.along(page, &rect)?;
                Some(if mine {
                    Mark {
                        from,
                        to,
                        kind: MarkKind::CurrentHit,
                        color: CURRENT_HIT,
                    }
                } else {
                    Mark {
                        from,
                        to,
                        kind: MarkKind::Hit,
                        color: HIT,
                    }
                })
            });
        annotations.chain(bookmarks).chain(hits).collect()
    }

    /// Draws the strip over the pages: its fill, top and bottom arrow triangles, a tick per row of pages, the pill
    /// thumb and the marks. A document that fits on screen and has nothing marked draws none.
    pub(super) fn draw_minimap(&mut self) -> Result<(), H::Error> {
        let Some(Strip {
            rect,
            top_arrow,
            bottom_arrow,
            track,
            scale,
        }) = self.strip()
        else {
            return Ok(());
        };
        let marks = self.marks();
        let (from, to) = self.shown();
        if marks.is_empty() && scale.shows_all(from, to) {
            return Ok(());
        }
        let palette = Palette::for_background(self.background);
        let px = self.dpi_scale();
        let ticks = scale.ticks(self.layout.bands().iter().map(|band| band.start));
        let placed = scale.place(&marks);
        let (y0, y1) = scale.thumb(from, to);
        let top_tri = arrow_triangle_rect(&top_arrow, px);
        let bottom_tri = arrow_triangle_rect(&bottom_arrow, px);
        let corner_radius = ARROW_CORNER_RADIUS * px;
        let thumb_w = (track.width() - 2.0 * THUMB_INSET_X * px).max(MIN_THUMB_WIDTH * px);
        let thumb_x = track.x0 + (track.width() - thumb_w) * 0.5;
        let host = &mut self.host;
        host.fill(&rect, palette.track)?;
        host.rounded_triangle(
            &top_tri,
            TriangleDirection::Up,
            corner_radius,
            palette.arrow,
        )?;
        host.rounded_triangle(
            &bottom_tri,
            TriangleDirection::Down,
            corner_radius,
            palette.arrow,
        )?;
        for y in ticks {
            host.fill(
                &Rect::new(track.x0, y, track.width(), px.round().max(1.0)),
                palette.tick,
            )?;
        }
        host.band(
            &Rect::new(thumb_x, y0, thumb_w, y1 - y0),
            thumb_w * 0.5,
            palette.thumb,
            Wash::Translucent,
        )?;
        for mark in placed {
            let (x0, x1) = mark.lane.span(track.x0, track.width());
            // Lanes keep a hairline apart, so neighbours read as two marks.
            let gap = if mark.lane == Lane::Across {
                0.0
            } else {
                px * 0.5
            };
            host.fill(
                &Rect::new(x0 + gap, mark.y0, x1 - x0 - 2.0 * gap, mark.y1 - mark.y0),
                palette.ink(mark.color),
            )?;
        }
        Ok(())
    }

    /// The left button went down at client pixel (`x`, `y`): on an arrow button it scrolls one step along the layout;
    /// on the track it holds the strip and scrolls there (a press on the thumb keeps the screen where it is and drags
    /// it). Returns whether the strip took the press.
    pub(super) fn minimap_down(&mut self, x: i32, y: i32) -> bool {
        let Some(Strip {
            rect,
            top_arrow,
            bottom_arrow,
            scale,
            ..
        }) = self.strip()
        else {
            return false;
        };
        let (x, y) = (x as f32, y as f32);
        if !contains(&rect, x, y) {
            return false;
        }
        let (from, to) = self.shown();
        // Not drawn: the press is the pages'.
        if scale.shows_all(from, to) && self.marks().is_empty() {
            return false;
        }
        if contains(&top_arrow, x, y) {
            let step = (self.viewport_size().1 * ARROW_SCROLL_FRACTION)
                .max(MIN_ARROW_SCROLL * self.dpi_scale());
            self.viewport.scroll_forward(-step, &self.layout);
            self.invalidate();
            return true;
        }
        if contains(&bottom_arrow, x, y) {
            let step = (self.viewport_size().1 * ARROW_SCROLL_FRACTION)
                .max(MIN_ARROW_SCROLL * self.dpi_scale());
            self.viewport.scroll_forward(step, &self.layout);
            self.invalidate();
            return true;
        }
        let (y0, y1) = scale.thumb(from, to);
        let grab = if (y0..=y1).contains(&y) {
            y - (y0 + y1) / 2.0
        } else {
            0.0
        };
        self.minimap.held = Some(grab);
        self.scroll_with_strip(&scale, y, grab);
        true
    }

    /// The mouse moved while the strip is held: the screen follows it. Returns whether the strip is held.
    pub(super) fn minimap_drag(&mut self, y: i32) -> bool {
        let Some(grab) = self.minimap.held else {
            return false;
        };
        if let Some(Strip { scale, .. }) = self.strip() {
            self.scroll_with_strip(&scale, y as f32, grab);
        }
        true
    }

    /// The left button came up: the strip lets go. Returns whether it was held.
    pub(super) fn minimap_up(&mut self) -> bool {
        self.minimap.held.take().is_some()
    }

    /// Whether the strip is held, or the pointer at client pixel (`x`, `y`) is over it: the pointer is an arrow.
    pub(super) fn over_minimap(&self, x: i32, y: i32) -> bool {
        self.minimap.held.is_some()
            || self
                .strip()
                .is_some_and(|Strip { rect, .. }| contains(&rect, x as f32, y as f32))
    }

    /// Scrolls so the screen sits where strip pixel `y` says, held `grab` pixels below the thumb's middle.
    fn scroll_with_strip(&mut self, scale: &Scale, y: f32, grab: f32) {
        let (from, to) = self.shown();
        let target = scale.scroll_to(y, grab, to - from);
        if self.layout.flow() == Flow::Paged
            && let Some(band) = self.layout.band_at(target + (to - from) / 2.0)
            && band != self.viewport.band()
        {
            let first = self.layout.bands()[band].pages.start;
            self.viewport.scroll_to_page(first, &self.layout);
        }
        let (from, _) = self.shown();
        let delta = (target - from) * self.viewport.scale();
        match self.layout.axis() {
            Axis::Vertical => self.viewport.scroll_by(0.0, delta, &self.layout),
            Axis::Horizontal => self.viewport.scroll_by(delta, 0.0, &self.layout),
        }
        self.invalidate();
    }
}

/// Whether client pixel (`x`, `y`) lies inside `rect`.
fn contains(rect: &Rect, x: f32, y: f32) -> bool {
    x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1
}

/// Centered bounding box for a scrollbar arrow triangle inside `button`.
fn arrow_triangle_rect(button: &Rect, px: f32) -> Rect {
    let w = ARROW_TRIANGLE_WIDTH * px;
    let h = ARROW_TRIANGLE_HEIGHT * px;
    Rect::new(
        button.x0 + (button.width() - w) * 0.5,
        button.y0 + (button.height() - h) * 0.5,
        w,
        h,
    )
}
