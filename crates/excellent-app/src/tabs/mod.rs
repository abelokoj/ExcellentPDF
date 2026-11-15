//! Tabs (plan R.12): the files open in the window, one shown at a time on the one page canvas. The model
//! ([`model::Tabs`]) is plain data; [`TabState`] holds it for the window and does what the tab commands ask, the
//! strip ([`strip`]) shows it in the title bar and the tab list ([`list`]) finds any tab by name.
//!
//! Only the shown tab has a document open (W.14's rule): switching gives the canvas the other file, and the view the
//! reader left that tab in (its page and place, zoom, layout and rotation: R.1, R.2, R.9) comes back once the canvas
//! reports its pages. The tabs start as the last session left them
//! (R.9, [`crate::session`]); a tab whose file has gone shows greyed, and the document panel offers "Locate…".
//!
//! Tabs can be gathered into named, coloured groups (R.13, [`groups`]): a chip in the strip names each, collapses
//! and expands it, and opens its edit dialog ([`group_dialog`]).

mod group_dialog;
mod groups;
mod list;
mod model;
mod strip;

use std::path::PathBuf;

use excellent_core::formats::EXTENSIONS;
use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use crate::goto::PageState;
use crate::open::Launch;
use crate::settings::Live;

pub(crate) use group_dialog::{Tone, action_button, color_name, group_dialog};
pub(crate) use groups::{GroupColor, GroupId};
pub(crate) use list::tab_list;
pub(crate) use model::{Closed, Cycle, Step, TabId, Tabs};
pub(crate) use strip::{pin_mark, tab_strip};

/// `settings.toml`'s key for the order Ctrl+Tab goes through the tabs in: `"recent"` (the default) or `"strip"`.
pub(crate) const TAB_CYCLE_ORDER: &str = "tab_cycle_order";

/// The window's tabs, and the view each switch brings back.
#[derive(Clone, Copy)]
pub(crate) struct TabState {
    tabs: Signal<Tabs>,
    /// Where to go once the canvas has opened the shown tab's file.
    restore: Stored<Option<Restore>>,
    /// The shown tab's view, as the canvas last reported it since it went there.
    shown_view: Stored<Option<String>>,
    /// The group whose edit dialog is open.
    editing: Signal<Option<GroupId>>,
    pages: PageState,
    live: Live,
}

impl std::fmt::Debug for TabState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabState").finish_non_exhaustive()
    }
}

impl TabState {
    /// A pane's tabs (V.1), starting as `tabs`: the last session's, with the files the app was started with
    /// ([`crate::panes`] routes them). `pages` is the pane's canvas's.
    pub(crate) fn new(tabs: Tabs, pages: PageState, live: Live) -> Self {
        // The shown tab's view comes back once its file is open.
        let restore = tabs.active().map(Restore::of);
        Self {
            tabs: signal(tabs),
            restore: stored(restore),
            shown_view: stored(None),
            editing: signal(None),
            pages,
            live,
        }
    }

    /// The shown tab's file, while it is there.
    pub(crate) fn shown_path(self) -> Option<PathBuf> {
        self.tabs.with(|tabs| {
            tabs.active()
                .filter(|tab| !tab.missing)
                .map(|tab| tab.path.clone())
        })
    }

    /// The shown tab's file name, while its file is there.
    pub(crate) fn shown_name(self) -> Option<String> {
        self.shown_path().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
    }

    /// Whether a tab is shown whose file is there: the canvas has a document.
    pub(crate) fn has_shown(self) -> bool {
        self.tabs
            .with(|tabs| tabs.active().is_some_and(|tab| !tab.missing))
    }

    /// The shown tab, when its file has gone: the tab and where its file was.
    pub(crate) fn shown_missing(self) -> Option<(TabId, PathBuf)> {
        self.tabs.with(|tabs| {
            tabs.active()
                .filter(|tab| tab.missing)
                .map(|tab| (tab.id, tab.path.clone()))
        })
    }

