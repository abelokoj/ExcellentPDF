//! The split view (plan §11.15; V.1 to V.7): up to four panes in a two-by-two grid ([`layout`]), each with its own
//! tabs, its own page canvas and what follows them (the page, the links, the thumbnails, the outline, the bookmarks
//! and the edits). One pane is active (V.5): the title bar's tab strip, the sidebar and every command are its.
//!
//! A new pane shows the shown tab again at the same place (V.1) or other files (V.2: the open dialog, a drop on an
//! edge of the page area, `--split` on the command line); a tab moves to another pane in its view (V.3); closing a
//! pane gives its tabs to the one beside it or closes them (V.6), and a pane left with no tab closes. The layout and
//! every pane's tabs are saved with the session (V.7, [`crate::session`]).
//!
//! Two panes showing one file share its document (the registry of `excellent_core::shared`), so it is read once and
//! an edit made in one is the other's too.

mod grid;
mod layout;

use std::path::PathBuf;
use std::time::Duration;

use nsh_core::geometry::Points;
use nsh_core::prelude::*;

use crate::bookmarks::BookmarkState;
use crate::editing::Editing;
use crate::full_screen::FullScreen;
use crate::goto::PageState;
use crate::links::LinkState;
use crate::open::{Launch, Split as LaunchSplit};
use crate::outline::OutlineState;
use crate::reading::Reading;
use crate::recent::RecentState;
use crate::session::Restored;
use crate::settings::Live;
use crate::tabs::{self, Closed, Step, TabState, Tabs};
use crate::thumbnails::Thumbnails;

pub(crate) use grid::pane_grid;
pub(crate) use layout::{CELLS, Layout, Side};

/// `settings.toml`'s key for what closing a pane does with its tabs: `"move"` (the default) or `"close"`.
pub(crate) const CLOSE_PANE_TABS: &str = "close_pane_tabs";

/// How long after a file drag leaves one drop target the window waits for it to enter another before it stops
/// showing the edge targets: leaving one target for the next comes as a leave, then an enter.
const DRAG_GONE: Duration = Duration::from_millis(150);

/// What closing a pane does with its tabs (V.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClosingTabs {
    /// They move to the pane beside it.
    #[default]
    Move,
    /// They close, and can be reopened from the pane beside it.
    Close,
}

impl ClosingTabs {
    /// The setting's value, or the default.
    pub(crate) fn parse(id: Option<&str>) -> Self {
        match id {
            Some("close") => Self::Close,
            _ => Self::Move,
        }
    }
}

/// What a pane command does (V.1 to V.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneAction {
    /// The shown tab in a new pane on that side, at the same place (V.1).
    Split(Side),
    /// Files from the open dialog in a new pane on the right (V.2).
    OpenInSplit,
    /// The shown tab to the next or previous pane, or to a new one on that side when there is one pane (V.3).
    MoveTab(Step),
    /// The `n`th pane in reading order, from 0, becomes active (V.5).
    Focus(usize),
    /// The next or previous pane becomes active (V.5).
    Cycle(Step),
    /// Closes the active pane (V.6).
    Close,
    /// Every pane of a row or column the same size (V.4).
    Equalize,
}

/// One pane: its tabs, its canvas and what follows them.
#[derive(Clone, Copy)]
pub(crate) struct Pane {
    /// Its cell of the grid.
    pub(crate) cell: usize,
    pub(crate) canvas: ElementRef,
    /// The view around the canvas that holds the keyboard focus for it: the canvas is a window of its own.
    pub(crate) focus: ElementRef,
    pub(crate) pages: PageState,
    pub(crate) tabs: TabState,
    pub(crate) link_state: LinkState,
    pub(crate) thumbs: Thumbnails,
    pub(crate) editing: Editing,
    /// The outline (B.1): the pages report the places its new entries open.
    pub(crate) outline: OutlineState,
    /// The personal bookmarks (B.6): the pages report the places new ones open.
    pub(crate) bookmarks: BookmarkState,
}

