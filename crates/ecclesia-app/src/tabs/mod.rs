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
mod properties;
mod strip;

use std::path::{Path, PathBuf};

use ecclesia_core::formats::EXTENSIONS;
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
pub(crate) use properties::properties_dialog;
pub(crate) use strip::{home_icon_src, pin_mark, tab_strip};

/// `settings.toml`'s key for the order Ctrl+Tab goes through the tabs in: `"recent"` (the default) or `"strip"`.
pub(crate) const TAB_CYCLE_ORDER: &str = "tab_cycle_order";

/// `settings.toml`'s key for the tab bar style: `"vivaldi"` (the default) or `"chrome"`.
pub(crate) const TAB_STYLE: &str = "tab_style";

/// `settings.toml`'s key for the minimum unpinned tab width in logical pixels (default `250`).
pub(crate) const TAB_MIN_WIDTH_PX: &str = "tab_min_width_px";

/// `settings.toml`'s key for the tab height in logical pixels (default `34`).
pub(crate) const TAB_HEIGHT_PX: &str = "tab_height_px";

/// `settings.toml`'s key for the tab name and workspace label font size in logical pixels (default `12`).
pub(crate) const TAB_FONT_SIZE_PX: &str = "tab_font_size_px";

/// `settings.toml`'s key for the tab group snapshot saved from the tab context menu.
pub(crate) const SAVED_TAB_GROUP: &str = "saved_tab_group";

/// Legacy key retained as a fallback when migrating older configs saved in centimeters.
const LEGACY_TAB_MIN_WIDTH_CM: &str = "tab_min_width_cm";

/// Default minimum width of unpinned tabs in logical pixels.
pub(crate) const DEFAULT_TAB_MIN_WIDTH_PX: f64 = 250.0;

/// Lower bound for minimum tab width setting in logical pixels.
pub(crate) const MIN_TAB_MIN_WIDTH_PX: f64 = 60.0;

/// Upper bound for minimum tab width setting in logical pixels.
pub(crate) const MAX_TAB_MIN_WIDTH_PX: f64 = 500.0;

/// Default height of tabs in logical pixels (20% taller than the original `28 px`).
pub(crate) const DEFAULT_TAB_HEIGHT_PX: f64 = 34.0;

/// Lower bound for tab height setting in logical pixels.
pub(crate) const MIN_TAB_HEIGHT_PX: f64 = 24.0;

/// Upper bound for tab height setting in logical pixels.
pub(crate) const MAX_TAB_HEIGHT_PX: f64 = 56.0;

/// Default font size of tab and workspace labels in logical pixels.
pub(crate) const DEFAULT_TAB_FONT_SIZE_PX: f64 = 12.0;

/// Lower bound for tab font size setting in logical pixels.
pub(crate) const MIN_TAB_FONT_SIZE_PX: f64 = 10.0;

/// Upper bound for tab font size setting in logical pixels.
pub(crate) const MAX_TAB_FONT_SIZE_PX: f64 = 20.0;

/// Logical 96-DPI points per centimeter (`96 / 2.54`).
const POINTS_PER_CM: f64 = 96.0 / 2.54;

/// Visual presentation style of tabs in the title bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TabStyle {
    /// Floating rounded-pill tabs with an active mode indicator dot (default).
    #[default]
    Vivaldi,
    /// Chrome-style top-accented attached tabs.
    Chrome,
}

impl TabStyle {
    /// Parses a persisted setting string into a [`TabStyle`], defaulting to [`TabStyle::Vivaldi`].
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("chrome") => Self::Chrome,
            _ => Self::Vivaldi,
        }
    }
}

/// Returns the clamped minimum unpinned tab width in logical pixels from `settings`.
///
/// Defaults to [`DEFAULT_TAB_MIN_WIDTH_PX`] (`250 px`), falling back to the legacy
/// `tab_min_width_cm` key if present.
pub(crate) fn min_width_px(settings: &crate::settings::Settings) -> f32 {
    let raw = settings.number(TAB_MIN_WIDTH_PX).or_else(|| {
        settings
            .number(LEGACY_TAB_MIN_WIDTH_CM)
            .map(|cm| cm * POINTS_PER_CM)
    });
    raw.map_or(DEFAULT_TAB_MIN_WIDTH_PX, |px_val| {
        px_val.clamp(MIN_TAB_MIN_WIDTH_PX, MAX_TAB_MIN_WIDTH_PX)
    }) as f32
}

