//! Workspaces (plan §11.16; W.1 to W.8, W.14): named sets of tabs, like Vivaldi's. Each window shows one, and a
//! workspace shows in one window at a time (W.8, [`claims`]): a switch to one another window shows focuses that
//! window. Switching saves the window's panes and tabs and puts the other's in their place ([`Panes::show`]). A workspace not shown is rows in
//! the library and nothing else: no document, no file handle (W.14). The switcher at the left of the title bar
//! ([`view::switcher_button`]) lists them; the palette and Ctrl+Alt+1 to 9 switch too.
//!
//! What a workspace holds is saved by [`crate::session`]; this module keeps the list (names, icons, colours, order)
//! and the shown one, and does what the workspace commands ask.

mod claims;
mod view;
mod windows;

use std::path::PathBuf;

use excellent_store::Store;
use excellent_store::workspace::WorkspaceId;
use excellent_store::workspaces::{self, Look, Workspace};
use nsh_core::prelude::*;

use crate::open::Launch;
use crate::panes::{ClosingTabs, Panes};
use crate::session::{self, Keeper, Restored};
use crate::settings::Live;
use crate::shell::App;
use crate::tabs::{Closed, GroupColor, Step};

pub(crate) use windows::{MAIN, WindowNo};

pub(crate) use view::{color_dot, switcher_button, workspace_dialogs};

/// `settings.toml`'s key for what opening a file open in another workspace does (W.6): `"switch"` (the default) or
/// `"here"`.
pub(crate) const OPEN_ELSEWHERE: &str = "open_elsewhere";

/// What opening a file that another workspace has open does (W.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Elsewhere {
    /// Shows that workspace, with the file.
    #[default]
    Switch,
    /// Opens it here as well; the two tabs share the document.
    Here,
}

impl Elsewhere {
    /// The setting's value, or the default.
    pub(crate) fn parse(id: Option<&str>) -> Self {
        match id {
            Some("here") => Self::Here,
            _ => Self::Switch,
        }
    }
}

/// How a new workspace starts (W.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Start {
    Empty,
    /// With every tab of the window, which leave the workspace shown.
    AllTabs,
    /// With the shown tab, which leaves.
    ShownTab,
}

/// Whether a tab sent to another workspace leaves this one (W.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transfer {
    Move,
    Copy,
}

/// What a workspace command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceAction {
    /// Opens the switcher's list (W.1).
    Switcher,
    /// A new workspace, shown at once with its edit dialog open to name it (W.2).
    New(Start),
    /// The shown workspace's edit dialog: name, icon, colour, order, delete (W.3).
    Edit,
    /// The shown workspace one place earlier or later in the list (W.3).
    Shift(Step),
    /// The shown tab to another workspace, picked from a list (W.4).
    SendTab(Transfer),
    /// The `n`th workspace, from 0 (W.5).
    Go(usize),
    /// The next or previous workspace, round the ends (W.5).
    Cycle(Step),
    /// Lists the workspaces, to open the one picked in a window of its own (W.8).
    OpenWindow,
}

/// Which of the workspace dialogs is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialog {
    /// The list, to switch.
    Switch,
    /// The list, to send the shown tab to the one picked.
    Send(Transfer),
    /// The shown workspace's edit dialog.
    Edit,
    /// "Delete this workspace?": move its tabs or close them.
    Delete,
    /// The list, to open the one picked in a new window (W.8).
    Window,
}

/// A window's workspaces: the list, which every window shares, and the one it shows.
#[derive(Clone, Copy)]
pub(crate) struct Workspaces {
    list: Signal<Vec<Workspace>>,
    shown: Signal<WorkspaceId>,
    panes: Panes,
    keeper: Keeper,
    live: Live,
    dialog: Signal<Option<Dialog>>,
    /// The window's number (W.8).
    window: WindowNo,
    /// What every window shares, to open another.
    app: App,
}

/// A number for a new window (W.8).
pub(crate) fn next_window() -> WindowNo {
    windows::next_number()
}

/// The list every window shares, as the library has it.
pub(crate) fn shared_list() -> Signal<Vec<Workspace>> {
    signal(read_list())
}

/// The workspaces the other windows showed when the app last ran (W.8), but `main`, the main window's.
pub(crate) fn reopened(main: WorkspaceId) -> Vec<WorkspaceId> {
    let mut ids = store()
        .and_then(|store| store.read(workspaces::windows).ok())
        .unwrap_or_default();
    ids.retain(|&id| id != main);
    ids.dedup();
    ids
}

