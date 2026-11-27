//! Citation and equation hover on the canvas (plan R.25): hold the pointer on a link inside the document, or on a
//! citation in plain text such as `[12]` or `(Smith, 2020)`, and a popover shows the entry, equation or figure it
//! points at, drawn from the page itself. The model is `ecclesia-features`' `read::refhover`.
//!
//! One thread per canvas does the waiting and the work: it holds each request for the hover delay, dropping it for a
//! newer one, then finds the target (walking the document's text when there is no link), picks the region and
//! renders it on the render pool. The canvas only asks, and draws what comes back.
//!
//! Parity with SumatraPDF Enhanced:
//! - Configurable maximum width as a percentage of page width (`40%..=100%`, default `90%`).
//! - Vivid red `0.7 pt` anti-aliased rounded bounding box (`#FF3030`) with rounded corners.
//! - Interactive edge & corner drag resizing with live flicker-free re-rendering.
//! - Mouse-wheel scrolling (with continuous page rollover) and `Ctrl+wheel` zooming inside the popover.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use ecclesia_core::darkmode::PageColors;
use ecclesia_core::document::{Document, SharedMupdf};
use ecclesia_core::render::{Priority, Recolor};
use ecclesia_core::shared::pool;
use ecclesia_core::transform::{Rotation, Transform};
use ecclesia_features::read::refhover::resolve::{
    self, Lookups, Pages, Place, Room, Shown, Size as PixelSize,
};
use ecclesia_features::read::refhover::{PageText, RectF};
use ecclesia_mupdf::{DeviceRect, DisplayList, Link, PagePoint, PageRect, Pixmap};

use super::{Cursor, PageCanvas, Waker};
use crate::Rect;
use crate::page::host::{Host, PopoverWheel, Wash};

/// How long the pointer rests before a popover shows, as in Enhanced (its `CitationHoverDelay`).
pub(in crate::page) const DEFAULT_DELAY: Duration = Duration::from_millis(300);
/// Default maximum popover width as a percentage of the page width (SumatraPDF Enhanced default).
pub(in crate::page) const DEFAULT_MAX_WIDTH_PCT: f32 = resolve::DEFAULT_MAX_WIDTH_PCT;
/// How long a popover stays after the pointer leaves, so it can cross the gap onto the popover.
const LINGER: Duration = Duration::from_millis(250);
/// Pages of text the hover thread keeps between requests.
const TEXT_CACHE: usize = 256;
/// Vivid red bounding-box color (`RGB(255, 48, 48)`), matching SumatraPDF Enhanced.
const POPOVER_BORDER: u32 = 0xFF_3030;
/// Bounding-box stroke thickness in typographical points (`1 pt = 1/72 in`).
const POPOVER_BORDER_PT: f32 = 0.7;
/// The popover's outer corner radius, in layout pixels at 96 dpi.
const RADIUS: f32 = 6.0;
/// Minimum padding between the red border and rendered slices, in layout pixels at 96 dpi.
const SLICE_PAD: f32 = 2.0;
/// The popover's shadow offset, in layout pixels at 96 dpi.
const SHADOW: f32 = 3.0;
/// Grab band thickness inside the popover's edges/corners, in layout pixels at 96 dpi.
const RESIZE_GRIP_IN: f32 = 8.0;
/// Extra grab band tolerance outside the popover border, in layout pixels at 96 dpi.
const RESIZE_GRIP_OUT: f32 = 2.0;
/// Minimum popover width while resizing, in layout pixels at 96 dpi (SumatraPDF Enhanced).
const MIN_POPOVER_WIDTH: f32 = 160.0;
/// Minimum popover height while resizing, in layout pixels at 96 dpi (SumatraPDF Enhanced).
const MIN_POPOVER_HEIGHT: f32 = 100.0;
/// Margin kept from the viewport edges when clamping popup size, in layout pixels at 96 dpi.
const VIEWPORT_MARGIN: f32 = 8.0;
/// Minimum interval between live re-render requests while actively dragging a resize edge.
const RESIZE_RENDER_THROTTLE: Duration = Duration::from_millis(45);

/// Physical-pixel geometry of the popover's rounded `0.7 pt` red bounding box and inner surface.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PopoverChrome {
    /// Exact `0.7 pt` border thickness in physical pixels (`0.7 * 96.0 * dpi_scale / 72.0`).
    border_px: f32,
    /// Outer corner radius in physical pixels (`6.0 * dpi_scale`).
    outer_radius: f32,
    /// Inner paper corner radius in physical pixels (`(outer_radius - border_px).max(0.0)`).
    inner_radius: f32,
    /// Inset from the outer frame to rendered PDF slices so corners never bleed over the border.
    content_inset: f32,
}

impl PopoverChrome {
    /// Computes popover chrome geometry at `dpi_scale` (1.0 at 96 dpi).
    fn at_scale(dpi_scale: f32) -> Self {
        let scale = if dpi_scale.is_finite() && dpi_scale > 0.0 {
            dpi_scale
        } else {
            1.0
        };
        let border_px = POPOVER_BORDER_PT * (96.0 * scale) / 72.0;
        let outer_radius = RADIUS * scale;
        let inner_radius = (outer_radius - border_px).max(0.0);
        let content_inset = (border_px + SLICE_PAD * scale).max(resolve::size::BORDER * scale);
        Self {
            border_px,
            outer_radius,
            inner_radius,
            content_inset,
        }
    }
}

/// Which edge or corner of the popover the pointer is grabbing or hovering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeEdge {
    /// Left edge (`ew-resize`).
    Left,
    /// Right edge (`ew-resize`).
    Right,
    /// Top edge (`ns-resize`).
    Top,
    /// Bottom edge (`ns-resize`).
    Bottom,
    /// Top-left corner (`nwse-resize`).
    TopLeft,
    /// Top-right corner (`nesw-resize`).
    TopRight,
    /// Bottom-left corner (`nesw-resize`).
    BottomLeft,
    /// Bottom-right corner (`nwse-resize`).
    BottomRight,
}

impl ResizeEdge {
    /// The pointer cursor shape for this resize edge or corner.
    const fn cursor(self) -> Cursor {
        match self {
            Self::Left | Self::Right => Cursor::ResizeEw,
            Self::Top | Self::Bottom => Cursor::ResizeNs,
            Self::TopLeft | Self::BottomRight => Cursor::ResizeNwse,
            Self::TopRight | Self::BottomLeft => Cursor::ResizeNesw,
        }
    }
}

/// Result of hit-testing a client-pixel point against an open popover frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PopoverHit {
    /// Pointer is outside the popover and its outer resize grab band.
    #[default]
    Outside,
    /// Pointer is strictly inside the popover content area (clicking navigates to the target).
    Interior,
    /// Pointer is over one of the 8 edge/corner resize bands.
    Edge(ResizeEdge),
}

/// Outcome of a left mouse press tested against the popover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PopoverPress {
    /// Press landed outside the popover; continue normal canvas handling.
    Ignored,
    /// Press landed strictly inside the popover and jumped to the previewed target.
    Navigated,
    /// Press grabbed a resize edge or corner; host should capture the mouse.
    StartedResize,
}

/// Active mouse drag resizing the popover frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ResizeDrag {
    edge: ResizeEdge,
    start_point: (f32, f32),
    start_frame: Rect,
    start_region: PageRect,
    last_render: Option<Instant>,
}