impl Pane {
    /// The pane of `cell`, starting with `tabs`.
    fn new(
        cell: usize,
        tabs: Tabs,
        live: Live,
        recent: RecentState,
        revision: Signal<u64>,
    ) -> Self {
        let pages = PageState::new();
        let tabs = TabState::new(tabs, pages, live);
        crate::recent::keep(recent, tabs);
        let editing = Editing::new(tabs, revision);
        Self {
            cell,
            canvas: element_ref(),
            focus: element_ref(),
            pages,
            tabs,
            link_state: LinkState::new(tabs),
            thumbs: Thumbnails::new(tabs),
            editing,
            outline: OutlineState::new(tabs, pages, editing),
            bookmarks: BookmarkState::new(tabs, pages),
        }
    }

    /// Shows a launch's files here, turning the page when the tab shown stays (R.11).
    fn launch(self, launch: &Launch) {
        if self.tabs.launch(launch)
            && let Some(page) = launch.page
        {
            self.go_to(page);
        }
    }

    /// Sends the canvas to the page `index`.
    pub(crate) fn go_to(self, index: usize) {
        self.canvas
            .command("goToPage", serde_json::json!({ "index": index }));
    }

    /// Whether one of its dialogs or menus is open: the canvas, a window over the app's, hides under it.
    pub(crate) fn covers_pages(self) -> bool {
        self.tabs.editing().is_some()
            || self.link_state.is_asking()
            || self.outline.is_menu_open()
            || self.bookmarks.covers_pages()
    }
}

/// The window's panes.
#[derive(Clone, Copy)]
pub(crate) struct Panes {
    layout: Signal<Layout>,
    /// The active pane's cell; changes when another becomes active, not when the layout's sizes do.
    active: Memo<usize>,
    slots: [Pane; CELLS],
    live: Live,
}

impl Panes {
    /// The last session's panes, with the files the app was started with.
    pub(crate) fn new(live: Live, recent: RecentState, restored: Restored) -> Self {
        let Restored {
            mut layout,
            mut tabs,
        } = restored;
        if let Some(launch) = crate::open::startup() {
            route(&mut layout, &mut tabs, launch);
        }
        let revision = signal(0);
        let mut tabs = tabs.into_iter();
        let slots: [Pane; CELLS] = std::array::from_fn(|cell| {
            Pane::new(
                cell,
                tabs.next().unwrap_or_default(),
                live,
                recent,
                revision,
            )
        });
        let layout = signal(layout);
        let panes = Self {
            layout,
            active: computed(move || layout.with(Layout::active)),
            slots,
            live,
        };
        for pane in slots {
            panes.close_when_empty(pane);
        }
        panes
    }

    /// Shows another workspace's panes and tabs in place of these (W.1), each pane's shown tab where it was left.
    pub(crate) fn show(self, restored: Restored) {
        let Restored { layout, tabs } = restored;
        batch(|| {
            self.layout.set(layout);
            for (pane, tabs) in self.slots.into_iter().zip(tabs) {
                pane.tabs.replace(tabs, pane.canvas);
            }
        });
        focus_later(self.active());
    }

    /// Takes every pane's tabs out, the active pane's last and its shown tab at the very end (W.2: a new workspace
    /// from all of them). The panes then close, as panes with no tab do.
    pub(crate) fn take_every_tab(self) -> Vec<Closed> {
        let active = self.active().cell;
        let mut order: Vec<Pane> = self
            .slots
            .into_iter()
            .filter(|pane| pane.cell != active)
            .collect();
        order.push(self.slots[active]);
        order
            .into_iter()
            .flat_map(|pane| pane.tabs.take_all())
            .collect()
    }

    /// Every pane, open or not, by cell.
    pub(crate) fn slots(self) -> [Pane; CELLS] {
        self.slots
    }

    /// The active pane, as it is now (untracked: for commands and for what is built once).
    pub(crate) fn active(self) -> Pane {
        self.slots[untracked(|| self.active.get())]
    }

    /// The active pane's cell (tracked).
    pub(crate) fn active_cell(self) -> usize {
        self.active.get()
    }

    /// The layout (tracked).
    pub(crate) fn layout(self) -> Layout {
        self.layout.get()
    }

    /// Every pane's tabs as they are now, by cell (tracked): what the session saves.
    pub(crate) fn current(self) -> [Tabs; CELLS] {
        self.slots.map(|pane| pane.tabs.current())
    }