/// The workspace the main window opens on (W.7) and what it holds.
pub(crate) fn opening() -> (WorkspaceId, Restored) {
    let shown = crate::library::store()
        .and_then(|store| store.read(workspaces::shown).ok().flatten())
        .unwrap_or(WorkspaceId::DEFAULT);
    (shown, session::load(shown).unwrap_or_default())
}

impl Workspaces {
    /// Window `window`'s workspaces, `shown` in `panes`; its tabs are saved from now on. Nothing else may show
    /// `shown` (W.8: [`open_in_window`](Self::open_in_window) asks first).
    pub(crate) fn new(app: App, panes: Panes, shown: WorkspaceId, window: WindowNo) -> Self {
        let id = shown;
        let shown = signal(shown);
        let this = Self {
            list: app.workspace_list,
            shown,
            panes,
            keeper: Keeper::new(panes, shown),
            live: app.live,
            dialog: signal(None),
            window,
            app,
        };
        windows::add(window, this);
        if let Err(holder) = windows::claim(window, id) {
            log::warn!("workspaces: window {holder} shows the workspace too");
        }
        this
    }

    /// The window `window` gives its ref to, so other windows can bring it to the front (W.8).
    pub(crate) fn set_window(self, window: nsh_core::element::desktop::WindowRef) {
        windows::set_window(self.window, window);
    }

    /// The window closes (W.8): every edit and its tabs are saved, its popped-out tabs come back first, and its
    /// workspace is free for another window.
    pub(crate) fn close(self) {
        self.panes.windows().dock_all(self.panes);
        for pane in self.panes.slots() {
            pane.editing.save_now();
        }
        self.keeper.save_now();
        windows::remove(self.window);
        remember_windows();
    }

    /// Whether another window shows workspace `id` (W.8).
    pub(crate) fn in_other_window(self, id: WorkspaceId) -> bool {
        windows::holder(id, self.window).is_some()
    }

    /// Shows workspace `id` in a window of its own (W.8); when a window shows it already, that window comes to the
    /// front.
    pub(crate) fn open_in_window(self, id: WorkspaceId) {
        if id == self.shown.get_untracked() {
            return;
        }
        match windows::holder(id, self.window) {
            Some(no) => windows::focus(no),
            None => crate::shell::open_workspace_window(self.app, id),
        }
    }

    /// Every workspace, in the reader's order.
    pub(crate) fn list(self) -> Vec<Workspace> {
        self.list.get()
    }

    /// The shown workspace's row (tracked).
    pub(crate) fn shown(self) -> Option<Workspace> {
        let id = self.shown.get();
        self.list
            .with(|list| list.iter().find(|w| w.id == id).cloned())
    }

    /// The shown workspace's id (tracked).
    pub(crate) fn shown_id(self) -> WorkspaceId {
        self.shown.get()
    }

    /// How many tabs the shown workspace has open now, in every pane (the list's count is the saved one).
    pub(crate) fn shown_tab_count(self) -> usize {
        self.panes
            .slots()
            .into_iter()
            .map(|pane| pane.tabs.count())
            .sum()
    }

    /// The open dialog, if any (tracked).
    pub(crate) fn dialog(self) -> Option<Dialog> {
        self.dialog.get()
    }

    /// Opens `dialog`, or closes the one open.
    pub(crate) fn set_dialog(self, dialog: Option<Dialog>) {
        self.dialog.set(dialog);
    }

    /// Runs a workspace command.
    pub(crate) fn run(self, action: WorkspaceAction) {
        match action {
            WorkspaceAction::Switcher => self.set_dialog(Some(Dialog::Switch)),
            WorkspaceAction::New(start) => self.create(start),
            WorkspaceAction::Edit => self.set_dialog(Some(Dialog::Edit)),
            WorkspaceAction::Shift(step) => self.shift(step),
            WorkspaceAction::SendTab(send) => {
                if self.list.with_untracked(Vec::len) > 1 {
                    self.set_dialog(Some(Dialog::Send(send)));
                }
            }
            WorkspaceAction::Go(n) => {
                if let Some(id) = self.list.with_untracked(|list| list.get(n).map(|w| w.id)) {
                    self.switch(id);
                }
            }
            WorkspaceAction::Cycle(step) => {
                let shown = self.shown.get_untracked();
                // W.8: past those other windows show.
                let free = windows::free_for(self.window, &self.ids());
                if let Some(id) = cycle(&free, shown, step) {
                    self.switch(id);
                }
            }
            WorkspaceAction::OpenWindow => {
                if self.list.with_untracked(Vec::len) > 1 {
                    self.set_dialog(Some(Dialog::Window));
                }
            }
        }
    }