/// Whether a rendered result places a fresh popover or swaps slices inside the open frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlacementMode {
    /// Place a freshly opened popover relative to the pointer.
    Auto,
    /// Keep the open popover's frame intact (resize, wheel scroll, or wheel zoom).
    KeepFrame,
}

/// What the pointer rests on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    /// A link to `target`, at `rect` on page `page`.
    Link {
        page: usize,
        rect: PageRect,
        target: ecclesia_mupdf::Target,
    },
    /// Text at `point` on page `page`, which may be a citation.
    Text { page: usize, point: PagePoint },
}

/// What the hover thread is asked to perform.
#[derive(Debug, Clone, Copy)]
enum AskKind {
    /// Resolve and render a hovered link or plain-text citation after `delay`.
    Resolve(Source),
    /// Re-render an already-open popover's region immediately (resize, wheel scroll, or zoom).
    Rerender {
        page: usize,
        region: PageRect,
        base_zoom: f32,
        user_zoom: f32,
        page_screen_width: f32,
    },
    /// The pointer left: dismiss the popover after [`LINGER`].
    Dismiss,
}

/// What the canvas last asked about, to ask again only when it changes: the link, or the character under the
/// pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Asked {
    Link(usize, PageRect),
    Char(usize, usize),
    /// The popover itself.
    Popover,
    Nothing,
}

/// A request to the hover thread.
struct Ask {
    generation: u64,
    kind: AskKind,
    delay: Duration,
    document: Arc<Document>,
    mupdf: SharedMupdf,
    bounds: Arc<Vec<PageRect>>,
    page_zoom: f32,
    room: Room,
    colors: Option<PageColors>,
    /// The pointer, in view pixels.
    at: (f32, f32),
}

/// The hover thread's answer to the newest [`Ask`].
struct Answer {
    generation: u64,
    /// `None`: nothing to show.
    shown: Option<Rendered>,
}

/// A popover's slices, rendered, and where it goes.
struct Rendered {
    slices: Vec<Pixmap>,
    /// The pointer it was asked for, in view pixels.
    at: (f32, f32),
    /// The target, for an interior click on the popover to go to.
    target: (usize, PagePoint),
    /// Currently rendered destination page (0-based).
    page: usize,
    /// Currently rendered primary region, in page points.
    region: PageRect,
    /// Base zoom (device pixels per PDF point) chosen when opened.
    base_zoom: f32,
    /// User zoom multiplier (1.0 initially, adjusted via Ctrl+wheel).
    user_zoom: f32,
    /// Visible width of the source page on screen when opened, for max-width clamping.
    page_screen_width: f32,
    /// Whether this result places a fresh popover or refreshes the open frame in place.
    placement: PlacementMode,
}

/// A popover on screen.
pub(super) struct Popover<I> {
    slices: Vec<(I, f32, f32)>,
    /// Its frame, in view pixels.
    frame: Rect,
    target: (usize, PagePoint),
    /// The layout on screen when it showed: scrolling or zooming takes it down.
    visible: Rect,
    /// Currently displayed destination page (0-based).
    page: usize,
    /// Currently displayed primary PDF region, in page points.
    region: PageRect,
    /// Base zoom (device pixels per PDF point) when opened.
    base_zoom: f32,
    /// User zoom multiplier adjusted by Ctrl+wheel inside the popover.
    user_zoom: f32,
    /// Visible width of the source page on screen when opened, for max-width clamping.
    page_screen_width: f32,
}

/// The canvas's hover state.
pub(super) struct RefHover<I> {
    /// `None`: hover is off.
    delay: Option<Duration>,
    /// Maximum popover width as a percentage of page width (`40.0..=100.0`, default `90.0`).
    max_width_pct: f32,
    asks: Option<Sender<Ask>>,
    answers: Option<Receiver<Answer>>,
    asked: Asked,
    generation: u64,
    /// Answered but not shown yet: its slices upload on the canvas's thread.
    ready: Option<Rendered>,
    shown: Option<Popover<I>>,
    /// Active edge/corner resize drag, if any.
    resizing: Option<ResizeDrag>,
    /// Last hit-test result over the open popover, used by [`PageCanvas::cursor`].
    hovered_hit: PopoverHit,
}

impl<I> Default for RefHover<I> {
    fn default() -> Self {
        Self {
            delay: Some(DEFAULT_DELAY),
            max_width_pct: DEFAULT_MAX_WIDTH_PCT,
            asks: None,
            answers: None,
            asked: Asked::Nothing,
            generation: 0,
            ready: None,
            shown: None,
            resizing: None,
            hovered_hit: PopoverHit::Outside,
        }
    }
}

impl<I> RefHover<I> {
    /// Forgets the popover and what was asked: the pages changed. The thread's caches go with the document.
    pub(super) fn forget(&mut self) {
        self.generation += 1;
        self.asked = Asked::Nothing;
        self.ready = None;
        self.shown = None;
        self.resizing = None;
        self.hovered_hit = PopoverHit::Outside;
    }
}

impl<H: Host> PageCanvas<H> {
    /// How long the pointer rests on a link or citation before its popover shows (R.25); `None` turns it off.
    pub fn set_hover_delay(&mut self, delay: Option<Duration>) {
        self.refhover.delay = delay;
        if delay.is_none() && self.refhover.shown.is_some() {
            self.refhover.forget();
            self.invalidate();
        }
    }

    /// Sets the maximum reference preview width as a percentage of the page width (`40.0..=100.0`,
    /// default `90.0`).
    pub fn set_hover_max_width_pct(&mut self, pct: f32) {
        let clamped = if pct.is_finite() {
            pct.clamp(resolve::MIN_MAX_WIDTH_PCT, resolve::MAX_MAX_WIDTH_PCT)
        } else {
            DEFAULT_MAX_WIDTH_PCT
        };
        self.refhover.max_width_pct = clamped;
        let Some(page_screen_width) = self.refhover.shown.as_ref().map(|p| p.page_screen_width)
        else {
            return;
        };
        let max_w = self.max_popover_width(Some(page_screen_width));
        let scale = self.dpi_scale();
        let chrome = PopoverChrome::at_scale(scale);
        let rerender = if let Some(popover) = &mut self.refhover.shown
            && popover.frame.width() > max_w
        {
            popover.frame.x1 = (popover.frame.x0 + max_w).round();
            let zoom = popover.base_zoom * popover.user_zoom;
            let content = PixelSize::new(
                (popover.frame.width() - 2.0 * chrome.content_inset).max(1.0),
                (popover.frame.height() - 2.0 * chrome.content_inset).max(1.0),
            );
            self.bounds.get(popover.page).copied().map(|bounds| {
                let region = resolve::resized_region(popover.region, zoom, content, bounds);
                popover.region = region;
                (
                    popover.page,
                    region,
                    popover.base_zoom,
                    popover.user_zoom,
                    popover.page_screen_width,
                )
            })
        } else {
            None
        };
        if let Some((page, region, base_zoom, user_zoom, page_w)) = rerender {
            self.request_popover_rerender(page, region, base_zoom, user_zoom, page_w);
            self.invalidate();
        }
    }

