//! Synchronized scrolling (V.8): while it is on, a pane the reader moves leads and the window's other panes follow,
//! by the same page, the same position through the document, or the pages the reader paired
//! ([`excellent_features::split`]); the zoom follows too when the reader asks. Off by default; how it follows lives in
//! `settings.toml`.
//!
//! A pane follows through its canvas's `follow` command, which moves it without a jump on its history; the view the
//! canvas then reports says it followed, so it leads nothing and the panes never chase each other.

use excellent_features::split::{self, Follow, Marks, Position};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::{CELLS, Panes};

/// `settings.toml`'s key for how a pane follows: `"page"` (the default), `"position"` or `"pairs"`.
pub(crate) const SYNC_SCROLL: &str = "sync_scroll";
/// `settings.toml`'s key for whether the zoom follows too (default off).
pub(crate) const SYNC_ZOOM: &str = "sync_zoom";

/// The sync's state, shared by the panes.
#[derive(Clone, Copy)]
pub(crate) struct SyncScroll {
    on: Signal<bool>,
    marks: Signal<Marks>,
    menu_open: Signal<bool>,
    /// Each pane's last reported view, to line the others up with the active one when sync starts.
    last: Stored<[Option<JsonValue>; CELLS]>,
}

impl SyncScroll {
    pub(crate) fn new() -> Self {
        Self {
            on: signal(false),
            marks: signal(Marks::default()),
            menu_open: signal(false),
            last: stored(std::array::from_fn(|_| None)),
        }
    }
}

impl Panes {
    /// Whether synchronized scrolling is on (tracked).
    pub(crate) fn sync_on(self) -> bool {
        self.sync.on.get()
    }

    /// How the panes follow (tracked).
    pub(crate) fn sync_follow(self) -> Follow {
        self.live.read(|s| Follow::parse(s.text(SYNC_SCROLL)))
    }

    /// Whether the zoom follows too (tracked).
    pub(crate) fn sync_zoom(self) -> bool {
        self.live.read(|s| s.flag(SYNC_ZOOM)).unwrap_or(false)
    }

    /// Whether any page pairs are marked (tracked).
    pub(crate) fn has_page_pairs(self) -> bool {
        self.sync.marks.with(|marks| !marks.is_empty())
    }

    /// Whether the header's sync menu is open: the page canvases hide under it.
    pub(crate) fn sync_menu_open(self) -> bool {
        self.sync.menu_open.get()
    }

    /// The header's sync menu opened or closed.
    pub(crate) fn set_sync_menu_open(self, open: bool) {
        self.sync.menu_open.set(open);
    }

    /// Turns synchronized scrolling on or off; on, the other panes line up with the active one at once.
    pub(crate) fn toggle_sync(self) {
        let on = !self.sync.on.get_untracked();
        self.sync.on.set(on);
        if on {
            self.lead_from_active();
        }
    }

    /// Follows by `follow` from now on, and lines the panes up by it.
    pub(crate) fn set_sync_follow(self, follow: Follow) {
        self.live.change(|s| {
            s.set_text(
                SYNC_SCROLL,
                (follow != Follow::default()).then_some(follow.name()),
            );
        });
        self.lead_from_active();
    }

    /// The zoom follows, or stops following.
    pub(crate) fn toggle_sync_zoom(self) {
        let zoom = !untracked(|| self.sync_zoom());
        self.live
            .change(|s| s.set_flag(SYNC_ZOOM, zoom.then_some(true)));
        self.lead_from_active();
    }

    /// Pairs the pages every pane shows now, follows by the pairs, and turns sync on.
    pub(crate) fn pair_pages(self) {
        let pages: Vec<Option<usize>> = self
            .slots
            .iter()
            .map(|pane| {
                let shown = untracked(|| self.is_open(pane.cell) && pane.tabs.has_shown());
                shown.then(|| untracked(|| pane.pages.current()))
            })
            .collect();
        self.sync.marks.update(|marks| marks.mark(pages));
        self.live
            .change(|s| s.set_text(SYNC_SCROLL, Some(Follow::Pairs.name())));
        self.sync.on.set(true);
    }

    /// Forgets the page pairs.
    pub(crate) fn clear_page_pairs(self) {
        self.sync.marks.update(Marks::clear);
    }

    /// The canvas of the pane in `cell` reported its view: while sync is on, the other panes follow it, unless it
    /// moved to follow.
    pub(crate) fn on_view(self, cell: usize, data: &JsonValue) {
        self.sync.last.with(|last| last[cell] = Some(data.clone()));
        let followed = data.get("follow").and_then(JsonValue::as_bool) == Some(true);
        if !followed && untracked(|| self.sync.on.get() && self.is_split()) {
            self.lead(cell, data);
        }
    }

    fn lead_from_active(self) {
        if !untracked(|| self.sync.on.get() && self.is_split()) {
            return;
        }
        let cell = self.active().cell;
        if let Some(data) = self.sync.last.with(|last| last[cell].clone()) {
            self.lead(cell, &data);
        }
    }

    /// Moves every other open pane to follow the pane in `cell`, whose view is `data`.
    fn lead(self, cell: usize, data: &JsonValue) {
        let number = |key: &str| {
            data.get(key)
                .and_then(JsonValue::as_f64)
                .filter(|n| n.is_finite())
        };
        let Some(page) = data
            .get("page")
            .and_then(JsonValue::as_u64)
            .and_then(|page| usize::try_from(page).ok())
        else {
            return;
        };
        let from = Position {
            page,
            across: number("across").unwrap_or(0.0) as f32,
            down: number("down").unwrap_or(0.0) as f32,
        };
        let (follow, zoom) = untracked(|| (self.sync_follow(), self.sync_zoom()));
        let leader = self.slots[cell];
        let from_pages = leader.pages.heights();
        for pane in self.slots {
            if pane.cell == cell || !untracked(|| self.is_open(pane.cell) && pane.tabs.has_shown())
            {
                continue;
            }
            let pairs = self
                .sync
                .marks
                .with_untracked(|marks| marks.pairs(cell, pane.cell));
            let Some(to) = split::follow(follow, from, &from_pages, &pane.pages.heights(), &pairs)
            else {
                continue;
            };
            let mut args =
                serde_json::json!({ "page": to.page, "across": to.across, "down": to.down });
            if zoom {
                args["fit"] = data.get("fit").cloned().unwrap_or(JsonValue::Null);
                args["zoom"] = data.get("zoom").cloned().unwrap_or(JsonValue::Null);
            }
            pane.canvas.command("follow", args);
        }
    }
}