    /// Whether the window is split (tracked).
    pub(crate) fn is_split(self) -> bool {
        self.layout.with(Layout::is_split)
    }

    /// Whether `cell` holds a pane (tracked).
    fn is_open(self, cell: usize) -> bool {
        self.layout.with(|layout| layout.is_open(cell))
    }

    /// Whether row `row` holds a pane (tracked).
    fn row_open(self, row: usize) -> bool {
        self.layout.with(|layout| layout.row_open(row))
    }

    /// The height row `row` wants (tracked).
    fn row_size(self, row: usize) -> Option<Points> {
        self.layout.with(|layout| layout.row_size(row)).map(px)
    }

    /// The width the pane in `cell` wants (tracked).
    fn column_size(self, cell: usize) -> Option<Points> {
        self.layout.with(|layout| layout.column_size(cell)).map(px)
    }

    /// Keeps the rows' heights a divider drag left.
    fn resize_rows(self, sizes: &[Points]) {
        let sizes: Vec<f32> = sizes.iter().map(|size| size.0).collect();
        let mut layout = self.layout.get_untracked();
        if layout.resize_rows(&sizes) {
            self.layout.set(layout);
        }
    }

    /// Keeps the widths of row `row`'s panes a divider drag left.
    fn resize_columns(self, row: usize, sizes: &[Points]) {
        let sizes: Vec<f32> = sizes.iter().map(|size| size.0).collect();
        let mut layout = self.layout.get_untracked();
        if layout.resize_columns(row, &sizes) {
            self.layout.set(layout);
        }
    }

    /// Makes the pane in `cell` the active one, when it isn't.
    pub(crate) fn focus(self, cell: usize) {
        let changes = self
            .layout
            .with_untracked(|layout| layout.active() != cell && layout.is_open(cell));
        if changes {
            self.layout.update(|layout| {
                layout.focus(cell);
            });
        }
    }

    /// Makes the pane in `cell` active and gives its pages the keyboard focus.
    fn focus_pane(self, cell: usize) {
        self.focus(cell);
        focus_later(self.slots[cell]);
    }

    /// Runs a pane command.
    pub(crate) fn run(self, action: PaneAction) {
        match action {
            PaneAction::Split(side) => self.split_shown(side),
            PaneAction::OpenInSplit => {
                tabs::pick_documents(move |paths| self.open_in_split(Side::Right, paths));
            }
            PaneAction::MoveTab(step) => self.move_tab(step),
            PaneAction::Focus(position) => {
                if let Some(cell) = self.layout.with_untracked(|layout| layout.at(position)) {
                    self.focus_pane(cell);
                }
            }
            PaneAction::Cycle(step) => {
                let cell = self.layout.with_untracked(|layout| layout.step(step));
                self.focus_pane(cell);
            }
            PaneAction::Close => self.close_active(),
            PaneAction::Equalize => self.layout.update(Layout::equalize),
        }
    }

    /// Shows a later launch's files (R.11): in the active pane, and a split's other files in new panes.
    pub(crate) fn launch(self, launch: &Launch) {
        for (n, files) in split_groups(launch).into_iter().enumerate() {
            let one = Launch {
                files,
                page: launch.page,
                split: None,
            };
            if n == 0 || !self.split(Side::Right, |pane| pane.launch(&one)) {
                self.active().launch(&one);
            }
        }
    }

    /// Opens `paths` in a new pane on `side` of the active one (V.2), or in the active pane when there are four.
    pub(crate) fn open_in_split(self, side: Side, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let launch = Launch {
            files: paths,
            page: None,
            split: None,
        };
        if !self.split(side, |pane| pane.launch(&launch)) {
            self.active().launch(&launch);
        }
    }

    /// The shown tab again in a new pane on `side`, at the same place (V.1).
    fn split_shown(self, side: Side) {
        if let Some(copy) = self.active().tabs.shown_copy() {
            self.split(side, |pane| pane.tabs.adopt(vec![copy]));
        }
    }