    /// Whether client pixel (`x`, `y`) is over the popover or its edge/corner resize grab band.
    pub(super) fn over_popover(&self, x: i32, y: i32) -> bool {
        if self.refhover.resizing.is_some() {
            return true;
        }
        self.refhover.shown.as_ref().is_some_and(|p| {
            hit_test_popover(&p.frame, x as f32, y as f32, self.dpi_scale()) != PopoverHit::Outside
        })
    }

    /// Pointer cursor override while hovering or resizing the open popover.
    pub(super) fn popover_cursor(&self) -> Option<Cursor> {
        if let Some(drag) = self.refhover.resizing {
            return Some(drag.edge.cursor());
        }
        self.refhover.shown.as_ref()?;
        match self.refhover.hovered_hit {
            PopoverHit::Edge(edge) => Some(edge.cursor()),
            PopoverHit::Interior => Some(Cursor::Hand),
            PopoverHit::Outside => None,
        }
    }

    /// Pointer cursor override evaluated live at client pixel (`x`, `y`).
    pub(super) fn popover_cursor_at(&self, x: i32, y: i32) -> Option<Cursor> {
        if let Some(drag) = self.refhover.resizing {
            return Some(drag.edge.cursor());
        }
        let popover = self.refhover.shown.as_ref()?;
        match hit_test_popover(&popover.frame, x as f32, y as f32, self.dpi_scale()) {
            PopoverHit::Edge(edge) => Some(edge.cursor()),
            PopoverHit::Interior => Some(Cursor::Hand),
            PopoverHit::Outside => None,
        }
    }