    /// Shows workspace `id`: the shown one's panes are saved, and `id`'s take their place. When another window shows
    /// it, that window comes to the front instead (W.8).
    pub(crate) fn switch(self, id: WorkspaceId) {
        if id == self.shown.get_untracked() {
            return;
        }
        if let Err(holder) = windows::claim(self.window, id) {
            windows::focus(holder);
            return;
        }
        self.keeper.save_now();
        let restored = session::load(id).unwrap_or_default();
        batch(|| {
            self.shown.set(id);
            self.panes.show(restored);
        });
        // Waiting also lets the save above land first, so the list's counts are current.
        if self.window == MAIN {
            self.write(move |tx| workspaces::set_shown(tx, id));
        } else {
            remember_windows();
        }
        self.reload();
    }

    /// Opens `paths` (the open dialog, a drop on the window, a later launch): in the shown workspace, unless one is
    /// open in another and the setting says to switch there (W.6).
    pub(crate) fn open(self, launch: &Launch) {
        if self.elsewhere() == Elsewhere::Switch
            && launch.split.is_none()
            && let Some(id) = launch.files.iter().find_map(|path| self.holder(path))
        {
            // W.8: another window shows that workspace, so the files open there.
            if let Some(no) = windows::holder(id, self.window)
                && let Some(there) = windows::workspaces(no)
            {
                windows::focus(no);
                there.panes.launch(launch);
                return;
            }
            self.switch(id);
        }
        self.panes.launch(launch);
    }

    /// The open tabs of every workspace but the shown one (W.9), by workspace in the list's order, as last saved.
    /// shortcut: another window's own workspace is read as saved, up to a second behind; read its panes when that
    /// shows.
    pub(crate) fn tabs_elsewhere(self) -> Vec<(Workspace, Vec<PathBuf>)> {
        let shown = self.shown.get_untracked();
        let tabs = store()
            .and_then(|store| {
                store
                    .read(|conn| workspaces::tabs_elsewhere(conn, shown))
                    .inspect_err(|err| log::warn!("workspaces: can't read their tabs: {err}"))
                    .ok()
            })
            .unwrap_or_default();
        self.list.with_untracked(|list| {
            list.iter()
                .filter_map(|workspace| {
                    let paths: Vec<PathBuf> = tabs
                        .iter()
                        .filter(|(id, _)| *id == workspace.id)
                        .map(|(_, path)| path.clone())
                        .collect();
                    (!paths.is_empty()).then(|| (workspace.clone(), paths))
                })
                .collect()
        })
    }

    /// Shows workspace `id`'s tab of `path` (W.9): in the window that shows it, which comes to the front, or here
    /// after a switch. The pane that has the tab becomes the active one.
    pub(crate) fn show_tab(self, id: WorkspaceId, path: PathBuf) {
        let there = match windows::holder(id, self.window) {
            Some(no) => {
                windows::focus(no);
                windows::workspaces(no)
            }
            None => {
                self.switch(id);
                (self.shown.get_untracked() == id).then_some(self)
            }
        };
        let Some(there) = there else {
            return;
        };
        let panes = there.panes;
        let slots = panes.slots();
        match slots.iter().position(|pane| pane.tabs.has_file(&path)) {
            Some(cell) => {
                panes.focus(cell);
                slots[cell].tabs.open_at(path, 0);
            }
            // Closed since it was saved: it opens again.
            None => panes.active().tabs.open_at(path, 0),
        }
    }

    /// Opens files picked in the open dialog.
    pub(crate) fn open_paths(self, files: Vec<PathBuf>) {
        if !files.is_empty() {
            self.open(&Launch {
                files,
                page: None,
                split: None,
            });
        }
    }

    /// Gives the shown workspace a new look.
    pub(crate) fn restyle(self, look: Look) {
        let id = self.shown.get_untracked();
        let saved = look.clone();
        self.write(move |tx| workspaces::restyle(tx, id, &saved));
        self.list.update(|list| {
            if let Some(w) = list.iter_mut().find(|w| w.id == id) {
                w.name = look.name;
                w.icon = look.icon;
                w.color = look.color;
            }
        });
    }