/// Returns the clamped tab height in logical pixels from `settings`.
///
/// Defaults to [`DEFAULT_TAB_HEIGHT_PX`] (`34 px`) and clamps to
/// `[MIN_TAB_HEIGHT_PX, MAX_TAB_HEIGHT_PX]` (`24..=56 px`).
pub(crate) fn tab_height_px(settings: &crate::settings::Settings) -> f32 {
    settings
        .number(TAB_HEIGHT_PX)
        .map_or(DEFAULT_TAB_HEIGHT_PX, |px_val| {
            px_val.clamp(MIN_TAB_HEIGHT_PX, MAX_TAB_HEIGHT_PX)
        }) as f32
}

/// Returns the clamped tab and workspace label font size in logical pixels from `settings`.
///
/// Defaults to [`DEFAULT_TAB_FONT_SIZE_PX`] (`12 px`) and clamps to
/// `[MIN_TAB_FONT_SIZE_PX, MAX_TAB_FONT_SIZE_PX]` (`10..=20 px`).
pub(crate) fn tab_font_size_px(settings: &crate::settings::Settings) -> f32 {
    settings
        .number(TAB_FONT_SIZE_PX)
        .map_or(DEFAULT_TAB_FONT_SIZE_PX, |px_val| {
            px_val.clamp(MIN_TAB_FONT_SIZE_PX, MAX_TAB_FONT_SIZE_PX)
        }) as f32
}