    /// Tracks pointer hover over the popover and drives an active edge/corner resize drag.
    ///
    /// Returns `true` while an active resize drag consumes the move event.
    pub(super) fn popover_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        if let Some(mut drag) = self.refhover.resizing {
            if let Some((x, y)) = at {
                let scale = self.dpi_scale();
                let chrome = PopoverChrome::at_scale(scale);
                let (view_w, view_h) = self.viewport_size();
                let margin = VIEWPORT_MARGIN * scale;
                let page_screen_width = self.refhover.shown.as_ref().map(|p| p.page_screen_width);
                let max_w = self.max_popover_width(page_screen_width);
                let min_w = (MIN_POPOVER_WIDTH * scale).min(max_w).max(1.0);
                let max_h = (view_h - 2.0 * margin).max(MIN_POPOVER_HEIGHT * scale);
                let min_h = (MIN_POPOVER_HEIGHT * scale).min(max_h).max(1.0);
                let dx = x as f32 - drag.start_point.0;
                let dy = y as f32 - drag.start_point.1;
                let bounds = ResizeBounds {
                    min_w,
                    max_w,
                    min_h,
                    max_h,
                    view_w,
                    view_h,
                    margin,
                };
                let new_frame = resize_frame(drag.start_frame, drag.edge, dx, dy, bounds);
                let rerender = if let Some(popover) = &mut self.refhover.shown {
                    popover.frame = new_frame;
                    let zoom = popover.base_zoom * popover.user_zoom;
                    let due = drag
                        .last_render
                        .is_none_or(|last| last.elapsed() >= RESIZE_RENDER_THROTTLE);
                    if due && let Some(bounds) = self.bounds.get(popover.page).copied() {
                        let new_region = region_for_resize(
                            drag.start_region,
                            drag.start_frame,
                            new_frame,
                            zoom,
                            chrome.content_inset,
                            bounds,
                        );
                        if !is_empty_page_rect(new_region) && new_region != popover.region {
                            drag.last_render = Some(Instant::now());
                            popover.region = new_region;
                            popover.target = (
                                popover.page,
                                PagePoint {
                                    x: new_region.x0,
                                    y: new_region.y0,
                                },
                            );
                            Some((
                                popover.page,
                                new_region,
                                popover.base_zoom,
                                popover.user_zoom,
                                popover.page_screen_width,
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                self.refhover.resizing = Some(drag);
                if let Some((page, region, base_zoom, user_zoom, page_w)) = rerender {
                    self.request_popover_rerender(page, region, base_zoom, user_zoom, page_w);
                }
                self.invalidate();
            }
            return true;
        }
        self.refhover.hovered_hit = match (at, self.refhover.shown.as_ref()) {
            (Some((x, y)), Some(popover)) => {
                hit_test_popover(&popover.frame, x as f32, y as f32, self.dpi_scale())
            }
            _ => PopoverHit::Outside,
        };
        false
    }

    /// Finishes an active resize drag on mouse-up, ensuring a crisp final render at the settled
    /// size while keeping the popover open. Returns whether a resize drag ended.
    pub(super) fn popover_up(&mut self) -> bool {
        self.finish_popover_resize()
    }

    /// Finishes an active resize drag cleanly when capture is lost.
    pub(super) fn popover_cancel(&mut self) {
        let _ = self.finish_popover_resize();
    }

    /// Ends a resize drag and schedules one settled high-res re-render if needed.
    fn finish_popover_resize(&mut self) -> bool {
        let Some(drag) = self.refhover.resizing.take() else {
            return false;
        };
        let chrome = PopoverChrome::at_scale(self.dpi_scale());
        let rerender = if let Some(popover) = &mut self.refhover.shown {
            let zoom = popover.base_zoom * popover.user_zoom;
            self.bounds.get(popover.page).copied().and_then(|bounds| {
                let settled = region_for_resize(
                    drag.start_region,
                    drag.start_frame,
                    popover.frame,
                    zoom,
                    chrome.content_inset,
                    bounds,
                );
                if is_empty_page_rect(settled) {
                    return None;
                }
                popover.region = settled;
                popover.target = (
                    popover.page,
                    PagePoint {
                        x: settled.x0,
                        y: settled.y0,
                    },
                );
                Some((
                    popover.page,
                    settled,
                    popover.base_zoom,
                    popover.user_zoom,
                    popover.page_screen_width,
                ))
            })
        } else {
            None
        };
        if let Some((page, region, base_zoom, user_zoom, page_w)) = rerender {
            self.request_popover_rerender(page, region, base_zoom, user_zoom, page_w);
        }
        self.invalidate();
        true
    }

    /// Handles mouse-wheel scroll or `Ctrl+wheel` zoom when the pointer at device pixel (`x`, `y`)
    /// is over an open reference hover popover.
    ///
    /// Returns `true` when the popover absorbed the event so the main document does not scroll.
    pub fn popover_wheel(&mut self, x: f32, y: f32, action: PopoverWheel) -> bool {
        let scale = self.dpi_scale();
        let Some(hit) = self
            .refhover
            .shown
            .as_ref()
            .map(|p| hit_test_popover(&p.frame, x, y, scale))
        else {
            return false;
        };
        if hit == PopoverHit::Outside {
            return false;
        }
        // Keep the popover pinned open while the user scrolls or zooms inside it.
        self.refhover.generation += 1;
        self.refhover.asked = Asked::Popover;
        let chrome = PopoverChrome::at_scale(scale);

        let rerender = match action {
            PopoverWheel::Zoom { notches } => {
                if !notches.is_finite() || notches.abs() < 1e-5 {
                    return true;
                }
                let Some(popover) = &mut self.refhover.shown else {
                    return true;
                };
                let next_user = (popover.user_zoom * resolve::size::ZOOM_STEP.powf(notches))
                    .clamp(resolve::size::MIN_ZOOM, resolve::size::MAX_ZOOM);
                if (next_user - popover.user_zoom).abs() < 1e-5 {
                    return true;
                }
                let Some(bounds) = self.bounds.get(popover.page).copied() else {
                    return true;
                };
                let content = PixelSize::new(
                    (popover.frame.width() - 2.0 * chrome.content_inset).max(1.0),
                    (popover.frame.height() - 2.0 * chrome.content_inset).max(1.0),
                );
                let next_region = resolve::resized_region(
                    popover.region,
                    popover.base_zoom * next_user,
                    content,
                    bounds,
                );
                if is_empty_page_rect(next_region) {
                    return true;
                }
                popover.user_zoom = next_user;
                popover.region = next_region;
                popover.target = (
                    popover.page,
                    PagePoint {
                        x: next_region.x0,
                        y: next_region.y0,
                    },
                );
                Some((
                    popover.page,
                    next_region,
                    popover.base_zoom,
                    popover.user_zoom,
                    popover.page_screen_width,
                ))
            }
            PopoverWheel::Scroll { delta_y } => {
                if !delta_y.is_finite() || delta_y.abs() < 1e-3 {
                    return true;
                }
                let page_count = self.bounds.len();
                let Some(popover) = &mut self.refhover.shown else {
                    return true;
                };
                let zoom = popover.base_zoom * popover.user_zoom;
                if zoom <= 0.0 || !zoom.is_finite() || page_count == 0 {
                    return true;
                }
                let Some(mut bounds) = self.bounds.get(popover.page).copied() else {
                    return true;
                };
                let region_w = (popover.region.x1 - popover.region.x0).max(1.0);
                let region_h = (popover.region.y1 - popover.region.y0).max(1.0);
                let scroll_pt = delta_y / zoom;
                let mut page = popover.page;
                let mut new_y0 = popover.region.y0 + scroll_pt;

                if new_y0 < bounds.y0 {
                    if page > 0 {
                        let overflow = bounds.y0 - new_y0;
                        page -= 1;
                        if let Some(prev) = self.bounds.get(page).copied() {
                            bounds = prev;
                            new_y0 = (bounds.y1 - region_h - overflow).max(bounds.y0);
                        } else {
                            page = popover.page;
                            new_y0 = bounds.y0;
                        }
                    } else {
                        new_y0 = bounds.y0;
                    }
                } else if new_y0 + region_h > bounds.y1 {
                    if page + 1 < page_count {
                        let overflow = (new_y0 + region_h) - bounds.y1;
                        page += 1;
                        if let Some(next) = self.bounds.get(page).copied() {
                            bounds = next;
                            new_y0 = (bounds.y0 + overflow)
                                .min((bounds.y1 - region_h).max(bounds.y0))
                                .max(bounds.y0);
                        } else {
                            page = popover.page;
                            new_y0 = (bounds.y1 - region_h).max(bounds.y0);
                        }
                    } else {
                        new_y0 = (bounds.y1 - region_h).max(bounds.y0);
                    }
                }

                let clamped_h = region_h.min((bounds.y1 - bounds.y0).max(1.0));
                let mut new_x0 = popover.region.x0;
                let mut clamped_w = region_w;
                if new_x0 + clamped_w > bounds.x1 {
                    new_x0 = (bounds.x1 - clamped_w).max(bounds.x0);
                    clamped_w = clamped_w.min((bounds.x1 - new_x0).max(1.0));
                }
                new_x0 = new_x0.max(bounds.x0);
                let next_region = PageRect {
                    x0: new_x0,
                    y0: new_y0,
                    x1: (new_x0 + clamped_w).min(bounds.x1),
                    y1: (new_y0 + clamped_h).min(bounds.y1),
                };
                if page == popover.page && next_region == popover.region {
                    return true;
                }
                popover.page = page;
                popover.region = next_region;
                popover.target = (
                    page,
                    PagePoint {
                        x: next_region.x0,
                        y: next_region.y0,
                    },
                );
                Some((
                    page,
                    next_region,
                    popover.base_zoom,
                    popover.user_zoom,
                    popover.page_screen_width,
                ))
            }
        };

        if let Some((page, region, base_zoom, user_zoom, page_w)) = rerender {
            self.request_popover_rerender(page, region, base_zoom, user_zoom, page_w);
        }
        true
    }

    /// The pointer moved to client pixel `at` (`None`: off the canvas), over `link` if any: asks the hover thread
    /// about what is under it when that changed. Over the popover or its resize border, it stays.
    pub(super) fn hover_moved(&mut self, at: Option<(i32, i32)>, link: Option<&Link>) {
        if self.refhover.resizing.is_some() {
            return;
        }
        let Some(delay) = self.refhover.delay else {
            return;
        };
        if at.is_some_and(|(x, y)| self.over_popover(x, y)) {
            // Holding it: a take-down on its way is void, and leaving it counts as a change.
            self.refhover.generation += 1;
            self.refhover.asked = Asked::Popover;
            return;
        }
        let (asked, source) = match at {
            Some((x, y)) => self.hover_source(x, y, link),
            None => (Asked::Nothing, None),
        };
        if asked == self.refhover.asked {
            return;
        }
        self.refhover.asked = asked;
        self.refhover.generation += 1;
        self.refhover.ready = None;
        if source.is_none() && self.refhover.shown.is_none() {
            // Nothing shown, nothing to wait for; the thread drops what it holds when the next request comes.
            return;
        }
        let kind = match source {
            Some(src) => AskKind::Resolve(src),
            None => AskKind::Dismiss,
        };
        let Some(ask) = self.ask(kind, delay, at.unwrap_or_default()) else {
            return;
        };
        self.send_ask(ask);
    }

    /// Schedules an immediate worker re-render of the open popover's current region and zoom.
    fn request_popover_rerender(
        &mut self,
        page: usize,
        region: PageRect,
        base_zoom: f32,
        user_zoom: f32,
        page_screen_width: f32,
    ) {
        self.refhover.generation += 1;
        self.refhover.ready = None;
        let kind = AskKind::Rerender {
            page,
            region,
            base_zoom,
            user_zoom,
            page_screen_width,
        };
        let Some(ask) = self.ask(kind, Duration::ZERO, (0, 0)) else {
            return;
        };
        self.send_ask(ask);
    }

    /// Maximum allowed popover width in physical device pixels for the configured setting and
    /// current viewport/page dimensions.
    fn max_popover_width(&self, page_screen_width: Option<f32>) -> f32 {
        let scale = self.dpi_scale();
        let (view_w, _) = self.viewport_size();
        let frac = self.refhover.max_width_pct / 100.0;
        let view_cap = (view_w - 2.0 * VIEWPORT_MARGIN * scale).max(1.0);
        match page_screen_width.filter(|w| w.is_finite() && *w > 0.0) {
            Some(page_w) => (page_w * frac).min(view_cap),
            None => (view_w * frac).min(view_cap),
        }
    }

    /// What is under client pixel (`x`, `y`): a link into this document, or text.
    fn hover_source(&self, x: i32, y: i32, link: Option<&Link>) -> (Asked, Option<Source>) {
        let Some((page, point)) = self.page_point(x, y) else {
            return (Asked::Nothing, None);
        };
        if let Some(link) = link {
            return match link.target {
                Some(target) => (
                    Asked::Link(page, link.rect),
                    Some(Source::Link {
                        page,
                        rect: link.rect,
                        target,
                    }),
                ),
                None => (Asked::Nothing, None),
            };
        }
        let Some(text) = self.texts.get(page).filter(|text| text.is_over_text(point)) else {
            return (Asked::Nothing, None);
        };
        match text.nearest_char(point) {
            Some(index) => (Asked::Char(page, index), Some(Source::Text { page, point })),
            None => (Asked::Nothing, None),
        }
    }

    /// The request for `kind`, with what the thread needs to answer it.
    fn ask(&self, kind: AskKind, delay: Duration, (x, y): (i32, i32)) -> Option<Ask> {
        let document = self.document.clone()?;
        let mupdf = document
            .mupdf()
            .inspect_err(|err| log::warn!("cannot open the document for a hover preview: {err}"))
            .ok()?;
        let src_page = match &kind {
            AskKind::Resolve(Source::Link { page, .. } | Source::Text { page, .. }) => Some(*page),
            AskKind::Rerender { .. } | AskKind::Dismiss => None,
        };
        let page_rect = src_page
            .and_then(|page| self.layout.page(page))
            .map(|page| self.viewport.to_screen(&page))
            .map_or(RectF::default(), |r| {
                RectF::new(r.x0, r.y0, r.width(), r.height())
            });
        let (width, _) = self.viewport_size();
        let wait = match &kind {
            AskKind::Resolve(_) => delay,
            AskKind::Rerender { .. } => Duration::ZERO,
            AskKind::Dismiss => LINGER,
        };
        Some(Ask {
            generation: self.refhover.generation,
            kind,
            delay: wait,
            document,
            mupdf,
            bounds: Arc::new(self.bounds.clone()),
            page_zoom: self.viewport.scale(),
            room: Room {
                screen_width: width,
                page: page_rect,
                pointer_y: y as f32,
                scale: self.dpi_scale(),
                max_width_pct: self.refhover.max_width_pct,
            },
            colors: self.page_colors,
            at: (x as f32, y as f32),
        })
    }

    /// Sends `ask` to the hover thread, starting it the first time.
    fn send_ask(&mut self, ask: Ask) {
        if self.refhover.asks.is_none() {
            let (asks, inbox) = mpsc::channel();
            let (outbox, answers) = mpsc::channel();
            let waker = Arc::clone(&self.waker);
            let spawned = std::thread::Builder::new()
                .name("refhover".to_owned())
                .spawn(move || serve(&inbox, &outbox, &waker));
            if let Err(err) = spawned {
                log::error!("cannot start the hover preview thread: {err}");
                self.refhover.delay = None;
                return;
            }
            self.refhover.asks = Some(asks);
            self.refhover.answers = Some(answers);
        }
        if let Some(asks) = &self.refhover.asks
            && asks.send(ask).is_err()
        {
            log::error!("the hover preview thread stopped; previews are off");
            self.refhover.delay = None;
        }
    }

    /// Takes the hover thread's answers: the newest one shows its popover, or takes it down. Returns whether the
    /// frame changed.
    pub(super) fn take_hover(&mut self) -> bool {
        let Some(answers) = &self.refhover.answers else {
            return false;
        };
        let mut changed = false;
        while let Ok(answer) = answers.try_recv() {
            if answer.generation != self.refhover.generation {
                continue;
            }
            match answer.shown {
                Some(rendered) => self.refhover.ready = Some(rendered),
                None => {
                    self.refhover.ready = None;
                    if self.refhover.resizing.is_none() {
                        self.refhover.hovered_hit = PopoverHit::Outside;
                        changed |= self.refhover.shown.take().is_some();
                    }
                }
            }
        }
        changed | self.upload_popover()
    }

    /// Uploads a rendered popover's slices and either places a fresh popover (clamped to the
    /// configured max width percentage of the page and within the viewport) or swaps slices into
    /// the open frame in place during resize / wheel updates.
    fn upload_popover(&mut self) -> bool {
        if self.refhover.ready.is_none() || !self.host.can_upload() {
            return false;
        }
        let Some(rendered) = self.refhover.ready.take() else {
            return false;
        };
        let mut slices = Vec::with_capacity(rendered.slices.len());
        for pixmap in &rendered.slices {
            let Some((image, _)) = self.host.upload(pixmap) else {
                return false;
            };
            slices.push((image, pixmap.width() as f32, pixmap.height() as f32));
        }

        if rendered.placement == PlacementMode::KeepFrame {
            let Some(popover) = &mut self.refhover.shown else {
                return false;
            };
            popover.slices = slices;
            popover.page = rendered.page;
            popover.region = rendered.region;
            popover.target = rendered.target;
            popover.base_zoom = rendered.base_zoom;
            popover.user_zoom = rendered.user_zoom;
            return true;
        }

        let scale = self.dpi_scale();
        let chrome = PopoverChrome::at_scale(scale);
        let inset = chrome.content_inset;
        let (view_w, view_h) = self.viewport_size();
        let margin = VIEWPORT_MARGIN * scale;
        let max_w = self.max_popover_width(Some(rendered.page_screen_width));
        let max_h = (view_h - 2.0 * margin).max(1.0);
        let raw_w = slices.iter().map(|s| s.1).fold(0.0, f32::max) + 2.0 * inset;
        let raw_h = slices.iter().map(|s| s.2).sum::<f32>() + 2.0 * inset;
        let width = raw_w.min(max_w);
        let height = raw_h.min(max_h);
        let gap = resolve::size::POINTER_GAP * scale / 2.0;
        let (x, y) = rendered.at;
        let below = y + gap;
        let top = if below + height <= view_h - margin || y - gap < view_h - below {
            below.min((view_h - margin - height).max(margin.min(view_h - height).max(0.0)))
        } else {
            (y - gap - height).max(margin.min((view_h - height).max(0.0)))
        };
        let max_left = (view_w - margin - width).max(0.0);
        let min_left = margin.min(max_left);
        let left = (x - width / 2.0).clamp(min_left, max_left);
        self.refhover.shown = Some(Popover {
            slices,
            frame: Rect::new(left.round(), top.round(), width.round(), height.round()),
            target: rendered.target,
            visible: self.viewport.visible(),
            page: rendered.page,
            region: rendered.region,
            base_zoom: rendered.base_zoom,
            user_zoom: rendered.user_zoom,
            page_screen_width: rendered.page_screen_width,
        });
        self.refhover.resizing = None;
        true
    }

    /// Draws the popover over everything: a soft drop shadow, opaque rounded paper surface, the
    /// rendered slices inset cleanly inside the corners, and a crisp `0.7 pt` vivid red rounded
    /// bounding box on top. Scrolling or zooming the main document takes it down instead.
    pub(super) fn draw_popover(&mut self) -> Result<(), H::Error> {
        if self.refhover.resizing.is_none()
            && self
                .refhover
                .shown
                .as_ref()
                .is_some_and(|p| p.visible != self.viewport.visible())
        {
            self.refhover.forget();
        }
        let Some(popover) = &self.refhover.shown else {
            return Ok(());
        };
        let scale = self.dpi_scale();
        let chrome = PopoverChrome::at_scale(scale);
        let f = popover.frame;
        let dark = luminance(self.background) < 0.5;
        let shadow = if dark { 0x00_0000 } else { 0x1a_1d23 };
        let paper = self.paper();
        let host = &mut self.host;

        // 1. Soft rounded drop shadow behind the popup.
        let lift = SHADOW * scale;
        host.band(
            &Rect::new(f.x0 + lift / 2.0, f.y0 + lift, f.width(), f.height()),
            chrome.outer_radius,
            shadow,
            Wash::Translucent,
        )?;

        // 2. Opaque rounded paper interior inset past the outer edge of the red stroke so paper
        //    never bleeds outside the border or leaves a seam on the inside at any DPI scale.
        let fill_inset = (chrome.border_px * 0.5).max(0.501);
        let inner = Rect::new(
            f.x0 + fill_inset,
            f.y0 + fill_inset,
            (f.width() - 2.0 * fill_inset).max(0.0),
            (f.height() - 2.0 * fill_inset).max(0.0),
        );
        host.fill_rounded_rect(&inner, chrome.inner_radius, paper)?;

        // 3. Rendered PDF slices inset cleanly inside the rounded paper rect.
        let inset = chrome.content_inset;
        let content_x = f.x0 + inset;
        let content_w = (f.width() - 2.0 * inset).max(0.0);
        let content_bottom = f.y1 - inset;
        let mut y = f.y0 + inset;
        if content_w > 0.0 {
            for (image, w, h) in &popover.slices {
                if y >= content_bottom {
                    break;
                }
                let draw_w = (*w).min(content_w);
                let draw_h = (*h).min(content_bottom - y);
                if draw_w > 0.0 && draw_h > 0.0 {
                    host.image(image, &Rect::new(content_x, y, draw_w, draw_h));
                }
                y += *h;
            }
        }

        // 4. Crisp 0.7 pt vivid red (#FF3030) rounded bounding box on top.
        host.stroke_rounded_rect(&f, chrome.outer_radius, chrome.border_px, POPOVER_BORDER)?;
        Ok(())
    }

    /// The left button went down at client pixel (`x`, `y`): grabbing an edge/corner begins an
    /// interactive resize drag; clicking strictly inside jumps to the target and dismisses the
    /// popover.
    pub(super) fn popover_down(&mut self, x: i32, y: i32) -> PopoverPress {
        let scale = self.dpi_scale();
        let Some((frame, region, target)) = self
            .refhover
            .shown
            .as_ref()
            .map(|p| (p.frame, p.region, p.target))
        else {
            return PopoverPress::Ignored;
        };
        match hit_test_popover(&frame, x as f32, y as f32, scale) {
            PopoverHit::Outside => PopoverPress::Ignored,
            PopoverHit::Edge(edge) => {
                self.refhover.generation += 1;
                self.refhover.asked = Asked::Popover;
                self.refhover.hovered_hit = PopoverHit::Edge(edge);
                self.refhover.resizing = Some(ResizeDrag {
                    edge,
                    start_point: (x as f32, y as f32),
                    start_frame: frame,
                    start_region: region,
                    last_render: None,
                });
                PopoverPress::StartedResize
            }
            PopoverHit::Interior => {
                self.refhover.forget();
                self.go_to_point(target.0, target.1, None);
                PopoverPress::Navigated
            }
        }
    }
}

/// Hit-tests client pixel (`x`, `y`) against the popover's `frame` at `dpi_scale`, classifying
/// the 8 resize edges/corners (including a small tolerance just outside the border), the clickable
/// interior, or outside.
fn hit_test_popover(frame: &Rect, x: f32, y: f32, dpi_scale: f32) -> PopoverHit {
    let scale = if dpi_scale.is_finite() && dpi_scale > 0.0 {
        dpi_scale
    } else {
        1.0
    };
    let w = frame.width();
    let h = frame.height();
    if !(w > 0.0 && h > 0.0 && x.is_finite() && y.is_finite()) {
        return PopoverHit::Outside;
    }
    let out = RESIZE_GRIP_OUT * scale;
    if x < frame.x0 - out || x >= frame.x1 + out || y < frame.y0 - out || y >= frame.y1 + out {
        return PopoverHit::Outside;
    }
    let max_grip = (w.min(h) * 0.45).max(1.0);
    let grip = (RESIZE_GRIP_IN * scale).clamp(1.0, max_grip);
    let left = x < frame.x0 + grip;
    let right = x >= frame.x1 - grip;
    let top = y < frame.y0 + grip;
    let bottom = y >= frame.y1 - grip;
    let edge = match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    };
    match edge {
        Some(edge) => PopoverHit::Edge(edge),
        None => PopoverHit::Interior,
    }
}

/// Viewport and size limits applied while dragging a popover resize edge or corner.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ResizeBounds {
    min_w: f32,
    max_w: f32,
    min_h: f32,
    max_h: f32,
    view_w: f32,
    view_h: f32,
    margin: f32,
}

/// Computes the clamped popover frame during a drag of `edge` by (`dx`, `dy`) physical pixels.
fn resize_frame(start: Rect, edge: ResizeEdge, dx: f32, dy: f32, bounds: ResizeBounds) -> Rect {
    let eff_max_w = bounds.max_w.max(1.0);
    let eff_min_w = bounds.min_w.min(eff_max_w).max(1.0);
    let eff_max_h = bounds.max_h.max(1.0);
    let eff_min_h = bounds.min_h.min(eff_max_h).max(1.0);

    let (mut x0, mut y0, mut x1, mut y1) = (start.x0, start.y0, start.x1, start.y1);

    match edge {
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
            let max_x0 = x1 - eff_min_w;
            let min_x0 = (x1 - eff_max_w).max(bounds.margin);
            x0 = (start.x0 + dx).clamp(min_x0.min(max_x0), max_x0);
        }
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
            let min_x1 = x0 + eff_min_w;
            let max_x1 = (x0 + eff_max_w).min(bounds.view_w - bounds.margin);
            x1 = (start.x1 + dx).clamp(min_x1, max_x1.max(min_x1));
        }
        ResizeEdge::Top | ResizeEdge::Bottom => {}
    }