    /// Moves the shown tab to the next or previous pane in reading order; with one pane, to a new pane on that side,
    /// unless it is the pane's only tab (V.3).
    fn move_tab(self, step: Step) {
        let from = self.active();
        if untracked(|| self.is_split()) {
            let to = self.slots[self.layout.with_untracked(|layout| layout.step(step))];
            batch(|| {
                if let Some(tab) = from.tabs.take_shown() {
                    to.tabs.adopt(vec![tab]);
                    self.focus(to.cell);
                }
            });
            focus_later(to);
            return;
        }
        if untracked(|| from.tabs.count()) < 2 {
            return;
        }
        let side = match step {
            Step::Next => Side::Right,
            Step::Previous => Side::Left,
        };
        batch(|| {
            if let Some(tab) = from.tabs.take_shown() {
                self.split(side, |pane| pane.tabs.adopt(vec![tab]));
            }
        });
    }

    /// Opens a new pane on `side` of the active one, makes it active and lets `fill` give it its tabs, all at once:
    /// a pane is never seen empty. `false`, changing nothing, when there are four panes.
    fn split(self, side: Side, fill: impl FnOnce(Pane)) -> bool {
        let Some(split) = self.layout.with_untracked(|layout| layout.plan_split(side)) else {
            return false;
        };
        let pane = self.slots[split.cell];
        batch(|| {
            // The active pane moves over to leave its cell to the new one.
            if let Some((from, to)) = split.moved {
                let taken = self.slots[from].tabs.take_all();
                self.slots[to].tabs.adopt(taken);
            }
            fill(pane);
            self.layout.update(|layout| layout.apply(split));
        });
        focus_later(pane);
        true
    }

    /// Closes the active pane, its tabs going to the pane beside it or closing as the setting says (V.6). The last
    /// pane stays. Edits are saved as the pane's tabs go (K.15), so closing loses nothing.
    fn close_active(self) {
        let pane = self.active();
        if !untracked(|| self.is_split()) {
            return;
        }
        let closing = self
            .live
            .read(|s| ClosingTabs::parse(s.text(CLOSE_PANE_TABS)));
        let mut neighbour = None;
        batch(|| {
            self.layout
                .update(|layout| neighbour = layout.close(pane.cell));
            let Some(to) = neighbour else {
                return;
            };
            let taken = pane.tabs.take_all();
            match closing {
                ClosingTabs::Move => self.slots[to].tabs.adopt(taken),
                ClosingTabs::Close => self.slots[to].tabs.remember_closed(taken),
            }
        });
        if let Some(to) = neighbour {
            focus_later(self.slots[to]);
        }
    }

    /// Closes `pane` once its last tab has gone, while another pane is open.
    fn close_when_empty(self, pane: Pane) {
        effect(move || {
            let empty = !pane.tabs.has_tabs();
            let closes = empty
                && self
                    .layout
                    .with(|layout| layout.is_open(pane.cell) && layout.is_split());
            if !closes {
                return;
            }
            let mut next = None;
            untracked(|| {
                self.layout.update(|layout| {
                    let was_active = layout.active() == pane.cell;
                    let neighbour = layout.close(pane.cell);
                    next = neighbour.filter(|_| was_active);
                });
            });
            if let Some(next) = next {
                focus_later(self.slots[next]);
            }
        });
    }
}

/// Gives `pane`'s pages the keyboard focus once what made it active has been laid out (a new pane's view isn't
/// shown before).
fn focus_later(pane: Pane) {
    // The timer runs whether or not its handle is kept.
    drop(set_timeout(Duration::ZERO, move || pane.focus.focus()));
}

/// A file drag over the window (V.2): while it lasts, the edges of the page area take drops for new panes. Every
/// drop target follows it, since the drag goes from one to the next.
#[derive(Clone, Copy)]
pub(crate) struct FileDrag {
    on: Signal<bool>,
    /// Counts the drag's enters, so a leave followed by an enter elsewhere doesn't end it.
    entered: Stored<u64>,
}

impl FileDrag {
    pub(crate) fn new() -> Self {
        Self {
            on: signal(false),
            entered: stored(0),
        }
    }

    /// Whether files are being dragged over the window (tracked).
    pub(crate) fn is_on(self) -> bool {
        self.on.get()
    }

    /// The drag entered a drop target.
    pub(crate) fn enter(self) {
        self.entered.with(|entered| *entered += 1);
        if !self.on.get_untracked() {
            self.on.set(true);
        }
    }