    /// The workspace the shown one's tabs go to when it is deleted, and that is shown then: one no other window
    /// shows (W.8).
    pub(crate) fn neighbour(self) -> Option<Workspace> {
        let shown = self.shown.get();
        let ids: Vec<WorkspaceId> = self.list.with(|list| list.iter().map(|w| w.id).collect());
        let to = neighbour(&windows::free_for(self.window, &ids), shown)?;
        self.list
            .with(|list| list.iter().find(|w| w.id == to).cloned())
    }

    /// Deletes the shown workspace (W.3), moving its tabs to its neighbour or closing them. The last workspace stays.
    /// Edits are saved as they are made (K.15), so closing tabs here loses no work.
    pub(crate) fn delete_shown(self, tabs_go: ClosingTabs) {
        let Some(to) = self.neighbour().map(|w| w.id) else {
            return;
        };
        let gone = self.shown.get_untracked();
        let tabs = match tabs_go {
            ClosingTabs::Move => self.panes.take_every_tab(),
            ClosingTabs::Close => Vec::new(),
        };
        self.switch(to);
        self.panes.active().tabs.adopt(tabs);
        self.write(move |tx| workspaces::delete(tx, gone).map(drop));
        self.reload();
    }

    /// Sends the shown tab to workspace `to` (W.4); a moved tab leaves this one. It keeps its page and view.
    pub(crate) fn send_tab(self, send: Transfer, to: WorkspaceId) {
        let tabs = self.panes.active().tabs;
        let tab = match send {
            Transfer::Move => tabs.take_shown(),
            Transfer::Copy => tabs.shown_copy(),
        };
        if let Some(tab) = tab {
            // W.8: a workspace another window shows takes it there and then; its saved tabs would be written over.
            match windows::holder(to, self.window).and_then(windows::workspaces) {
                Some(there) => there.panes.active().tabs.adopt(vec![tab]),
                None => session::add_to(to, &tab),
            }
            self.reload();
        }
    }

    /// A new workspace (W.2), shown with `start`'s tabs, and its edit dialog open to name it.
    fn create(self, start: Start) {
        let count = self.list.with_untracked(Vec::len);
        let look = Look {
            name: t!("workspace.new_name", n = count + 1),
            icon: None,
            color: Some(
                GroupColor::ALL[count % GroupColor::ALL.len()]
                    .id()
                    .to_owned(),
            ),
        };
        let Some(id) = self.write(move |tx| workspaces::create(tx, &look)) else {
            return;
        };
        let tabs: Vec<Closed> = match start {
            Start::Empty => Vec::new(),
            Start::AllTabs => self.panes.take_every_tab(),
            Start::ShownTab => self.panes.active().tabs.take_shown().into_iter().collect(),
        };
        self.reload();
        self.switch(id);
        self.panes.active().tabs.adopt(tabs);
        self.set_dialog(Some(Dialog::Edit));
    }

    /// Moves the shown workspace one place in the list.
    fn shift(self, step: Step) {
        let shown = self.shown.get_untracked();
        let order = shifted(&self.ids(), shown, step);
        let ids = order.clone();
        self.write(move |tx| workspaces::reorder(tx, &ids));
        self.list
            .update(|list| list.sort_by_key(|w| order.iter().position(|&id| id == w.id)));
    }

    /// The workspaces' ids, in order.
    fn ids(self) -> Vec<WorkspaceId> {
        self.list
            .with_untracked(|list| list.iter().map(|w| w.id).collect())
    }

    /// The setting of W.6.
    fn elsewhere(self) -> Elsewhere {
        self.live.read(|s| Elsewhere::parse(s.text(OPEN_ELSEWHERE)))
    }

    /// Another workspace with a tab of `path`, unless the shown one has it too.
    fn holder(self, path: &std::path::Path) -> Option<WorkspaceId> {
        let here = self
            .panes
            .slots()
            .into_iter()
            .any(|pane| pane.tabs.has_file(path));
        if here {
            return None;
        }
        let shown = self.shown.get_untracked();
        store()?
            .read(|conn| workspaces::holding(conn, path, shown))
            .inspect_err(|err| log::warn!("workspaces: can't look for the file: {err}"))
            .ok()
            .flatten()
    }

    /// Reads the list again from the library.
    fn reload(self) {
        self.list.set(read_list());
    }

