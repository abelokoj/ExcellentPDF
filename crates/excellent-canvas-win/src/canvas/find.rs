//! Find in document on the canvas (plan F.1). The model is `excellent-features`' `search::find`; this reads the
//! pages' text on the render workers, from the reader's page round to it, keeps the hits, draws them and tells the app
//! (the `found` event), which shows the find bar and the result list.
//!
//! Pages are read a few at a time below the screen's tiles and text, and taken in the order they were asked for, so
//! hits arrive in search order and the first one after the reader's place is known as soon as its page is read.

use std::collections::VecDeque;
use std::sync::Arc;

use excellent_core::render::{Job, Priority};
use excellent_core::shared::pool;
use excellent_features::search::find::{self, Direction, Found, HitId, Hits, Options, Query};
use excellent_mupdf::{PageRect, Quad, TextLayout};
use excellent_text::PageText;
use serde_json::json;

use super::Canvas;
use crate::gfx::{Gfx, Wash};
use crate::placement::Placement;

/// Pages read at once.
const IN_FLIGHT: usize = 4;
/// Every hit: a light yellow that keeps black text black under it (drawn darkest-wins, as the selection is).
const HIT: u32 = 0xffe680;
/// The reader's hit: a stronger orange.
const CURRENT: u32 = 0xffb054;
/// Where a hit scrolled to is put, as a share of the window's height from the top.
const SHOW_AT: f32 = 0.33;

/// A search under way or done.
#[derive(Debug)]
struct Search {
    /// The app's number for it, sent back with every `found` event so a late answer to an old query is known.
    id: u64,
    query: Query,
    /// Pages still to read, in search order.
    order: VecDeque<usize>,
    /// Pages being read, in search order.
    running: VecDeque<(usize, Job<TextLayout>)>,
    searched: usize,
    pages: usize,
    hits: Hits,
}

/// The canvas's find state.
#[derive(Debug, Default)]
pub(super) struct Finder {
    search: Option<Search>,
    /// A search whose pages were laid out again (a reflowed book): run again once the new layout is in.
    again: Option<(u64, Query)>,
}

impl Finder {
    /// Stops the search; it runs again when the pages are back (`keep`), or not at all.
    pub(super) fn forget(&mut self, keep: bool) {
        let search = self.search.take();
        self.again = if keep {
            search.map(|search| (search.id, search.query))
        } else {
            None
        };
    }
}

/// The box round `quads`.
fn bounds(quads: &[Quad]) -> Option<PageRect> {
    let mut points = quads.iter().flat_map(|q| [q.ul, q.ur, q.ll, q.lr]);
    let first = points.next()?;
    let mut rect = PageRect {
        x0: first.x,
        y0: first.y,
        x1: first.x,
        y1: first.y,
    };
    for p in points {
        rect.x0 = rect.x0.min(p.x);
        rect.y0 = rect.y0.min(p.y);
        rect.x1 = rect.x1.max(p.x);
        rect.y1 = rect.y1.max(p.y);
    }
    Some(rect)
}

impl Canvas {
    /// Searches the document for `text` (F.1) as `options` say, from the reader's page round to it; the app's number
    /// for the search is `id`. Blank text clears the search.
    pub(crate) fn find(&mut self, id: u64, text: &str, options: Options) {
        self.finder.forget(false);
        match Query::new(text, options) {
            Some(query) => self.start_find(id, query),
            None => self.invalidate(),
        }
    }

    fn start_find(&mut self, id: u64, query: Query) {
        let pages = self.layout.len();
        let start = self.viewport.current_page(&self.layout).unwrap_or(0);
        self.finder.search = Some(Search {
            id,
            query,
            order: find::page_order(start, pages).collect(),
            running: VecDeque::new(),
            searched: 0,
            pages,
            hits: Hits::new(),
        });
        self.feed_find();
        self.report_found(Vec::new());
        self.invalidate();
    }

    /// Runs a search again once its pages were laid out again.
    pub(super) fn find_again(&mut self) {
        if self.document.is_none() || self.layout.is_empty() {
            return;
        }
        if let Some((id, query)) = self.finder.again.take() {
            self.start_find(id, query);
        }
    }

    /// Stops the search and takes its hits off the pages.
    pub(crate) fn find_clear(&mut self) {
        self.finder.forget(false);
        self.invalidate();
    }