    /// The drag left a drop target: over when it enters no other soon.
    pub(crate) fn leave(self) {
        let entered = self.entered.get();
        drop(set_timeout(DRAG_GONE, move || {
            if self.entered.get() == entered {
                self.on.set(false);
            }
        }));
    }

    /// The files were dropped.
    pub(crate) fn drop_done(self) {
        self.entered.with(|entered| *entered += 1);
        self.on.set(false);
    }
}

/// What the panes share with the window: its reading settings and full screen, the recent list and the file drag.
#[derive(Clone, Copy)]
pub(crate) struct Shared {
    pub(crate) reading: Reading,
    pub(crate) full_screen: FullScreen,
    pub(crate) recent: RecentState,
    pub(crate) file_drag: FileDrag,
}

/// The files of a launch, one pane's worth each: all of them without `--split`, one each with it, and
/// `--split-same`'s first twice.
fn split_groups(launch: &Launch) -> Vec<Vec<PathBuf>> {
    match launch.split {
        None => vec![launch.files.clone()],
        Some(LaunchSplit::Files) => launch.files.iter().map(|file| vec![file.clone()]).collect(),
        Some(LaunchSplit::Same) => launch
            .files
            .first()
            .map(|file| vec![vec![file.clone()]; 2])
            .unwrap_or_default(),
    }
}

/// Puts the launch the app started with into the restored panes (V.13): its files in the active pane, a split's
/// others each in a new pane on the right of the last; past the fourth pane they open as its tabs.
fn route(layout: &mut Layout, tabs: &mut [Tabs; CELLS], launch: &Launch) {
    for (n, files) in split_groups(launch).into_iter().enumerate() {
        if n > 0
            && let Some(split) = layout.plan_split(Side::Right)
        {
            if let Some((from, to)) = split.moved {
                tabs.swap(from, to);
            }
            layout.apply(split);
        }
        tabs[layout.active()].launch(&files, launch.page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("C:/papers/{name}.pdf"))
    }

    fn launch(names: &[&str], split: Option<LaunchSplit>) -> Launch {
        Launch {
            files: names.iter().map(|name| path(name)).collect(),
            page: None,
            split,
        }
    }

    fn shown(tabs: &Tabs) -> Option<PathBuf> {
        tabs.active().map(|tab| tab.path.clone())
    }

    #[test]
    fn a_plain_launch_opens_its_files_in_the_active_pane() {
        let mut layout = Layout::default();
        let mut tabs: [Tabs; CELLS] = Default::default();
        route(&mut layout, &mut tabs, &launch(&["a", "b"], None));
        assert!(!layout.is_split());
        assert_eq!(tabs[0].all().len(), 2);
        assert_eq!(shown(&tabs[0]), Some(path("b")));
    }

    #[test]
    fn split_puts_each_file_in_a_pane_of_its_own() {
        let mut layout = Layout::default();
        let mut tabs: [Tabs; CELLS] = Default::default();
        let files = ["a", "b", "c", "d", "e"];
        route(
            &mut layout,
            &mut tabs,
            &launch(&files, Some(LaunchSplit::Files)),
        );
        assert_eq!(layout.count(), 4);
        let shown: Vec<_> = tabs.iter().map(shown).collect();
        assert_eq!(
            shown,
            [
                Some(path("a")),
                Some(path("b")),
                Some(path("c")),
                Some(path("e"))
            ]
        );
        assert_eq!(
            tabs[3].all().len(),
            2,
            "past the fourth pane, tabs of the last"
        );
    }

    #[test]
    fn split_same_shows_the_file_in_two_panes() {
        let mut layout = Layout::default();
        let mut tabs: [Tabs; CELLS] = Default::default();
        route(
            &mut layout,
            &mut tabs,
            &launch(&["a"], Some(LaunchSplit::Same)),
        );
        assert_eq!(layout.panes().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(shown(&tabs[0]), shown(&tabs[1]));
        assert_eq!(layout.active(), 1);
    }

    #[test]
    fn closing_tabs_reads_its_setting() {
        assert_eq!(ClosingTabs::parse(None), ClosingTabs::Move);
        assert_eq!(ClosingTabs::parse(Some("close")), ClosingTabs::Close);
        assert_eq!(ClosingTabs::parse(Some("sideways")), ClosingTabs::Move);
    }
}