    /// The shown tab's file, while it is there, and the page it shows: what the recent list notes (R.10).
    pub(crate) fn shown_place(self) -> Option<(PathBuf, usize)> {
        let path = self.shown_path()?;
        // Until the canvas has gone to the page a switch brings back, the page it reports is the last file's.
        let settled = self.restore.with(|restore| restore.is_none());
        settled.then(|| (path, self.pages.current()))
    }

    /// Whether any tab is open.
    pub(crate) fn has_tabs(self) -> bool {
        self.tabs.with(|tabs| !tabs.all().is_empty())
    }

    /// How many tabs are open.
    pub(crate) fn count(self) -> usize {
        self.tabs.with(|tabs| tabs.all().len())
    }

    /// The tabs as they are now, the shown one at the page and in the view it shows: what the session saves (R.9).
    /// It tracks the tabs and the page (a new view on the same page is saved with the next change, or on quitting).
    pub(crate) fn current(self) -> Tabs {
        let page = self.pages.current();
        let mut tabs = self.tabs.get();
        // Until the canvas has gone to the page a switch brings back, the page it reports is the last file's.
        let settled = self.restore.with(|restore| restore.is_none());
        if let Some(id) = tabs.active().filter(|tab| !tab.missing).map(|tab| tab.id)
            && settled
        {
            tabs.set_page(id, page);
            if let Some(view) = self.shown_view.with(|view| view.clone()) {
                tabs.set_view(id, view);
            }
        }
        tabs
    }

    /// The group whose edit dialog is open.
    pub(crate) fn editing(self) -> Option<GroupId> {
        self.editing.get()
    }

    /// Closes the group edit dialog.
    pub(crate) fn stop_editing(self) {
        self.editing.set(None);
    }

    /// Opens the edit dialog of the group `id`.
    fn edit(self, id: GroupId) {
        self.editing.set(Some(id));
    }

    /// The group `id`, while it has tabs.
    pub(crate) fn group(self, id: GroupId) -> Option<groups::Group> {
        self.tabs.with(|tabs| tabs.group(id).cloned())
    }

    /// Names the group `id`.
    pub(crate) fn rename_group(self, id: GroupId, name: &str) {
        self.tabs.update(|tabs| tabs.rename_group(id, name));
    }

    /// Colours the group `id`.
    pub(crate) fn recolor_group(self, id: GroupId, color: GroupColor) {
        self.tabs.update(|tabs| tabs.recolor_group(id, color));
    }

    /// Collapses or expands the group `id` (which may show another tab).
    pub(crate) fn toggle_group(self, id: GroupId) {
        self.change(|tabs| {
            tabs.toggle_group(id);
        });
    }

    /// Dissolves the group `id`, keeping its tabs.
    pub(crate) fn ungroup(self, id: GroupId) {
        self.tabs.update(|tabs| tabs.ungroup(id));
    }

    /// Closes the group `id` with its tabs.
    pub(crate) fn close_group(self, id: GroupId) {
        self.change(|tabs| tabs.close_group(id));
    }

    /// Whether any tab is open, or one can be reopened.
    fn has_any(self) -> bool {
        self.tabs
            .with(|tabs| !tabs.all().is_empty() || tabs.closed().next().is_some())
    }

    /// Shows the open dialog and opens the files picked, each in a new tab.
    pub(crate) fn open_dialog(self) {
        pick_documents(move |paths| self.open_paths(paths));
    }