    /// Asks the workers for more pages' text.
    fn feed_find(&mut self) {
        let Some(document) = &self.document else {
            return;
        };
        let Some(search) = &mut self.finder.search else {
            return;
        };
        if search.order.is_empty() || search.running.len() >= IN_FLIGHT {
            return;
        }
        let mupdf = match document.mupdf() {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::error!(
                    "cannot open {:?} again to search it: {err}",
                    document.path()
                );
                return;
            }
        };
        while search.running.len() < IN_FLIGHT
            && let Some(page) = search.order.pop_front()
        {
            let job = pool().read_text(&mupdf, page, Priority::Nearby);
            let waker = Arc::clone(&self.waker);
            job.when_done(move || waker.wake());
            search.running.push_back((page, job));
        }
    }

    /// Takes the pages read, in search order; whether any hits were added.
    pub(super) fn take_found(&mut self) -> bool {
        let Some(search) = &mut self.finder.search else {
            return false;
        };
        let mut added = Vec::new();
        let mut read_any = false;
        while let Some((page, job)) = search.running.front_mut() {
            let page = *page;
            let Some(result) = job.try_take() else {
                break;
            };
            search.running.pop_front();
            read_any = true;
            search.searched += 1;
            let layout = match result {
                Ok(layout) => layout,
                Err(err) => {
                    if !err.is_aborted() {
                        log::warn!("cannot read page {page} to search it: {err}");
                    }
                    continue;
                }
            };
            let found = find::find_on_page(&PageText::new(layout), &search.query);
            added.extend((0..found.len()).map(|nth| HitId { page, nth }));
            search.hits.add(page, found);
        }
        if !read_any {
            return false;
        }
        // The first hit found is the first one from the reader's page on: show it, as browsers do.
        let first = (search.hits.current().is_none())
            .then(|| added.first().copied())
            .flatten();
        self.feed_find();
        if let Some(first) = first
            && let Some(search) = &mut self.finder.search
        {
            search.hits.set_current(first);
            self.show_hit(first);
        }
        self.report_found(added);
        true
    }

    /// Goes to the next or previous hit (F3, Shift+F3).
    pub(crate) fn find_step(&mut self, direction: Direction) {
        let page = self.viewport.current_page(&self.layout).unwrap_or(0);
        let Some(search) = &mut self.finder.search else {
            return;
        };
        if let Some(id) = search.hits.step(direction, page) {
            self.show_hit(id);
            self.report_found(Vec::new());
        }
    }

    /// Goes to hit `id`, from the result list.
    pub(crate) fn find_show(&mut self, id: HitId) {
        let Some(search) = &mut self.finder.search else {
            return;
        };
        if search.hits.set_current(id) {
            self.show_hit(id);
            self.report_found(Vec::new());
        }
    }

    /// Scrolls hit `id` into view, a third of the way down, unless it is on screen already.
    fn show_hit(&mut self, id: HitId) {
        let Some(rect) = self
            .finder
            .search
            .as_ref()
            .and_then(|search| search.hits.get(id))
            .and_then(|found| bounds(&found.quads))
        else {
            return;
        };
        let screen = self.screen();
        if let Some(on_screen) = self.screen_box(id.page, &rect)
            && on_screen.x0 >= screen.x0
            && on_screen.y0 >= screen.y0
            && on_screen.x1 <= screen.x1
            && on_screen.y1 <= screen.y1
        {
            self.invalidate();
            return;
        }
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, id.page)
        else {
            return;
        };
        let (x, y) = placement.to_layout(excellent_mupdf::PagePoint {
            x: rect.x0,
            y: rect.y0,
        });
        self.viewport.scroll_to_point(id.page, x, y, &self.layout);
        self.viewport
            .scroll_by(0.0, -self.viewport.height() * SHOW_AT, &self.layout);
        self.invalidate();
    }

    /// Tells the app where the search is (`found`): `{id, searched, pages, total, current, hits}`, `current` the
    /// reader's hit's place in document order (from 0, or `null`), `hits` the hits new since the last event, each
    /// `{page, nth, before, hit, after}`.
    fn report_found(&self, added: Vec<HitId>) {
        let Some(search) = &self.finder.search else {
            return;
        };
        let hits: Vec<_> = added
            .into_iter()
            .filter_map(|id| {
                let found = search.hits.get(id)?;
                Some(json!({
                    "page": id.page,
                    "nth": id.nth,
                    "before": found.snippet.before,
                    "hit": found.snippet.hit,
                    "after": found.snippet.after,
                }))
            })
            .collect();
        self.emit(
            "found",
            json!({
                "id": search.id,
                "searched": search.searched,
                "pages": search.pages,
                "total": search.hits.len(),
                "current": search.hits.index_of_current(),
                "hits": hits,
            }),
        );
    }

    /// The hits on the screen, ready to draw.
    pub(super) fn found_on_screen(&self) -> FoundOnScreen {
        let mut on_screen = FoundOnScreen::default();
        let Some(search) = &self.finder.search else {
            return on_screen;
        };
        let current = search.hits.current();
        let FoundOnScreen { others, mine } = &mut on_screen;
        let visible = self.viewport.visible();
        for (page, _) in self.layout.pages_in(&visible) {
            let Some(placement) =
                Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
            else {
                continue;
            };
            for (nth, found) in search.hits.on_page(page).iter().enumerate() {
                let into = if current == Some(HitId { page, nth }) {
                    &mut *mine
                } else {
                    &mut *others
                };
                into.extend(screen_quads(self, &placement, found));
            }
        }
        on_screen
    }
}

/// The hits on the screen, in client pixels.
#[derive(Debug, Default)]
pub(super) struct FoundOnScreen {
    others: Vec<[(f32, f32); 4]>,
    /// The reader's hit.
    mine: Vec<[(f32, f32); 4]>,
}

impl FoundOnScreen {
    /// Draws them, the reader's in a stronger colour.
    pub(super) fn draw(&self, gfx: &Gfx, wash: Wash) -> windows::core::Result<()> {
        gfx.highlight(&self.others, HIT, wash)?;
        gfx.highlight(&self.mine, CURRENT, wash)
    }
}

/// `found`'s quads on screen.
fn screen_quads<'a>(
    canvas: &'a Canvas,
    placement: &'a Placement,
    found: &'a Found,
) -> impl Iterator<Item = [(f32, f32); 4]> + 'a {
    found.quads.iter().map(move |quad| {
        [quad.ul, quad.ur, quad.lr, quad.ll].map(|p| {
            let (x, y) = placement.to_layout(p);
            canvas.viewport.to_screen_point(x, y)
        })
    })
}
