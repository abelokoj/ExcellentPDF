//! The tabs of a window (R.12), as data: which files are open in which order, which one is shown, the order they were
//! last shown in, and the ones closed lately. No UI or OS types: the strip and the commands work through it.
//!
//! A tab holds no document while it is not shown (W.14's rule, kept from the start): only its path and the page it
//! was left at. Pinned tabs stay together at the start of the strip.

use std::path::{Path, PathBuf};

/// How many closed tabs can be reopened.
const CLOSED_KEPT: usize = 25;

/// A tab, for as long as the window keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TabId(pub(super) u64);

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tab {
    pub(crate) id: TabId,
    pub(crate) path: PathBuf,
    pub(crate) pinned: bool,
    /// Unsaved changes (shown as a dot); set once documents can be edited (K.14, K.15).
    pub(crate) dirty: bool,
    /// The page it was left at, from 0.
    pub(crate) page: usize,
}

impl Tab {
    /// Its name in the strip: the file's name.
    pub(crate) fn title(&self) -> String {
        file_name(&self.path)
    }
}

/// A closed tab, kept so it can be reopened where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Closed {
    pub(crate) path: PathBuf,
    pub(crate) page: usize,
    pub(crate) pinned: bool,
    /// Its place in the strip when it was closed.
    index: usize,
}

impl Closed {
    /// Its name: the file's name.
    pub(crate) fn title(&self) -> String {
        file_name(&self.path)
    }
}

/// The order Ctrl+Tab goes through the tabs in (`tab_cycle_order` in `settings.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Cycle {
    /// The order they were last shown in, as Enhanced and most editors do.
    #[default]
    Recent,
    /// Their order in the strip.
    Strip,
}

impl Cycle {
    /// The setting's value, or the default.
    pub(crate) fn parse(id: Option<&str>) -> Self {
        match id {
            Some("strip") => Self::Strip,
            _ => Self::Recent,
        }
    }
}

/// Forward or back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Next,
    Previous,
}

/// The tabs of a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tabs {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    /// Most recently shown first.
    recent: Vec<TabId>,
    /// Most recently closed last.
    closed: Vec<Closed>,
    next_id: u64,
}

impl Tabs {
    /// The tabs, in the strip's order.
    pub(crate) fn all(&self) -> &[Tab] {
        &self.tabs
    }