    /// Makes a change in the library and waits for it: they are small, and the list is read right after.
    fn write<T: Send + 'static>(
        self,
        f: impl FnOnce(&excellent_store::rusqlite::Transaction<'_>) -> excellent_store::Result<T>
        + Send
        + 'static,
    ) -> Option<T> {
        store()?
            .write(f)
            .wait()
            .inspect_err(|err| log::warn!("workspaces: can't save the change: {err}"))
            .ok()
    }
}

/// The library, if it opened.
fn store() -> Option<&'static Store> {
    crate::library::store()
}

/// Notes which workspaces the other windows show, for the next start (W.8).
fn remember_windows() {
    let shown = windows::others();
    if let Some(store) = store()
        && let Err(err) = store
            .write(move |tx| workspaces::set_windows(tx, &shown))
            .wait()
    {
        log::warn!("workspaces: can't note the windows: {err}");
    }
}

/// The list from the library; without one, the first workspace alone.
fn read_list() -> Vec<Workspace> {
    let list = store()
        .and_then(|store| {
            store
                .read(workspaces::list)
                .inspect_err(|err| log::warn!("workspaces: can't read them: {err}"))
                .ok()
        })
        .unwrap_or_default();
    if list.is_empty() {
        return vec![Workspace {
            id: WorkspaceId::DEFAULT,
            name: String::new(),
            icon: None,
            color: None,
            tabs: 0,
        }];
    }
    list
}

/// The workspace after or before `shown`, round the ends; none when it is the only one.
fn cycle<T: Copy + PartialEq>(ids: &[T], shown: T, step: Step) -> Option<T> {
    let at = ids.iter().position(|&id| id == shown)?;
    let n = ids.len();
    let next = match step {
        Step::Next => (at + 1) % n,
        Step::Previous => (at + n - 1) % n,
    };
    (next != at).then(|| ids[next])
}

/// The workspace before `shown`, or after it when it is the first.
fn neighbour<T: Copy + PartialEq>(ids: &[T], shown: T) -> Option<T> {
    let at = ids.iter().position(|&id| id == shown)?;
    at.checked_sub(1)
        .and_then(|before| ids.get(before))
        .or_else(|| ids.get(at + 1))
        .copied()
}

/// The order with `shown` one place later or earlier; at an end it stays.
fn shifted<T: Copy + PartialEq>(ids: &[T], shown: T, step: Step) -> Vec<T> {
    let mut ids = ids.to_vec();
    if let Some(at) = ids.iter().position(|&id| id == shown) {
        let to = match step {
            Step::Next => at + 1,
            Step::Previous => at.wrapping_sub(1),
        };
        if to < ids.len() {
            ids.swap(at, to);
        }
    }
    ids
}

/// The name a workspace shows: its own, or the first workspace's default.
pub(crate) fn display_name(workspace: &Workspace) -> String {
    if workspace.name.trim().is_empty() {
        t!("workspace.default_name")
    } else {
        workspace.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspaces_cycle_round_the_ends() {
        assert_eq!(cycle(&[1, 2, 3], 1, Step::Next), Some(2));
        assert_eq!(cycle(&[1, 2, 3], 3, Step::Next), Some(1));
        assert_eq!(cycle(&[1, 2, 3], 1, Step::Previous), Some(3));
        assert_eq!(
            cycle(&[1], 1, Step::Next),
            None,
            "one workspace goes nowhere"
        );
    }

    #[test]
    fn a_deleted_workspace_gives_way_to_the_one_before_it() {
        assert_eq!(neighbour(&[1, 2, 3], 2), Some(1));
        assert_eq!(neighbour(&[1, 2, 3], 1), Some(2));
        assert_eq!(neighbour(&[1], 1), None, "the last one stays");
    }

    #[test]
    fn a_workspace_moves_one_place_and_stops_at_the_ends() {
        assert_eq!(shifted(&[1, 2, 3], 1, Step::Next), [2, 1, 3]);
        assert_eq!(shifted(&[1, 2, 3], 1, Step::Previous), [1, 2, 3]);
        assert_eq!(shifted(&[1, 2, 3], 3, Step::Next), [1, 2, 3]);
    }

    #[test]
    fn opening_elsewhere_reads_its_setting() {
        assert_eq!(Elsewhere::parse(None), Elsewhere::Switch);
        assert_eq!(Elsewhere::parse(Some("here")), Elsewhere::Here);
        assert_eq!(Elsewhere::parse(Some("there")), Elsewhere::Switch);
    }
}
