//! Citation and equation hover on the canvas (plan R.25): hold the pointer on a link inside the document, or on a
//! citation in plain text such as `[12]` or `(Smith, 2020)`, and a popover shows the entry, equation or figure it
//! points at, drawn from the page itself. The model is `ecclesia-features`' `read::refhover`.
//!
//! One thread per canvas does the waiting and the work: it holds each request for the hover delay, dropping it for a
//! newer one, then finds the target (walking the document's text when there is no link), picks the region and
//! renders it on the render pool. The canvas only asks, and draws what comes back.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use ecclesia_core::darkmode::PageColors;
use ecclesia_core::document::{Document, SharedMupdf};
use ecclesia_core::render::{Priority, Recolor};
use ecclesia_core::shared::pool;
use ecclesia_core::transform::{Rotation, Transform};
use ecclesia_features::read::refhover::resolve::{self, Lookups, Pages, Place, Room, Shown};
use ecclesia_features::read::refhover::{PageText, RectF};
use ecclesia_mupdf::{DeviceRect, DisplayList, Link, PagePoint, PageRect, Pixmap};

use super::{PageCanvas, Waker};
use crate::Rect;
use crate::page::host::{Host, Wash};

/// How long the pointer rests before a popover shows, as in Enhanced (its `CitationHoverDelay`).
pub(in crate::page) const DEFAULT_DELAY: Duration = Duration::from_millis(300);
/// How long a popover stays after the pointer leaves, so it can cross the gap onto the popover.
const LINGER: Duration = Duration::from_millis(250);
/// Pages of text the hover thread keeps between requests.
const TEXT_CACHE: usize = 256;
/// The popover's corner radius, in pixels at 96 dpi.
const RADIUS: f32 = 6.0;
/// The popover's shadow offset, in pixels at 96 dpi.
const SHADOW: f32 = 3.0;

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
    /// `None`: the pointer left; answer only to say so, after [`LINGER`].
    source: Option<Source>,
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
    /// The target, for a click on the popover to go to.
    target: (usize, PagePoint),
}

/// A popover on screen.
pub(super) struct Popover<I> {
    slices: Vec<(I, f32, f32)>,
    /// Its frame, in view pixels.
    frame: Rect,
    target: (usize, PagePoint),
    /// The layout on screen when it showed: scrolling or zooming takes it down.
    visible: Rect,
}

/// The canvas's hover state.
pub(super) struct RefHover<I> {
    /// `None`: hover is off.
    delay: Option<Duration>,
    asks: Option<Sender<Ask>>,
    answers: Option<Receiver<Answer>>,
    asked: Asked,
    generation: u64,
    /// Answered but not shown yet: its slices upload on the canvas's thread.
    ready: Option<Rendered>,
    shown: Option<Popover<I>>,
}