/// Returns the title bar height in logical pixels derived from the configured tab height.
///
/// Always keeps at least `40 px` (Windows caption button height) and gives `12 px` of vertical
/// breathing room over the tab height so taller user-chosen tabs never clip.
pub(crate) fn title_bar_height_px(settings: &crate::settings::Settings) -> f32 {
    (tab_height_px(settings) + 12.0).max(40.0)
}

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
    /// Whether any tab's right-click context menu is currently open.
    menu_open: Signal<bool>,
    /// The tab whose document Properties dialog is currently open.
    properties_for: Signal<Option<TabId>>,
    pages: PageState,
    pub(crate) live: Live,
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
            menu_open: signal(false),
            properties_for: signal(None),
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

    /// Like [`Self::shown_place`], along with the document's total page count when known.
    pub(crate) fn shown_place_with_total(self) -> Option<(PathBuf, usize, usize)> {
        let (path, page) = self.shown_place()?;
        Some((path, page, self.pages.count()))
    }

    /// Whether any tab is open.
    pub(crate) fn has_tabs(self) -> bool {
        self.tabs.with(|tabs| !tabs.all().is_empty())
    }

    /// Whether a tab of the file at `path` is open (untracked).
    pub(crate) fn has_file(self, path: &std::path::Path) -> bool {
        self.tabs.with_untracked(|tabs| tabs.find(path).is_some())
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

    /// Whether any tab's context menu is currently open (tracked).
    pub(crate) fn is_menu_open(self) -> bool {
        self.menu_open.get()
    }

    /// Notes whether a tab context menu is open.
    pub(crate) fn set_menu_open(self, open: bool) {
        self.menu_open.set(open);
    }

    /// The tab whose document Properties dialog is currently open (tracked).
    pub(crate) fn properties_tab(self) -> Option<TabId> {
        self.properties_for.get()
    }

    /// Opens the document Properties dialog for tab `id`.
    pub(crate) fn open_properties(self, id: TabId) {
        self.properties_for.set(Some(id));
    }

    /// Opens the document Properties dialog for the currently shown tab, if any.
    pub(crate) fn open_properties_shown(self) {
        if let Some(id) = self
            .tabs
            .with_untracked(|tabs| tabs.active().map(|tab| tab.id))
        {
            self.open_properties(id);
        }
    }

    /// Closes the document Properties dialog.
    pub(crate) fn close_properties(self) {
        self.properties_for.set(None);
    }

    /// Snapshot of tab `id`'s data, while it is open (tracked).
    pub(crate) fn tab_info(self, id: TabId) -> Option<model::Tab> {
        self.tabs.with(|tabs| tabs.get(id).cloned())
    }

    /// Returns the known page count for `id` if it is currently the active loaded tab.
    pub(crate) fn page_count_for(self, id: TabId) -> Option<usize> {
        let is_active = self
            .tabs
            .with_untracked(|tabs| tabs.active().map(|tab| tab.id) == Some(id));
        if is_active {
            let count = untracked(|| self.pages.count());
            (count > 0).then_some(count)
        } else {
            None
        }
    }

    /// Marks every tab pointing to `doc_path` (directly or via `.expdf` working PDF) as dirty or clean.
    pub(crate) fn set_dirty_resolved(self, doc_path: &std::path::Path, dirty: bool) {
        let needle = doc_path.to_string_lossy().to_lowercase();
        self.tabs.update(|tabs| {
            tabs.set_dirty_matching(
                |tab_path| {
                    tab_path.to_string_lossy().to_lowercase() == needle
                        || crate::expdf::resolve_open_path(tab_path)
                            .to_string_lossy()
                            .to_lowercase()
                            == needle
                },
                dirty,
            );
        });
    }

    /// Closes every unpinned tab other than `id`.
    pub(crate) fn close_others_of(self, id: TabId) {
        self.change(|tabs| tabs.close_others(id));
    }

    /// Closes every unpinned tab to the right of `id`.
    pub(crate) fn close_right_of(self, id: TabId) {
        self.change(|tabs| tabs.close_to_the_right(id));
    }

    /// Closes every unpinned tab to the left of `id`.
    pub(crate) fn close_left_of(self, id: TabId) {
        self.change(|tabs| tabs.close_to_the_left(id));
    }

    /// Closes all unpinned tabs (or all tabs if only pinned tabs remain).
    pub(crate) fn close_all(self) {
        self.change(Tabs::close_all_tabs);
    }

    /// Whether there is at least one unpinned tab other than `id`.
    pub(crate) fn can_close_others(self, id: TabId) -> bool {
        self.tabs.with(|tabs| tabs.can_close_others(id))
    }

    /// Whether there is at least one unpinned tab to the right of `id`.
    pub(crate) fn can_close_right(self, id: TabId) -> bool {
        self.tabs.with(|tabs| tabs.can_close_right(id))
    }

    /// Whether there is at least one unpinned tab to the left of `id`.
    pub(crate) fn can_close_left(self, id: TabId) -> bool {
        self.tabs.with(|tabs| tabs.can_close_left(id))
    }

    /// Pins or unpins the tab `id`.
    pub(crate) fn toggle_pin(self, id: TabId) {
        self.tabs.update(|tabs| {
            let pinned = tabs.get(id).is_some_and(|tab| tab.pinned);
            tabs.set_pinned(id, !pinned);
        });
    }

    /// Moves the tab `id` to `to_index` within its pinned or unpinned section, returning `true`
    /// iff the tab exists and its strip position actually changed.
    pub(crate) fn move_tab(self, id: TabId, to_index: usize) -> bool {
        let target = self.tabs.with_untracked(|tabs| {
            let from = tabs.index_of(id)?;
            let clamped = tabs.clamped_target_index(id, to_index)?;
            (clamped != from).then_some(clamped)
        });
        let Some(target) = target else {
            return false;
        };
        self.tabs.update(|tabs| tabs.move_to(id, target));
        true
    }

    /// Steps the tab `id` by `delta` slots within its pinned or unpinned section, returning `true`
    /// iff its strip position actually changed.
    pub(crate) fn step_tab_by(self, id: TabId, delta: isize) -> bool {
        if delta == 0 {
            return false;
        }
        let Some(next_index) = self.tabs.with_untracked(|tabs| {
            tabs.index_of(id)
                .map(|from| from.saturating_add_signed(delta))
        }) else {
            return false;
        };
        self.move_tab(id, next_index)
    }

    /// Sets or clears a custom accent colour on the tab `id`.
    pub(crate) fn set_color(self, id: TabId, color: Option<GroupColor>) {
        self.tabs.update(|tabs| tabs.set_tab_color(id, color));
    }

    /// Reveals the file of tab `id` in the OS file manager.
    pub(crate) fn show_in_folder(self, id: TabId) {
        if let Some(path) = self
            .tabs
            .with_untracked(|tabs| tabs.get(id).map(|tab| tab.path.clone()))
        {
            reveal_in_folder(&path);
        }
    }

    /// Copies the full file path of tab `id` to the system clipboard.
    pub(crate) fn copy_file_path(self, id: TabId) {
        if let Some(path) = self
            .tabs
            .with_untracked(|tabs| tabs.get(id).map(|tab| tab.path.clone()))
        {
            drop(set_clipboard(path.to_string_lossy().into_owned()));
        }
    }

    /// Returns the next or previous supported document in the shown tab's folder, wrapping around.
    pub(crate) fn adjacent_in_folder(self, step: Step) -> Option<PathBuf> {
        let current = untracked(|| self.shown_path())?;
        adjacent_document_in_folder(&current, step)
    }

    /// Renames the currently shown tab's file on disk and updates the tab in place.
    pub(crate) fn rename_shown_file(self, new_name: &str) -> Result<PathBuf, String> {
        let (id, old_path) = self
            .tabs
            .with_untracked(|tabs| {
                tabs.active()
                    .filter(|tab| !tab.missing)
                    .map(|tab| (tab.id, tab.path.clone()))
            })
            .ok_or_else(|| t!("rename.no_document"))?;
        let final_name = validate_rename_filename(&old_path, new_name)
            .ok_or_else(|| t!("rename.invalid_name"))?;
        let parent = old_path.parent().ok_or_else(|| t!("rename.invalid_name"))?;
        let target = parent.join(&final_name);
        if target == old_path {
            return Ok(old_path);
        }
        if target.exists() {
            return Err(t!("rename.already_exists"));
        }
        std::fs::rename(&old_path, &target).map_err(|err| err.to_string())?;
        self.tabs.update(|tabs| tabs.relocate(id, target.clone()));
        Ok(target)
    }

    /// Persists the current open tabs (path, page, pinned, custom colour) as the saved tab group.
    pub(crate) fn save_tab_group(self) {
        let current = untracked(|| self.current());
        let saved = serialize_tab_group(&current);
        if saved.is_empty() {
            return;
        }
        self.live.change(|settings| {
            settings.set_value(SAVED_TAB_GROUP, Some(toml::Value::Array(saved)));
        });
    }

    /// Whether a saved tab group is available to restore (tracked).
    pub(crate) fn has_saved_tab_group(self) -> bool {
        self.live.read(|settings| {
            settings
                .value(SAVED_TAB_GROUP)
                .and_then(toml::Value::as_array)
                .is_some_and(|entries| !entries.is_empty())
        })
    }

    /// Restores the previously saved tab group into this pane.
    pub(crate) fn restore_tab_group(self) {
        let entries = untracked(|| {
            self.live.read(|settings| {
                settings
                    .value(SAVED_TAB_GROUP)
                    .and_then(toml::Value::as_array)
                    .map(|array| deserialize_tab_group(array))
                    .unwrap_or_default()
            })
        });
        if entries.is_empty() {
            return;
        }
        self.leave_shown();
        self.tabs.update(|tabs| {
            for entry in entries {
                let id = match tabs.find(&entry.path) {
                    Some(existing) => {
                        tabs.activate(existing);
                        existing
                    }
                    None => tabs.open(entry.path, entry.page),
                };
                if entry.pinned {
                    tabs.set_pinned(id, true);
                }
                if entry.color.is_some() {
                    tabs.set_tab_color(id, entry.color);
                }
            }
        });
        self.arrive();
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

    /// Takes tab `id` out at its current page and view, saving them first if it is currently shown.
    pub(crate) fn take_tab(self, id: TabId) -> Option<Closed> {
        self.take_tabs(&[id]).into_iter().next()
    }

    /// Takes `clicked` — or all multi-selected tabs in strip order when `clicked` is part of a 2+
    /// tab selection — saving the shown tab's page and view first if it is among them.
    pub(crate) fn take_tabs_for(self, clicked: TabId) -> Vec<Closed> {
        let ids = self
            .tabs
            .with_untracked(|tabs| tabs.target_ids_for(clicked));
        self.take_tabs(&ids)
    }

    /// Takes every tab in `ids` in the order given, saving the shown tab's page and view first if
    /// it is among them and restoring the newly shown tab afterward.
    pub(crate) fn take_tabs(self, ids: &[TabId]) -> Vec<Closed> {
        if ids.is_empty() {
            return Vec::new();
        }
        let takes_shown = self
            .tabs
            .with_untracked(|tabs| tabs.active().is_some_and(|tab| ids.contains(&tab.id)));
        if takes_shown {
            self.leave_shown();
        }
        let mut taken = Vec::new();
        self.tabs.update(|tabs| {
            taken = tabs.take_many(ids);
        });
        if takes_shown && !taken.is_empty() {
            self.arrive();
        }
        taken
    }

    /// Takes all multi-selected tabs in strip order when 2+ are selected, otherwise the shown tab.
    pub(crate) fn take_selected_or_shown(self) -> Vec<Closed> {
        let ids = self.tabs.with_untracked(Tabs::selected_or_active_ids);
        self.take_tabs(&ids)
    }

    /// Copies all multi-selected non-missing tabs in strip order when 2+ are selected, otherwise
    /// the shown tab.
    pub(crate) fn copy_selected_or_shown(self) -> Vec<Closed> {
        let tabs = untracked(|| self.current());
        let ids = tabs.selected_or_active_ids();
        tabs.copy_many(&ids)
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

    /// Shows `copy`'s file here (V.9): its tab when there is one, else `copy` as a new tab; then sends `then` to
    /// `canvas` once the file is open, or at once when it was shown already.
    pub(crate) fn show_then(self, copy: Closed, then: Then, canvas: ElementRef) {
        let shown = self.tabs.with_untracked(|tabs| {
            tabs.active()
                .is_some_and(|tab| !tab.missing && tab.path == copy.path)
        });
        if shown {
            canvas.command(then.0, then.1);
            return;
        }
        self.change(|tabs| match tabs.find(&copy.path) {
            Some(id) => tabs.activate(id),
            None => {
                tabs.adopt(copy);
            }
        });
        self.restore.with(|restore| {
            if let Some(restore) = restore {
                restore.then = Some(then);
            }
        });
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
            Action::CloseOthers => self.close_others_of(id),
            Action::CloseToTheRight => self.close_right_of(id),
            Action::CloseToTheLeft => self.close_left_of(id),
            Action::CloseAll => self.close_all(),
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

    /// Toggles `id` in the multi-selection set (V.12).
    pub(crate) fn ctrl_toggle_selected(self, id: TabId) {
        self.tabs.update(|tabs| tabs.ctrl_toggle_selected(id));
    }

    /// Selects the contiguous range of tabs from the active/anchor tab through `id` inclusive.
    pub(crate) fn select_range_to(self, id: TabId) {
        self.tabs.update(|tabs| tabs.select_range_to(id));
    }

    /// Clears all multi-selected tabs.
    pub(crate) fn clear_selected(self) {
        self.tabs.update(Tabs::clear_selected);
    }

    /// Whether `id` is multi-selected (V.12).
    pub(crate) fn is_selected(self, id: TabId) -> bool {
        self.tabs.with(|tabs| tabs.is_selected(id))
    }

    /// Number of currently multi-selected tabs.
    pub(crate) fn selected_count(self) -> usize {
        self.tabs.with(|tabs| tabs.selected_ids().len())
    }

    /// The currently active tab's id, if any (untracked).
    pub(crate) fn active_id(self) -> Option<TabId> {
        self.tabs
            .with_untracked(|tabs| tabs.active().map(|tab| tab.id))
    }

    /// Returns a duplicate of tab `id` at its current page/view without closing the original.
    pub(crate) fn copy_of(self, id: TabId) -> Option<Closed> {
        let mut tabs = self.current();
        tabs.take(id)
    }

    /// Takes up to `max` tabs to tile (the multi-selected tabs if >= 2, else up to `max` tabs from the strip,
    /// starting with the active tab), leaving at least one tab in the strip only if fewer than 2 tabs exist.
    pub(crate) fn take_for_tiling(self, max: usize) -> Vec<Closed> {
        let mut taken = Vec::new();
        self.change(|tabs| {
            let mut ids: Vec<TabId> = tabs.selected_ids().to_vec();
            if ids.len() < 2 {
                ids.clear();
                if let Some(active) = tabs.active().map(|t| t.id) {
                    ids.push(active);
                }
                for tab in tabs.all() {
                    if !ids.contains(&tab.id) {
                        ids.push(tab.id);
                    }
                    if ids.len() >= max {
                        break;
                    }
                }
            } else {
                ids.truncate(max);
            }
            tabs.clear_selected();
            if ids.len() < 2 {
                return;
            }
            for id in ids {
                if let Some(closed) = tabs.take(id) {
                    taken.push(closed);
                }
            }
        });
        taken
    }

    /// Adopts `taken` into one named tab group (V.12 untile).
    pub(crate) fn adopt_into_group(self, taken: Vec<Closed>, group_name: &str) {
        if taken.is_empty() {
            return;
        }
        self.change(|tabs| {
            let mut ids = Vec::with_capacity(taken.len());
            for tab in taken {
                ids.push(tabs.adopt(tab));
            }
            tabs.group_tabs(&ids, group_name);
        });
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
            "pages" => self.come_back(canvas),
            // Until the shown tab's view has come back, the views reported are the last file's.
            "view" if self.restore.with(|restore| restore.is_none()) => {
                let view = event.data.to_string();
                self.shown_view.with(|shown| *shown = Some(view));
            }
            _ => {}
        }
    }

    /// Takes the shown tab back to where it was left, now that its file is open, then sends the command waiting for
    /// it (V.9).
    fn come_back(self, canvas: ElementRef) {
        self.shown_view.with(Option::take);
        let Some(Restore { page, view, then }) = self.restore.with(Option::take) else {
            return;
        };
        match view {
            Some(view) => canvas.command("setView", view),
            None if page > 0 => canvas.command("goToPage", serde_json::json!({ "index": page })),
            None => {}
        }
        if let Some((name, args)) = then {
            canvas.command(name, args);
        }
    }

    /// Puts `tabs` in place of every tab (another workspace's, W.1) and shows their shown tab where it was left.
    /// `canvas` is the pane's: when it already has the file open it reports no new pages, so the tab is taken back
    /// here.
    pub(crate) fn replace(self, tabs: Tabs, canvas: ElementRef) {
        let before = untracked(|| self.shown_path());
        self.leave_shown();
        self.tabs.set(tabs);
        self.arrive();
        if before.is_some() && untracked(|| self.shown_path()) == before {
            self.come_back(canvas);
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
            let resolved = crate::expdf::resolve_open_path(&path);
            JsonValue::from(resolved.to_string_lossy().into_owned())
        })
    }
}

/// Shows the open dialog, and hands the files picked to `then`.
pub(crate) fn pick_documents(then: impl FnOnce(Vec<PathBuf>) + 'static) {
    let extensions: Vec<&str> = EXTENSIONS
        .iter()
        .copied()
        .chain(["expdf", "djvu", "djv", "chm", "cbr"])
        .collect();
    let dialog = OpenDialog::files()
        .multiple(true)
        .title(t!("open.title"))
        .filter(FileFilter::new(t!("open.filter"), extensions));
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
    /// A canvas command to send once the view is back (V.9: a link opened from another pane).
    then: Option<Then>,
}

/// A canvas command and its arguments.
pub(crate) type Then = (&'static str, JsonValue);

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
            then: None,
        }
    }
}

