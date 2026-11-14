//! Tabs (plan R.12): the files open in the window, one shown at a time on the one page canvas. The model
//! ([`model::Tabs`]) is plain data; [`TabState`] holds it for the window and does what the tab commands ask, the
//! strip ([`strip`]) shows it in the title bar and the tab list ([`list`]) finds any tab by name.
//!
//! Only the shown tab has a document open (W.14's rule): switching gives the canvas the other file, and the page the
//! reader left that tab at comes back once the canvas reports its pages.

mod list;
mod model;
mod strip;

use std::path::PathBuf;

use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use crate::goto::PageState;
use crate::settings::Live;

pub(crate) use list::tab_list;
pub(crate) use model::{Cycle, Step, TabId};
pub(crate) use strip::tab_strip;

use model::Tabs;

/// `settings.toml`'s key for the order Ctrl+Tab goes through the tabs in: `"recent"` (the default) or `"strip"`.
pub(crate) const TAB_CYCLE_ORDER: &str = "tab_cycle_order";

/// What the open dialog offers: the formats MuPDF opens (plan D3).
const DOCUMENT_EXTENSIONS: [&str; 9] = [
    "pdf", "xps", "oxps", "epub", "fb2", "mobi", "cbz", "svg", "djvu",
];

/// The window's tabs, and the page each switch brings back.
#[derive(Clone, Copy)]
pub(crate) struct TabState {
    tabs: Signal<Tabs>,
    /// The page to go to once the canvas has opened the shown tab's file.
    restore: Stored<Option<usize>>,
    pages: PageState,
    live: Live,
}

impl std::fmt::Debug for TabState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabState").finish_non_exhaustive()
    }
}

impl TabState {
    /// The window's tabs: the file the app was started with, if any.
    pub(crate) fn new(pages: PageState, live: Live) -> Self {
        let mut tabs = Tabs::default();
        if let Some(path) = crate::startup_file() {
            tabs.open(path.to_path_buf(), 0);
        }
        Self {
            tabs: signal(tabs),
            restore: stored(None),
            pages,
            live,
        }
    }

    /// The shown tab's file.
    pub(crate) fn shown_path(self) -> Option<PathBuf> {
        self.tabs
            .with(|tabs| tabs.active().map(|tab| tab.path.clone()))
    }

    /// Whether a tab is shown.
    pub(crate) fn has_shown(self) -> bool {
        self.tabs.with(|tabs| tabs.active().is_some())
    }

    /// Whether any tab is open, or one can be reopened.
    fn has_any(self) -> bool {
        self.tabs
            .with(|tabs| !tabs.all().is_empty() || tabs.closed().next().is_some())
    }

    /// Shows the open dialog and opens the files picked, each in a new tab.
    pub(crate) fn open_dialog(self) {
        let dialog = OpenDialog::files()
            .multiple(true)
            .title(t!("open.title"))
            .filter(FileFilter::new(t!("open.filter"), DOCUMENT_EXTENSIONS));
        let answer = pick_paths(dialog);
        spawn(async move {
            match answer.await {
                Ok(Some(paths)) => self.open_paths(paths),
                Ok(None) => {}
                Err(err) => log::warn!("open: the dialog failed: {err}"),
            }
        });
    }

    /// Opens `paths`, each in a new tab, and shows the last.
    pub(crate) fn open_paths(self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.leave_shown();
        self.tabs.update(|tabs| {
            for path in paths {
                tabs.open(path, 0);
            }
        });
        self.arrive();
    }

    /// Shows the tab `id`.
    pub(crate) fn show(self, id: TabId) {
        if self.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) == Some(id) {
            return;
        }
        self.leave_shown();
        self.tabs.update(|tabs| tabs.activate(id));
        self.arrive();
    }

    /// Closes the tab `id`.
    pub(crate) fn close(self, id: TabId) {
        self.change(|tabs| tabs.close(id));
    }

    /// Runs one of the tab commands on the shown tab.
    pub(crate) fn run(self, action: Action) {
        let Some(id) = self.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) else {
            if action == Action::ReopenClosed {
                self.change(|tabs| {
                    tabs.reopen_closed();
                });
            }
            return;
        };
        match action {
            Action::Close => self.close(id),
            Action::CloseOthers => self.change(|tabs| tabs.close_others(id)),
            Action::CloseToTheRight => self.change(|tabs| tabs.close_to_the_right(id)),
            Action::CloseDuplicates => self.change(|tabs| tabs.close_duplicates_of(id)),
            Action::ReopenClosed => self.change(|tabs| {
                tabs.reopen_closed();
            }),
            Action::Cycle(step) => {
                let cycle = self.live.read(|s| Cycle::parse(s.text(TAB_CYCLE_ORDER)));
                if let Some(to) = self.tabs.with(|tabs| tabs.cycle(cycle, step)) {
                    self.show(to);
                }
            }
            Action::Move(step) => self.tabs.update(|tabs| tabs.move_by(id, step)),
            Action::TogglePin => self.tabs.update(|tabs| {
                let pinned = tabs.get(id).is_some_and(|tab| tab.pinned);
                tabs.set_pinned(id, !pinned);
            }),
        }
    }

    /// Reopens the closed tab of `path`.
    fn reopen(self, path: &std::path::Path) {
        self.change(|tabs| {
            tabs.reopen(path);
        });
    }

    /// Takes the canvas's `pages` event: the shown tab's file is open, so its page comes back.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent, canvas: ElementRef) {
        if &*event.name != "pages" {
            return;
        }
        if let Some(page) = self.restore.with(Option::take)
            && page > 0
        {
            canvas.command("goToPage", serde_json::json!({ "index": page }));
        }
    }

    /// Changes the tabs in a way that may change the one shown.
    fn change(self, f: impl FnOnce(&mut Tabs)) {
        let before = self.tabs.with(|tabs| tabs.active().map(|tab| tab.id));
        self.leave_shown();
        self.tabs.update(f);
        if self.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) != before {
            self.arrive();
        }
    }

    /// Notes the page the shown tab is at, before it stops being shown (or closes, for reopening).
    fn leave_shown(self) {
        let page = untracked(|| self.pages.current());
        self.tabs.update(|tabs| {
            if let Some(id) = tabs.active().map(|tab| tab.id) {
                tabs.set_page(id, page);
            }
        });
    }

    /// A tab is shown now: its page is to come back.
    fn arrive(self) {
        let page = self.tabs.with(|tabs| tabs.active().map(|tab| tab.page));
        self.restore.with(|restore| *restore = page);
    }

    /// The canvas's `path` property: the shown tab's file, or none.
    pub(crate) fn path_prop(self) -> JsonValue {
        self.shown_path().map_or(JsonValue::Null, |path| {
            JsonValue::from(path.to_string_lossy().into_owned())
        })
    }
}

/// What a tab command does to the shown tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Close,
    CloseOthers,
    CloseToTheRight,
    CloseDuplicates,
    ReopenClosed,
    Cycle(Step),
    Move(Step),
    TogglePin,
}