    match edge {
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
            let max_y0 = y1 - eff_min_h;
            let min_y0 = (y1 - eff_max_h).max(bounds.margin);
            y0 = (start.y0 + dy).clamp(min_y0.min(max_y0), max_y0);
        }
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight => {
            let min_y1 = y0 + eff_min_h;
            let max_y1 = (y0 + eff_max_h).min(bounds.view_h - bounds.margin);
            y1 = (start.y1 + dy).clamp(min_y1, max_y1.max(min_y1));
        }
        ResizeEdge::Left | ResizeEdge::Right => {}
    }

    Rect {
        x0: x0.round(),
        y0: y0.round(),
        x1: x1.round(),
        y1: y1.round(),
    }
}

/// Maps a resized popover frame back into PDF page points, shifting the top-left origin when the
/// user drags the left or top edge and clamping through [`resolve::resized_region`].
fn region_for_resize(
    start_region: PageRect,
    start_frame: Rect,
    new_frame: Rect,
    zoom: f32,
    content_inset: f32,
    page_bounds: PageRect,
) -> PageRect {
    if !zoom.is_finite() || zoom <= 0.0 {
        return PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        };
    }
    let delta_left = (new_frame.x0 - start_frame.x0) / zoom;
    let delta_top = (new_frame.y0 - start_frame.y0) / zoom;
    let shifted_origin = PageRect {
        x0: start_region.x0 + delta_left,
        y0: start_region.y0 + delta_top,
        x1: start_region.x1,
        y1: start_region.y1,
    };
    let content_w = (new_frame.width() - 2.0 * content_inset).max(1.0);
    let content_h = (new_frame.height() - 2.0 * content_inset).max(1.0);
    resolve::resized_region(
        shifted_origin,
        zoom,
        PixelSize::new(content_w, content_h),
        page_bounds,
    )
}