    /// Asks where the missing file of the tab `id` went, and shows it from there.
    pub(crate) fn locate(self, id: TabId) {
        let name = self
            .tabs
            .with(|tabs| tabs.get(id).map(model::Tab::title))
            .unwrap_or_default();
        let dialog = OpenDialog::files()
            .title(t!("locate.title", name = name))
            .filter(FileFilter::new(
                t!("open.filter"),
                EXTENSIONS.iter().copied(),
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            match answer.await {
                Ok(Some(paths)) => {
                    if let Some(path) = paths.into_iter().next() {
                        self.tabs.update(|tabs| tabs.relocate(id, path));
                        self.arrive();
                    }
                }
                Ok(None) => {}
                Err(err) => log::warn!("locate: the dialog failed: {err}"),
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

    /// Shows a later launch's files (R.11). `true` when the tab shown stays and the launch gave a page: the canvas
    /// isn't reopening, so the caller turns the page.
    pub(crate) fn launch(self, launch: &Launch) -> bool {
        let before = self.tabs.with(|tabs| tabs.active().map(|tab| tab.id));
        self.leave_shown();
        self.tabs
            .update(|tabs| tabs.launch(&launch.files, launch.page));
        let after = self.tabs.with(|tabs| tabs.active().map(|tab| tab.id));
        self.arrive();
        if before == after && launch.page.is_some() {
            self.restore.with(Option::take);
            return true;
        }
        false
    }

    /// Shows `path` at `page` (from the recent list): its tab if it has one, else a new one.
    pub(crate) fn open_at(self, path: PathBuf, page: usize) {
        match self.tabs.with(|tabs| tabs.find(&path)) {
            Some(id) => self.show(id),
            None => {
                self.leave_shown();
                self.tabs.update(|tabs| {
                    tabs.open(path, page);
                });
                self.arrive();
            }
        }
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

    /// Takes the shown tab out, at its page and in its view, to move it to another pane (V.3).
    pub(crate) fn take_shown(self) -> Option<Closed> {
        let mut taken = None;
        self.change(|tabs| {
            if let Some(id) = tabs.active().map(|tab| tab.id) {
                taken = tabs.take(id);
            }
        });
        taken
    }

    /// Takes every tab out, the shown one last: a closing pane's (V.6).
    pub(crate) fn take_all(self) -> Vec<Closed> {
        let mut taken = Vec::new();
        self.change(|tabs| taken = tabs.take_all());
        taken
    }

    /// Adds tabs taken from another pane at the end of the strip, each at its page and in its view, and shows the
    /// last.
    pub(crate) fn adopt(self, taken: Vec<Closed>) {
        if taken.is_empty() {
            return;
        }
        self.change(|tabs| {
            for tab in taken {
                tabs.adopt(tab);
            }
        });
    }

    /// Keeps `closed` to reopen, as if closed here (a closing pane's tabs, V.6).
    pub(crate) fn remember_closed(self, closed: Vec<Closed>) {
        self.tabs.update(|tabs| {
            for tab in closed {
                tabs.remember_closed(tab);
            }
        });
    }

    /// The shown tab as it is now, at its page and in its view, to show it in a new pane too (V.1). None while its
    /// file has gone.
    pub(crate) fn shown_copy(self) -> Option<Closed> {
        let tabs = untracked(|| self.current());
        let tab = tabs.active().filter(|tab| !tab.missing)?;
        Some(Closed {
            path: tab.path.clone(),
            page: tab.page,
            view: tab.view.clone(),
            pinned: false,
            index: 0,
        })
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
            Action::NewGroup => self.new_group(id),
            Action::EditGroup => match self.group_of(id) {
                Some(group) => self.edit(group),
                None => self.new_group(id),
            },
            Action::LeaveGroup => self.tabs.update(|tabs| tabs.leave_group(id)),
            Action::ToggleGroup => {
                if let Some(group) = self.group_of(id) {
                    self.toggle_group(group);
                }
            }
        }
    }

    /// The group of the tab `id`.
    fn group_of(self, id: TabId) -> Option<GroupId> {
        self.tabs
            .with(|tabs| tabs.get(id).and_then(|tab| tab.group))
    }

    /// Puts the tab `id` in a new group and opens the group's edit dialog, to name it.
    fn new_group(self, id: TabId) {
        let mut group = None;
        self.tabs.update(|tabs| group = tabs.new_group(id));
        if let Some(group) = group {
            self.edit(group);
        }
    }

    /// Reopens the closed tab of `path`.
    fn reopen(self, path: &std::path::Path) {
        self.change(|tabs| {
            tabs.reopen(path);
        });
    }

    /// Takes the canvas's `pages` event (the shown tab's file is open, so its view comes back) and `view` events.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent, canvas: ElementRef) {
        match &*event.name {
            "pages" => {
                self.shown_view.with(Option::take);
                match self.restore.with(Option::take) {
                    Some(Restore {
                        view: Some(view), ..
                    }) => canvas.command("setView", view),
                    Some(Restore { page, view: None }) if page > 0 => {
                        canvas.command("goToPage", serde_json::json!({ "index": page }));
                    }
                    _ => {}
                }
            }
            // Until the shown tab's view has come back, the views reported are the last file's.
            "view" if self.restore.with(|restore| restore.is_none()) => {
                let view = event.data.to_string();
                self.shown_view.with(|shown| *shown = Some(view));
            }
            _ => {}
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

    /// Notes the page and the view the shown tab is at, before it stops being shown (or closes, for reopening).
    fn leave_shown(self) {
        let view = self.shown_view.with(Option::take);
        // Until the canvas has gone to the page its tab was at, the page it reports is the last file's.
        if self.restore.with(|restore| restore.is_some()) {
            return;
        }
        let page = untracked(|| self.pages.current());
        self.tabs.update(|tabs| {
            if let Some(id) = tabs.active().map(|tab| tab.id) {
                tabs.set_page(id, page);
                if let Some(view) = view {
                    tabs.set_view(id, view);
                }
            }
        });
    }

    /// A tab is shown now: its page is to come back, and if its file has gone it says so.
    fn arrive(self) {
        self.tabs.update(|tabs| {
            if let Some((id, missing)) = tabs.active().map(|tab| (tab.id, !tab.path.is_file())) {
                tabs.set_missing(id, missing);
            }
        });
        let restore = self.tabs.with(|tabs| tabs.active().map(Restore::of));
        self.restore.with(|slot| *slot = restore);
        self.shown_view.with(Option::take);
    }

    /// The canvas's `path` property: the shown tab's file, or none.
    pub(crate) fn path_prop(self) -> JsonValue {
        self.shown_path().map_or(JsonValue::Null, |path| {
            JsonValue::from(path.to_string_lossy().into_owned())
        })
    }
}

/// Shows the open dialog, and hands the files picked to `then`.
pub(crate) fn pick_documents(then: impl FnOnce(Vec<PathBuf>) + 'static) {
    let dialog = OpenDialog::files()
        .multiple(true)
        .title(t!("open.title"))
        .filter(FileFilter::new(
            t!("open.filter"),
            EXTENSIONS.iter().copied(),
        ));
    let answer = pick_paths(dialog);
    spawn(async move {
        match answer.await {
            Ok(Some(paths)) => then(paths),
            Ok(None) => {}
            Err(err) => log::warn!("open: the dialog failed: {err}"),
        }
    });
}

/// Where a tab shown again goes once its file is open: its view, or its page when it has no view there.
#[derive(Debug, Clone, PartialEq)]
struct Restore {
    page: usize,
    view: Option<JsonValue>,
}

impl Restore {
    /// Where `tab` was left.
    fn of(tab: &model::Tab) -> Self {
        let view = tab
            .view
            .as_deref()
            .filter(|view| crate::session::page_of(view) == tab.page)
            .and_then(|view| serde_json::from_str(view).ok());
        Self {
            page: tab.page,
            view,
        }
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
    /// Puts the tab in a new group of its own and opens the group's edit dialog (R.13).
    NewGroup,
    /// Opens the edit dialog of the tab's group, making one if it has none.
    EditGroup,
    /// Takes the tab out of its group.
    LeaveGroup,
    /// Collapses or expands the tab's group.
    ToggleGroup,
}