    /// The tab shown, if any.
    pub(crate) fn active(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.get(id))
    }

    /// The tab `id`, while it is open.
    pub(crate) fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The closed tabs, most recently closed first.
    pub(crate) fn closed(&self) -> impl Iterator<Item = &Closed> {
        self.closed.iter().rev()
    }

    /// Opens `path` in a new tab at the end of the strip and shows it. A file open already gets another tab: the
    /// tab list shows the duplicates.
    pub(crate) fn open(&mut self, path: PathBuf, page: usize) -> TabId {
        let index = self.tabs.len();
        self.insert(index, path, page, false)
    }

    /// Shows the tab `id`.
    pub(crate) fn activate(&mut self, id: TabId) {
        if self.get(id).is_none() {
            return;
        }
        self.active = Some(id);
        self.recent.retain(|&other| other != id);
        self.recent.insert(0, id);
    }

    /// Closes the tab `id`, keeping it to reopen. Closing the tab shown shows the one shown before it.
    pub(crate) fn close(&mut self, id: TabId) {
        let Some(index) = self.index(id) else {
            return;
        };
        let tab = self.tabs.remove(index);
        self.closed.push(Closed {
            path: tab.path,
            page: tab.page,
            pinned: tab.pinned,
            index,
        });
        if self.closed.len() > CLOSED_KEPT {
            self.closed.remove(0);
        }
        self.recent.retain(|&other| other != id);
        if self.active == Some(id) {
            self.active = None;
            if let Some(&next) = self.recent.first() {
                self.activate(next);
            }
        }
    }

    /// Closes every tab but `id` and the pinned ones.
    pub(crate) fn close_others(&mut self, id: TabId) {
        let doomed: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| tab.id != id && !tab.pinned)
            .map(|tab| tab.id)
            .collect();
        self.close_all(&doomed, id);
    }

    /// Closes the tabs after `id` in the strip, except pinned ones.
    pub(crate) fn close_to_the_right(&mut self, id: TabId) {
        let Some(index) = self.index(id) else {
            return;
        };
        let doomed: Vec<TabId> = self.tabs[index + 1..]
            .iter()
            .filter(|tab| !tab.pinned)
            .map(|tab| tab.id)
            .collect();
        self.close_all(&doomed, id);
    }

    /// Closes every tab of a file but `keep`.
    pub(crate) fn close_duplicates_of(&mut self, keep: TabId) {
        let Some(path) = self.get(keep).map(|tab| tab.path.clone()) else {
            return;
        };
        let doomed: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| tab.id != keep && same_file(&tab.path, &path))
            .map(|tab| tab.id)
            .collect();
        self.close_all(&doomed, keep);
    }

    /// Reopens the tab closed last, where it was, and shows it.
    pub(crate) fn reopen_closed(&mut self) -> Option<TabId> {
        let closed = self.closed.pop()?;
        let index = closed.index.min(self.tabs.len());
        let id = self.insert(index, closed.path, closed.page, closed.pinned);
        Some(id)
    }

    /// Reopens the closed tab of `path` closed last (a row of the tab list).
    pub(crate) fn reopen(&mut self, path: &Path) -> Option<TabId> {
        let at = self.closed.iter().rposition(|closed| closed.path == path)?;
        let closed = self.closed.remove(at);
        self.closed.push(closed);
        self.reopen_closed()
    }

    /// Moves the tab `id` to `index` in the strip, kept among the pinned tabs or the others, as it is.
    pub(crate) fn move_to(&mut self, id: TabId, index: usize) {
        let Some(from) = self.index(id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let pinned = self.pinned_count();
        let index = if tab.pinned {
            index.min(pinned)
        } else {
            index.clamp(pinned, self.tabs.len())
        };
        self.tabs.insert(index, tab);
    }

    /// Moves the tab `id` one place along the strip.
    pub(crate) fn move_by(&mut self, id: TabId, step: Step) {
        if let Some(index) = self.index(id) {
            let to = match step {
                Step::Next => index + 1,
                Step::Previous => index.saturating_sub(1),
            };
            self.move_to(id, to);
        }
    }

    /// Pins or unpins the tab `id`: pinned, it goes to the end of the pinned tabs; unpinned, to the start of the
    /// others.
    pub(crate) fn set_pinned(&mut self, id: TabId, pinned: bool) {
        let Some(index) = self.index(id) else {
            return;
        };
        if self.tabs[index].pinned == pinned {
            return;
        }
        let mut tab = self.tabs.remove(index);
        tab.pinned = pinned;
        let at = self.pinned_count();
        self.tabs.insert(at, tab);
    }

    /// Notes the page the tab `id` shows.
    pub(crate) fn set_page(&mut self, id: TabId, page: usize) {
        if let Some(tab) = self.tab_mut(id) {
            tab.page = page;
        }
    }

    /// The tab Ctrl+Tab (`Next`) or Ctrl+Shift+Tab goes to from the one shown, in `cycle` order.
    pub(crate) fn cycle(&self, cycle: Cycle, step: Step) -> Option<TabId> {
        let order: Vec<TabId> = match cycle {
            Cycle::Recent => self.recent.clone(),
            Cycle::Strip => self.tabs.iter().map(|tab| tab.id).collect(),
        };
        let len = order.len();
        if len < 2 {
            return None;
        }
        let at = self
            .active
            .and_then(|active| order.iter().position(|&id| id == active))
            .unwrap_or(0);
        let to = match step {
            Step::Next => (at + 1) % len,
            Step::Previous => (at + len - 1) % len,
        };
        Some(order[to])
    }

    /// The files open in more than one tab, each with its tabs in strip order.
    pub(crate) fn duplicates(&self) -> Vec<Vec<TabId>> {
        let mut groups: Vec<Vec<&Tab>> = Vec::new();
        for tab in &self.tabs {
            match groups
                .iter_mut()
                .find(|group| same_file(&group[0].path, &tab.path))
            {
                Some(group) => group.push(tab),
                None => groups.push(vec![tab]),
            }
        }
        groups
            .into_iter()
            .filter(|group| group.len() > 1)
            .map(|group| group.into_iter().map(|tab| tab.id).collect())
            .collect()
    }

    fn insert(&mut self, index: usize, path: PathBuf, page: usize, pinned: bool) -> TabId {
        self.next_id += 1;
        let id = TabId(self.next_id);
        let index = if pinned {
            index.min(self.pinned_count())
        } else {
            index.clamp(self.pinned_count(), self.tabs.len())
        };
        self.tabs.insert(
            index,
            Tab {
                id,
                path,
                pinned,
                dirty: false,
                page,
            },
        );
        self.activate(id);
        id
    }

    /// Closes `doomed`, then shows `shown` (the tab the command was for).
    fn close_all(&mut self, doomed: &[TabId], shown: TabId) {
        for &id in doomed {
            self.close(id);
        }
        self.activate(shown);
    }

    fn index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    fn pinned_count(&self) -> usize {
        self.tabs.iter().take_while(|tab| tab.pinned).count()
    }
}

/// A path's last part, or the whole path when it has none.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
        .into_owned()
}