impl<I> Default for RefHover<I> {
    fn default() -> Self {
        Self {
            delay: Some(DEFAULT_DELAY),
            asks: None,
            answers: None,
            asked: Asked::Nothing,
            generation: 0,
            ready: None,
            shown: None,
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

    /// Whether client pixel (`x`, `y`) is over the popover.
    pub(super) fn over_popover(&self, x: i32, y: i32) -> bool {
        self.refhover
            .shown
            .as_ref()
            .is_some_and(|p| contains(&p.frame, x as f32, y as f32))
    }

    /// The pointer moved to client pixel `at` (`None`: off the canvas), over `link` if any: asks the hover thread
    /// about what is under it when that changed. Over the popover, it stays.
    pub(super) fn hover_moved(&mut self, at: Option<(i32, i32)>, link: Option<&Link>) {
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
        let Some(ask) = self.ask(source, delay, at.unwrap_or_default()) else {
            return;
        };
        self.send_ask(ask);
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

    /// The request for `source`, with what the thread needs to answer it.
    fn ask(&self, source: Option<Source>, delay: Duration, (x, y): (i32, i32)) -> Option<Ask> {
        let document = self.document.clone()?;
        let mupdf = document
            .mupdf()
            .inspect_err(|err| log::warn!("cannot open the document for a hover preview: {err}"))
            .ok()?;
        let src_page = match source {
            Some(Source::Link { page, .. } | Source::Text { page, .. }) => Some(page),
            None => None,
        };
        let page_rect = src_page
            .and_then(|page| self.layout.page(page))
            .map(|page| self.viewport.to_screen(&page))
            .map_or(RectF::default(), |r| {
                RectF::new(r.x0, r.y0, r.width(), r.height())
            });
        let (width, _) = self.viewport_size();
        Some(Ask {
            generation: self.refhover.generation,
            source,
            delay: if source.is_some() { delay } else { LINGER },
            document,
            mupdf,
            bounds: Arc::new(self.bounds.clone()),
            page_zoom: self.viewport.scale(),
            room: Room {
                screen_width: width,
                page: page_rect,
                pointer_y: y as f32,
                scale: self.dpi_scale(),
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
                    changed |= self.refhover.shown.take().is_some();
                }
            }
        }
        changed | self.upload_popover()
    }

    /// Uploads a rendered popover's slices and places it: below the pointer, or above it when there is more room
    /// there, within the view. Returns whether one is now shown.
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
        let scale = self.dpi_scale();
        let border = resolve::size::BORDER * scale;
        let width = slices.iter().map(|s| s.1).fold(0.0, f32::max) + 2.0 * border;
        let height = slices.iter().map(|s| s.2).sum::<f32>() + 2.0 * border;
        let (view_w, view_h) = self.viewport_size();
        let gap = resolve::size::POINTER_GAP * scale / 2.0;
        let (x, y) = rendered.at;
        let below = y + gap;
        let top = if below + height <= view_h || y - gap < view_h - below {
            below.min((view_h - height).max(0.0))
        } else {
            (y - gap - height).max(0.0)
        };
        let left = (x - width / 2.0).clamp(0.0, (view_w - width).max(0.0));
        self.refhover.shown = Some(Popover {
            slices,
            frame: Rect::new(left.round(), top.round(), width.round(), height.round()),
            target: rendered.target,
            visible: self.viewport.visible(),
        });
        true
    }

    /// Draws the popover over everything: a soft shadow, a hairline frame in the chrome's tone, and the slices
    /// stacked on the paper. Scrolling or zooming since it showed takes it down instead.
    pub(super) fn draw_popover(&mut self) -> Result<(), H::Error> {
        if self
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
        let (radius, border) = (RADIUS * scale, resolve::size::BORDER * scale);
        let f = popover.frame;
        let dark = luminance(self.background) < 0.5;
        let (edge, shadow) = if dark {
            (0x5c_6068, 0x00_0000)
        } else {
            (0xc4_c8cf, 0x1a_1d23)
        };
        let paper = self.paper();
        let host = &mut self.host;
        let lift = SHADOW * scale;
        host.band(
            &Rect::new(f.x0 + lift / 2.0, f.y0 + lift, f.width(), f.height()),
            radius,
            shadow,
            Wash::Translucent,
        )?;
        host.band(&f, radius, edge, Wash::Darkest)?;
        let hair = scale.max(1.0).round();
        let inner = Rect::new(
            f.x0 + hair,
            f.y0 + hair,
            f.width() - 2.0 * hair,
            f.height() - 2.0 * hair,
        );
        host.band(&inner, (radius - hair).max(0.0), paper, Wash::Darkest)?;
        let mut y = f.y0 + border;
        for (image, w, h) in &popover.slices {
            host.image(image, &Rect::new(f.x0 + border, y, *w, *h));
            y += h;
        }
        Ok(())
    }

    /// The left button went down at client pixel (`x`, `y`): on the popover, it goes to the target and takes the
    /// popover down. Returns whether the popover took the press.
    pub(super) fn popover_down(&mut self, x: i32, y: i32) -> bool {
        if !self.over_popover(x, y) {
            return false;
        }
        let target = self.refhover.shown.as_ref().map(|p| p.target);
        self.refhover.forget();
        if let Some((page, point)) = target {
            self.go_to_point(page, point, None);
        }
        true
    }
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    r.x0 <= x && x < r.x1 && r.y0 <= y && y < r.y1
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
            match inbox.recv_timeout(ask.delay) {
                Ok(newer) => ask = newer,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let shown = ask.source.and_then(|source| server.answer(&ask, source));
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
    /// The popover for `source`: its target, region and slices, rendered.
    fn answer(&mut self, ask: &Ask, source: Source) -> Option<Rendered> {
        let id = Arc::as_ptr(&ask.document) as usize;
        if self.document != id {
            *self = Self {
                document: id,
                ..Self::default()
            };
        }
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