/// Whether a `PageRect` has non-positive width or height.
fn is_empty_page_rect(r: PageRect) -> bool {
    r.x1 <= r.x0 || r.y1 <= r.y0
}

/// Converts a `resolve::RectF` (x, y, dx, dy) to a `PageRect` (x0, y0, x1, y1).
fn rectf_to_page_rect(r: RectF) -> PageRect {
    PageRect {
        x0: r.x,
        y0: r.y,
        x1: r.right(),
        y1: r.bottom(),
    }
}

/// Converts a `PageRect` (x0, y0, x1, y1) to a `resolve::RectF` (x, y, dx, dy).
fn page_rect_to_rectf(r: PageRect) -> RectF {
    RectF::new(r.x0, r.y0, (r.x1 - r.x0).max(0.0), (r.y1 - r.y0).max(0.0))
}

/// Relative brightness of `rgb`, 0 to 1, to tell a dark chrome from a light one.
pub(super) fn luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.0;
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// The hover thread: waits out each request's delay (a newer one replaces it), answers it, and wakes the canvas.
/// It ends when the canvas drops its side.
fn serve(inbox: &Receiver<Ask>, outbox: &Sender<Answer>, waker: &Waker) {
    let mut server = Server::default();
    while let Ok(mut ask) = inbox.recv() {
        loop {
            if ask.delay.is_zero() {
                match inbox.try_recv() {
                    Ok(newer) => ask = newer,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match inbox.recv_timeout(ask.delay) {
                    Ok(newer) => ask = newer,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }
        let shown = server.answer(&ask);
        if outbox
            .send(Answer {
                generation: ask.generation,
                shown,
            })
            .is_err()
        {
            return;
        }
        waker.wake();
    }
}

/// What the hover thread keeps between requests for one document: its lookups and the text it read.
#[derive(Default)]
struct Server {
    /// The document they belong to, by address.
    document: usize,
    lookups: Lookups,
    texts: HashMap<usize, PageText>,
}

impl Server {
    /// Answers an [`Ask`]: resolves & renders a new hover target, re-renders an open popover in
    /// place, or returns `None` to dismiss.
    fn answer(&mut self, ask: &Ask) -> Option<Rendered> {
        let id = Arc::as_ptr(&ask.document) as usize;
        if self.document != id {
            *self = Self {
                document: id,
                ..Self::default()
            };
        }
        match ask.kind {
            AskKind::Dismiss => None,
            AskKind::Rerender {
                page,
                region,
                base_zoom,
                user_zoom,
                page_screen_width,
            } => {
                let shown = Shown {
                    page,
                    region: page_rect_to_rectf(region),
                    continuation: None,
                };
                let zoom = base_zoom * user_zoom;
                if !zoom.is_finite() || zoom <= 0.0 || shown.region.is_empty() {
                    return None;
                }
                let slices = render(ask, &shown, zoom)?;
                Some(Rendered {
                    slices,
                    at: ask.at,
                    target: (
                        page,
                        PagePoint {
                            x: region.x0,
                            y: region.y0,
                        },
                    ),
                    page,
                    region,
                    base_zoom,
                    user_zoom,
                    page_screen_width,
                    placement: PlacementMode::KeepFrame,
                })
            }
            AskKind::Resolve(source) => self.resolve_and_render(ask, source),
        }
    }

    /// Resolves `source` and renders its initial popover slices.
    fn resolve_and_render(&mut self, ask: &Ask, source: Source) -> Option<Rendered> {
        if self.texts.len() > TEXT_CACHE {
            // shortcut: dropped wholesale, and kept through edits (which rarely change the text); key by revision if
            // a stale preview shows.
            self.texts.clear();
        }
        let mut pages = DocPages {
            mupdf: &ask.mupdf,
            bounds: &ask.bounds,
            texts: &mut self.texts,
        };
        let (src_page, src_rect, target) = match source {
            Source::Link { page, rect, target } => {
                let place = target.point.map_or(
                    Place {
                        page: target.page,
                        x: 0.0,
                        y: -1.0,
                    },
                    |p| Place {
                        page: target.page,
                        x: p.x,
                        y: p.y,
                    },
                );
                (
                    page,
                    RectF::new(rect.x0, rect.y0, rect.width(), rect.height()),
                    place,
                )
            }
            Source::Text { page, point } => {
                let found = self
                    .lookups
                    .find(&mut pages, page, point.x as i32, point.y as i32)?;
                (page, found.source.into(), found.target)
            }
        };
        let shown = resolve::shown(&mut pages, src_page, src_rect, target)?;
        let zoom = resolve::zoom(&shown, ask.page_zoom, ask.room);
        let slices = render(ask, &shown, zoom)?;
        let region = rectf_to_page_rect(shown.region);
        let target = (
            shown.page,
            PagePoint {
                x: shown.region.x,
                y: shown.region.y,
            },
        );
        Some(Rendered {
            slices,
            at: ask.at,
            target,
            page: shown.page,
            region,
            base_zoom: zoom,
            user_zoom: 1.0,
            page_screen_width: ask.room.page.dx,
            placement: PlacementMode::Auto,
        })
    }
}

/// Renders `shown`'s slices at `zoom`, upright, in the reader's page colours.
fn render(ask: &Ask, shown: &Shown, zoom: f32) -> Option<Vec<Pixmap>> {
    let bounds = ask.bounds.get(shown.page)?;
    let list = display_list(ask, shown.page)?;
    let ctm = Transform {
        zoom,
        rotation: Rotation::Upright,
    }
    .matrix(bounds);
    let mut slices = Vec::with_capacity(2);
    for region in std::iter::once(shown.region).chain(shown.continuation) {
        let area = DeviceRect {
            x0: ((region.x - bounds.x0) * zoom).floor() as i32,
            y0: ((region.y - bounds.y0) * zoom).floor() as i32,
            x1: ((region.right() - bounds.x0) * zoom).ceil() as i32,
            y1: ((region.bottom() - bounds.y0) * zoom).ceil() as i32,
        };
        let recolor = ask.colors.map(|colors| Recolor {
            colors,
            images: Arc::default(),
        });
        let job = pool().render_tile(&list, ctm, area, Priority::Visible, recolor);
        match job.wait() {
            Ok(pixmap) => slices.push(pixmap),
            Err(err) => {
                log::warn!("cannot render a hover preview: {err}");
                return None;
            }
        }
    }
    Some(slices)
}

/// Page `page`'s display list: the one a view already recorded, or a new one.
fn display_list(ask: &Ask, page: usize) -> Option<Arc<DisplayList>> {
    if let Some(list) = ask.document.display_list(page) {
        return Some(list);
    }
    let document = ask.mupdf.lock().expect("document lock poisoned");
    document
        .load_page(page)
        .and_then(|page| page.display_list(None))
        .inspect_err(|err| log::warn!("cannot record page {page} for a hover preview: {err}"))
        .ok()
        .map(Arc::new)
}

/// The document's pages for the resolver: text read from MuPDF on this thread, boxes as the canvas lays them out.
struct DocPages<'a> {
    mupdf: &'a SharedMupdf,
    bounds: &'a [PageRect],
    texts: &'a mut HashMap<usize, PageText>,
}

impl Pages for DocPages<'_> {
    fn count(&self) -> usize {
        self.bounds.len()
    }

    fn text(&mut self, page: usize) -> Option<PageText> {
        if page >= self.bounds.len() {
            return None;
        }
        if let Some(text) = self.texts.get(&page) {
            return Some(text.clone());
        }
        let layout = {
            let document = self.mupdf.lock().expect("document lock poisoned");
            document
                .load_page(page)
                .and_then(|p| p.text(None))
                .map(|t| t.layout())
        };
        match layout {
            Ok(layout) => {
                let text = PageText::from_layout(&layout);
                self.texts.insert(page, text.clone());
                Some(text)
            }
            Err(err) => {
                log::debug!("cannot read page {page}'s text for a hover preview: {err}");
                None
            }
        }
    }

    fn mediabox(&mut self, page: usize) -> Option<RectF> {
        self.bounds
            .get(page)
            .map(|b| RectF::new(b.x0, b.y0, b.width(), b.height()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_covers_all_eight_edges_interior_and_outside() {
        let frame = Rect::new(100.0, 200.0, 300.0, 200.0); // x: 100..400, y: 200..400
        let scale = 1.0;

        // All 4 corners
        assert_eq!(
            hit_test_popover(&frame, 102.0, 202.0, scale),
            PopoverHit::Edge(ResizeEdge::TopLeft)
        );
        assert_eq!(
            hit_test_popover(&frame, 398.0, 202.0, scale),
            PopoverHit::Edge(ResizeEdge::TopRight)
        );
        assert_eq!(
            hit_test_popover(&frame, 102.0, 398.0, scale),
            PopoverHit::Edge(ResizeEdge::BottomLeft)
        );
        assert_eq!(
            hit_test_popover(&frame, 398.0, 398.0, scale),
            PopoverHit::Edge(ResizeEdge::BottomRight)
        );

        // All 4 straight edges (and outer 2 DIP tolerance)
        assert_eq!(
            hit_test_popover(&frame, 250.0, 202.0, scale),
            PopoverHit::Edge(ResizeEdge::Top)
        );
        assert_eq!(
            hit_test_popover(&frame, 250.0, 398.0, scale),
            PopoverHit::Edge(ResizeEdge::Bottom)
        );
        assert_eq!(
            hit_test_popover(&frame, 99.0, 300.0, scale),
            PopoverHit::Edge(ResizeEdge::Left)
        );
        assert_eq!(
            hit_test_popover(&frame, 401.0, 300.0, scale),
            PopoverHit::Edge(ResizeEdge::Right)
        );

        // Interior & strictly outside
        assert_eq!(
            hit_test_popover(&frame, 250.0, 300.0, scale),
            PopoverHit::Interior
        );
        assert_eq!(
            hit_test_popover(&frame, 90.0, 300.0, scale),
            PopoverHit::Outside
        );
        assert_eq!(
            hit_test_popover(&frame, 250.0, 410.0, scale),
            PopoverHit::Outside
        );

        // Cursor mapping for all 8 edges
        assert_eq!(ResizeEdge::Left.cursor(), Cursor::ResizeEw);
        assert_eq!(ResizeEdge::Right.cursor(), Cursor::ResizeEw);
        assert_eq!(ResizeEdge::Top.cursor(), Cursor::ResizeNs);
        assert_eq!(ResizeEdge::Bottom.cursor(), Cursor::ResizeNs);
        assert_eq!(ResizeEdge::TopLeft.cursor(), Cursor::ResizeNwse);
        assert_eq!(ResizeEdge::BottomRight.cursor(), Cursor::ResizeNwse);
        assert_eq!(ResizeEdge::TopRight.cursor(), Cursor::ResizeNesw);
        assert_eq!(ResizeEdge::BottomLeft.cursor(), Cursor::ResizeNesw);
    }

    #[test]
    fn resize_frame_clamps_width_and_height_to_limits() {
        let start = Rect::new(200.0, 200.0, 300.0, 200.0); // x: 200..500, y: 200..400
        let bounds = ResizeBounds {
            min_w: 160.0,
            max_w: 600.0,
            min_h: 100.0,
            max_h: 500.0,
            view_w: 1200.0,
            view_h: 900.0,
            margin: 8.0,
        };

        // Pulling right edge far past max_w clamps to max_w while keeping x0 fixed.
        let wide = resize_frame(start, ResizeEdge::Right, 900.0, 0.0, bounds);
        assert_eq!(wide.x0, 200.0);
        assert_eq!(wide.width(), bounds.max_w);

        // Pulling right edge far left clamps to min_w while keeping x0 fixed.
        let narrow = resize_frame(start, ResizeEdge::Right, -250.0, 0.0, bounds);
        assert_eq!(narrow.x0, 200.0);
        assert_eq!(narrow.width(), bounds.min_w);

        // Pulling left edge far left respects viewport margin and pins x1.
        let left_clamped = resize_frame(start, ResizeEdge::Left, -500.0, 0.0, bounds);
        assert_eq!(left_clamped.x1, 500.0);
        assert_eq!(left_clamped.x0, bounds.margin);

        // Pulling bottom-right corner clamps both axes simultaneously.
        let corner = resize_frame(start, ResizeEdge::BottomRight, -400.0, 800.0, bounds);
        assert_eq!(corner.x0, 200.0);
        assert_eq!(corner.y0, 200.0);
        assert_eq!(corner.width(), bounds.min_w);
        assert_eq!(corner.height(), bounds.max_h);
    }

    #[test]
    fn red_border_chrome_geometry_is_exact_0_7pt_across_dpis() {
        assert_eq!(POPOVER_BORDER, 0xFF_3030);

        let at_100 = PopoverChrome::at_scale(1.0);
        assert!((at_100.border_px - (0.7 * 96.0 / 72.0)).abs() < 1e-5);
        assert!((at_100.outer_radius - 6.0).abs() < 1e-5);
        assert!((at_100.inner_radius - (6.0 - at_100.border_px)).abs() < 1e-5);
        assert!(at_100.content_inset >= at_100.border_px + 2.0);

        let at_150 = PopoverChrome::at_scale(1.5);
        assert!((at_150.border_px - 1.4).abs() < 1e-5);
        assert!((at_150.outer_radius - 9.0).abs() < 1e-5);
        assert!((at_150.inner_radius - 7.6).abs() < 1e-5);

        let at_200 = PopoverChrome::at_scale(2.0);
        assert!((at_200.border_px - (0.7 * 192.0 / 72.0)).abs() < 1e-5);
        assert!((at_200.outer_radius - 12.0).abs() < 1e-5);
    }
}