/// Whether two paths name one file. shortcut: compares the paths as written, ignoring case (Windows); the
/// registry's `FileId` (K.5) is the real identity, to use once tabs hold one.
fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(names: &[&str]) -> (Tabs, Vec<TabId>) {
        let mut tabs = Tabs::default();
        let ids = names
            .iter()
            .map(|name| tabs.open(PathBuf::from(format!("C:/docs/{name}.pdf")), 0))
            .collect();
        (tabs, ids)
    }

    fn titles(tabs: &Tabs) -> Vec<String> {
        tabs.all().iter().map(Tab::title).collect()
    }

    #[test]
    fn closing_shows_the_tab_shown_before_and_reopening_puts_it_back() {
        let (mut tabs, ids) = tabs(&["a", "b", "c"]);
        tabs.activate(ids[0]);
        tabs.activate(ids[2]);
        tabs.set_page(ids[2], 41);
        tabs.close(ids[2]);
        assert_eq!(tabs.active().map(|tab| tab.id), Some(ids[0]));
        let back = tabs.reopen_closed().expect("a closed tab");
        assert_eq!(titles(&tabs), ["a.pdf", "b.pdf", "c.pdf"]);
        assert_eq!(
            tabs.active().map(|tab| (tab.id, tab.page)),
            Some((back, 41))
        );
        assert!(tabs.reopen_closed().is_none());
    }

    #[test]
    fn close_others_and_to_the_right_spare_pinned_tabs() {
        let (mut tabs, ids) = tabs(&["a", "b", "c", "d"]);
        tabs.set_pinned(ids[3], true);
        assert_eq!(titles(&tabs), ["d.pdf", "a.pdf", "b.pdf", "c.pdf"]);
        tabs.close_to_the_right(ids[0]);
        assert_eq!(titles(&tabs), ["d.pdf", "a.pdf"]);
        let (mut tabs, ids) = self::tabs(&["a", "b", "c"]);
        tabs.set_pinned(ids[0], true);
        tabs.close_others(ids[2]);
        assert_eq!(titles(&tabs), ["a.pdf", "c.pdf"]);
        assert_eq!(tabs.active().map(|tab| tab.id), Some(ids[2]));
    }

    #[test]
    fn tabs_move_within_their_group() {
        let (mut tabs, ids) = tabs(&["a", "b", "c"]);
        tabs.set_pinned(ids[2], true);
        tabs.move_to(ids[0], 0);
        assert_eq!(
            titles(&tabs),
            ["c.pdf", "a.pdf", "b.pdf"],
            "not before a pinned tab"
        );
        tabs.move_by(ids[0], Step::Next);
        assert_eq!(titles(&tabs), ["c.pdf", "b.pdf", "a.pdf"]);
        tabs.move_to(ids[2], 9);
        assert_eq!(
            titles(&tabs),
            ["c.pdf", "b.pdf", "a.pdf"],
            "pinned stays pinned"
        );
        tabs.set_pinned(ids[2], false);
        tabs.move_to(ids[2], 9);
        assert_eq!(titles(&tabs), ["b.pdf", "a.pdf", "c.pdf"]);
    }

    #[test]
    fn ctrl_tab_follows_recent_use_or_the_strip() {
        let (mut tabs, ids) = tabs(&["a", "b", "c"]);
        tabs.activate(ids[0]);
        // Shown last: a, then c, then b.
        assert_eq!(tabs.cycle(Cycle::Recent, Step::Next), Some(ids[2]));
        assert_eq!(tabs.cycle(Cycle::Recent, Step::Previous), Some(ids[1]));
        assert_eq!(tabs.cycle(Cycle::Strip, Step::Next), Some(ids[1]));
        assert_eq!(tabs.cycle(Cycle::Strip, Step::Previous), Some(ids[2]));
        let (one, _) = self::tabs(&["a"]);
        assert_eq!(one.cycle(Cycle::Recent, Step::Next), None);
    }

    #[test]
    fn duplicates_are_found_and_closed() {
        let (mut tabs, ids) = tabs(&["Report", "notes", "report"]);
        assert_eq!(tabs.duplicates(), vec![vec![ids[0], ids[2]]]);
        tabs.close_duplicates_of(ids[2]);
        assert_eq!(titles(&tabs), ["notes.pdf", "report.pdf"]);
    }

    #[test]
    fn a_closed_tab_is_reopened_by_its_path_and_the_list_is_bounded() {
        let (mut tabs, ids) = tabs(&["a", "b"]);
        tabs.close(ids[0]);
        tabs.close(ids[1]);
        assert!(tabs.active().is_none());
        assert_eq!(
            tabs.closed().map(Closed::title).collect::<Vec<_>>(),
            ["b.pdf", "a.pdf"]
        );
        tabs.reopen(Path::new("C:/docs/a.pdf"))
            .expect("a is closed");
        assert_eq!(titles(&tabs), ["a.pdf"]);
        for i in 0..40 {
            let id = tabs.open(PathBuf::from(format!("{i}.pdf")), 0);
            tabs.close(id);
        }
        assert_eq!(tabs.closed().count(), CLOSED_KEPT);
    }
}