/// Opens the OS file manager selecting `path` (or its parent folder on Linux).
fn reveal_in_folder(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        if let Err(err) = std::process::Command::new("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()
        {
            log::warn!("show_in_folder: explorer.exe failed: {err}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
        {
            log::warn!("show_in_folder: open -R failed: {err}");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = path.parent().unwrap_or(path);
        if let Err(err) = std::process::Command::new("xdg-open").arg(target).spawn() {
            log::warn!("show_in_folder: xdg-open failed: {err}");
        }
    }
}

/// One tab stored in [`SAVED_TAB_GROUP`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedGroupEntry {
    path: PathBuf,
    page: usize,
    pinned: bool,
    color: Option<GroupColor>,
}

fn serialize_tab_group(tabs: &Tabs) -> Vec<toml::Value> {
    tabs.all()
        .iter()
        .map(|tab| {
            let mut row = toml::Table::new();
            row.insert(
                "path".to_owned(),
                toml::Value::String(tab.path.to_string_lossy().into_owned()),
            );
            row.insert(
                "page".to_owned(),
                toml::Value::Integer(i64::try_from(tab.page).unwrap_or(0)),
            );
            if tab.pinned {
                row.insert("pinned".to_owned(), toml::Value::Boolean(true));
            }
            if let Some(color) = tab.color {
                row.insert(
                    "color".to_owned(),
                    toml::Value::String(color.id().to_owned()),
                );
            }
            toml::Value::Table(row)
        })
        .collect()
}

fn deserialize_tab_group(values: &[toml::Value]) -> Vec<SavedGroupEntry> {
    values
        .iter()
        .filter_map(|value| {
            let table = value.as_table()?;
            let path = PathBuf::from(table.get("path")?.as_str()?);
            let page = table
                .get("page")
                .and_then(toml::Value::as_integer)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0);
            let pinned = table
                .get("pinned")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false);
            let color = table
                .get("color")
                .and_then(toml::Value::as_str)
                .map(GroupColor::parse);
            Some(SavedGroupEntry {
                path,
                page,
                pinned,
                color,
            })
        })
        .collect()
}

/// Whether `path` has a lowercase file extension listed in [`EXTENSIONS`].
fn is_supported_document_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            let lower = ext.to_ascii_lowercase();
            EXTENSIONS.contains(&lower.as_str())
        })
}

/// Filters `candidates` to supported documents, sorts them case-insensitively by file name
/// (breaking ties by exact path), and returns the next or previous entry relative to `current`.
fn select_adjacent_document(
    current: &Path,
    mut candidates: Vec<PathBuf>,
    step: Step,
) -> Option<PathBuf> {
    candidates.retain(|path| is_supported_document_path(path));
    candidates.sort_by(|a, b| {
        let a_key = a
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let b_key = b
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        a_key.cmp(&b_key).then_with(|| a.cmp(b))
    });
    if candidates.len() < 2 {
        return None;
    }
    let current_name_lower = current
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase());
    let pos = candidates
        .iter()
        .position(|path| path == current)
        .or_else(|| {
            let target = current_name_lower.as_deref()?;
            candidates.iter().position(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().to_ascii_lowercase() == target)
            })
        })?;
    let len = candidates.len();
    let next = match step {
        Step::Next => (pos + 1) % len,
        Step::Previous => (pos + len - 1) % len,
    };
    candidates.into_iter().nth(next)
}

/// Lists regular files in `current`'s parent directory and returns the adjacent supported document.
fn adjacent_document_in_folder(current: &Path, step: Step) -> Option<PathBuf> {
    let parent = current.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    let entries = std::fs::read_dir(parent).ok()?;
    let candidates: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    select_adjacent_document(current, candidates, step)
}

/// Validates and normalizes `new_name` for renaming `old_path`, preserving the original extension
/// when `new_name` does not specify one.
fn validate_rename_filename(old_path: &Path, new_name: &str) -> Option<String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.chars().any(|c| {
            matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()
        })
    {
        return None;
    }
    if Path::new(trimmed).extension().is_none()
        && let Some(orig_ext) = old_path.extension().and_then(|ext| ext.to_str())
        && !orig_ext.is_empty()
    {
        return Some(format!("{trimmed}.{orig_ext}"));
    }
    Some(trimmed.to_owned())
}

/// What a tab command does to the shown tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Close,
    CloseOthers,
    CloseToTheRight,
    CloseToTheLeft,
    CloseAll,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn tab_style_defaults_to_vivaldi_and_parses_chrome() {
        assert_eq!(TabStyle::default(), TabStyle::Vivaldi);
        assert_eq!(TabStyle::parse(None), TabStyle::Vivaldi);
        assert_eq!(TabStyle::parse(Some("vivaldi")), TabStyle::Vivaldi);
        assert_eq!(TabStyle::parse(Some("chrome")), TabStyle::Chrome);
    }

    #[test]
    fn tab_min_width_defaults_to_250_px_and_clamps() {
        let mut settings = Settings::default();
        assert!((min_width_px(&settings) - 250.0).abs() < f32::EPSILON);

        settings.set_number(LEGACY_TAB_MIN_WIDTH_CM, Some(6.0));
        let legacy_px = min_width_px(&settings);
        assert!((226.0..228.0).contains(&legacy_px));

        settings.set_number(TAB_MIN_WIDTH_PX, Some(10.0));
        assert!((min_width_px(&settings) - MIN_TAB_MIN_WIDTH_PX as f32).abs() < f32::EPSILON);

        settings.set_number(TAB_MIN_WIDTH_PX, Some(999.0));
        assert!((min_width_px(&settings) - MAX_TAB_MIN_WIDTH_PX as f32).abs() < f32::EPSILON);
    }

    #[test]
    fn tab_height_defaults_to_34_px_clamps_and_scales_title_bar() {
        let mut settings = Settings::default();
        assert!((tab_height_px(&settings) - 34.0).abs() < f32::EPSILON);
        assert!((title_bar_height_px(&settings) - 46.0).abs() < f32::EPSILON);

        settings.set_number(TAB_HEIGHT_PX, Some(10.0));
        assert!((tab_height_px(&settings) - MIN_TAB_HEIGHT_PX as f32).abs() < f32::EPSILON);
        assert!((title_bar_height_px(&settings) - 40.0).abs() < f32::EPSILON);

        settings.set_number(TAB_HEIGHT_PX, Some(100.0));
        assert!((tab_height_px(&settings) - MAX_TAB_HEIGHT_PX as f32).abs() < f32::EPSILON);
        assert!((title_bar_height_px(&settings) - 68.0).abs() < f32::EPSILON);

        settings.set_number(TAB_HEIGHT_PX, Some(42.0));
        assert!((tab_height_px(&settings) - 42.0).abs() < f32::EPSILON);
        assert!((title_bar_height_px(&settings) - 54.0).abs() < f32::EPSILON);
    }

    #[test]
    fn saved_tab_group_round_trips_paths_pages_pins_and_colors() {
        let mut tabs = Tabs::default();
        let a = tabs.open(PathBuf::from("C:/docs/a.pdf"), 3);
        let b = tabs.open(PathBuf::from("C:/docs/b.pdf"), 12);
        tabs.set_pinned(a, true);
        tabs.set_tab_color(b, Some(GroupColor::Purple));
        let serialized = serialize_tab_group(&tabs);
        let restored = deserialize_tab_group(&serialized);
        assert_eq!(
            restored,
            vec![
                SavedGroupEntry {
                    path: PathBuf::from("C:/docs/a.pdf"),
                    page: 3,
                    pinned: true,
                    color: None,
                },
                SavedGroupEntry {
                    path: PathBuf::from("C:/docs/b.pdf"),
                    page: 12,
                    pinned: false,
                    color: Some(GroupColor::Purple),
                },
            ]
        );
    }

    #[test]
    fn select_adjacent_document_sorts_case_insensitively_filters_and_wraps() {
        let candidates = vec![
            PathBuf::from("C:/docs/zebra.PDF"),
            PathBuf::from("C:/docs/notes.txt"),
            PathBuf::from("C:/docs/Alpha.epub"),
            PathBuf::from("C:/docs/beta.pdf"),
            PathBuf::from("C:/docs/readme"),
        ];
        let alpha = PathBuf::from("C:/docs/Alpha.epub");
        let beta = PathBuf::from("C:/docs/beta.pdf");
        let zebra = PathBuf::from("C:/docs/zebra.PDF");

        assert_eq!(
            select_adjacent_document(&alpha, candidates.clone(), Step::Next),
            Some(beta.clone())
        );
        assert_eq!(
            select_adjacent_document(&zebra, candidates.clone(), Step::Next),
            Some(alpha.clone())
        );
        assert_eq!(
            select_adjacent_document(&alpha, candidates.clone(), Step::Previous),
            Some(zebra.clone())
        );
        // Case-insensitive filename fallback matches on Windows paths differing only in case.
        assert_eq!(
            select_adjacent_document(
                Path::new("C:/docs/BETA.PDF"),
                candidates.clone(),
                Step::Next
            ),
            Some(zebra)
        );
        // Fewer than two supported documents yields None.
        assert_eq!(
            select_adjacent_document(
                &beta,
                vec![beta.clone(), PathBuf::from("C:/docs/notes.txt")],
                Step::Next,
            ),
            None
        );
        // Current file not present in the folder yields None.
        assert_eq!(
            select_adjacent_document(Path::new("C:/docs/missing.pdf"), candidates, Step::Next),
            None
        );
    }

    #[test]
    fn validate_rename_filename_trims_preserves_extension_and_rejects_invalid() {
        let pdf = Path::new("C:/docs/old.pdf");
        assert_eq!(
            validate_rename_filename(pdf, "  Report  "),
            Some("Report.pdf".to_owned())
        );
        assert_eq!(
            validate_rename_filename(pdf, "Report.epub"),
            Some("Report.epub".to_owned())
        );
        assert_eq!(
            validate_rename_filename(Path::new("C:/docs/README"), "NOTES"),
            Some("NOTES".to_owned())
        );
        for invalid in [
            "",
            "   ",
            ".",
            "..",
            "sub/report.pdf",
            "sub\\report.pdf",
            "bad:name.pdf",
            "bad*name.pdf",
            "bad?name.pdf",
            "bad<name.pdf",
            "bad>name.pdf",
            "bad\"name.pdf",
            "bad|name.pdf",
            "bad\0name.pdf",
        ] {
            assert_eq!(
                validate_rename_filename(pdf, invalid),
                None,
                "expected {invalid:?} to be rejected"
            );
        }
    }
}
