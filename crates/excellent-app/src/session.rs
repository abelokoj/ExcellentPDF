//! Session restore (plan R.9): the window's tabs, each at its page, saved to the library (K.20) a second after they
//! last changed and when the app quits, and put back when it starts. Restoring is lazy (W.14): the shown tab's
//! document opens, the others stay rows until they are shown. A tab whose file has gone is marked missing, which the
//! strip greys and the document panel answers with "Locate…".
//!
//! Tab groups (R.13) are saved with their tabs: name, colour and whether collapsed. The split view (V.7) is saved
//! whole: the panes' layout, and each pane's tabs under its cell.
//!
//! The View state saved for a tab is the canvas's `view` event (JSON: the page and the place on it, the zoom, the
//! layout and the rotation; R.1, R.2), or `{"page": 3}` when the tab has no view at its page.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use excellent_store::Store;
use excellent_store::workspace::{self, ClosedTab, SavedGroup, SavedTab, WorkspaceId};
use nsh_core::element::desktop::on_app_quit;
use nsh_core::prelude::*;

use crate::panes::{CELLS, Layout, Panes};
use crate::tabs::{Closed, GroupColor, GroupId, Tabs};

/// How long after the last change the tabs are saved (§11.16: "about 1 s").
const SAVE_DELAY: Duration = Duration::from_secs(1);

/// The last session's split view: the panes' layout, and each cell's tabs.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    pub(crate) layout: Layout,
    pub(crate) tabs: [Tabs; CELLS],
}

/// The last session, if the library is open and has one. Each tab's file is looked for (one `stat` per tab) so the
/// missing ones show at once.
pub(crate) fn restored() -> Option<Restored> {
    let store = crate::library::store()?;
    match store.read(|conn| workspace::load_panes(conn, WorkspaceId::DEFAULT)) {
        Ok(saved) => Some(from_saved_panes(saved, Path::is_file)),
        Err(err) => {
            log::warn!("session: can't read the tabs: {err}");
            None
        }
    }
}

/// Saves the panes and their tabs a second after each change and as the app quits. Without a library it does
/// nothing.
pub(crate) fn keep(panes: Panes) {
    let Some(store) = crate::library::store() else {
        return;
    };
    let pending = stored(None::<Spawned<()>>);
    let first = stored(true);
    effect(move || {
        let saved = to_saved_panes(&panes.layout(), &panes.current());
        // The first run is the session as it was restored: nothing to save.
        if first.replace(false) {
            return;
        }
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        pending.replace(Some(set_timeout(SAVE_DELAY, move || save(store, saved))));
    });
    on_app_quit(move || {
        if let Some(task) = pending.replace(None) {
            task.cancel();
        }
        let saved = untracked(|| to_saved_panes(&panes.layout(), &panes.current()));
        save(store, saved);
        if let Err(err) = store.flush() {
            log::warn!("session: the tabs weren't saved: {err}");
        }
        async {}
    });
}

/// Hands the panes to the library's writer.
fn save(store: &'static Store, saved: workspace::Panes) {
    let _ = store.write(move |tx| {
        let result = workspace::save_panes(tx, WorkspaceId::DEFAULT, &saved);
        if let Err(err) = &result {
            log::warn!("session: can't save the tabs: {err}");
        }
        result
    });
}

/// The panes as the library keeps them: each cell's tabs under its number.
fn to_saved_panes(layout: &Layout, tabs: &[Tabs]) -> workspace::Panes {
    workspace::Panes {
        layout: Some(layout.to_json()),
        panes: tabs.iter().map(to_saved).collect(),
    }
}

/// A saved session's panes; a session from before the split view is one pane.
fn from_saved_panes(saved: workspace::Panes, exists: impl Fn(&Path) -> bool) -> Restored {
    let layout = saved
        .layout
        .as_deref()
        .map_or_else(Layout::default, Layout::from_json);
    let mut panes = saved.panes.into_iter();
    let tabs = std::array::from_fn(|_| {
        panes
            .next()
            .map(|tabs| from_saved(tabs, &exists))
            .unwrap_or_default()
    });
    Restored { layout, tabs }
}

/// The tabs as the library keeps them.
fn to_saved(tabs: &Tabs) -> workspace::Tabs {
    // The groups in strip order, each at the place its first tab gives it.
    let mut place_of: HashMap<GroupId, usize> = HashMap::new();
    let mut groups = Vec::new();
    let open = tabs
        .all()
        .iter()
        .map(|tab| {
            let group = tab.group.and_then(|id| {
                let group = tabs.group(id)?;
                Some(*place_of.entry(id).or_insert_with(|| {
                    groups.push(SavedGroup {
                        name: group.name.clone(),
                        color: group.color.id().to_owned(),
                        collapsed: group.collapsed,
                    });
                    groups.len() - 1
                }))
            });
            saved_tab(&tab.path, tab.page, tab.view.as_deref(), tab.pinned, group)
        })
        .collect();
    workspace::Tabs {
        open,
        active: tabs.active_index(),
        groups,
        closed: tabs
            .closed()
            .map(|closed| ClosedTab {
                tab: saved_tab(
                    &closed.path,
                    closed.page,
                    closed.view.as_deref(),
                    closed.pinned,
                    None,
                ),
                position: closed.index,
            })
            .collect(),
    }
}

fn saved_tab(
    path: &Path,
    page: usize,
    view: Option<&str>,
    pinned: bool,
    group: Option<usize>,
) -> SavedTab {
    let view = view.filter(|view| page_of(view) == page).map_or_else(
        || serde_json::json!({ "page": page }).to_string(),
        str::to_owned,
    );
    SavedTab {
        path: path.to_path_buf(),
        view,
        pinned,
        group,
    }
}

/// The tabs of a saved session; `exists` tells which files are still there.
///
/// shortcut: the order the tabs were last shown in (Ctrl+Tab's) isn't saved, so it starts as the strip's; save it
/// when readers miss it.
fn from_saved(saved: workspace::Tabs, exists: impl Fn(&Path) -> bool) -> Tabs {
    let mut tabs = Tabs::default();
    let groups: Vec<GroupId> = saved
        .groups
        .into_iter()
        .map(|group| {
            tabs.restore_group(group.name, GroupColor::parse(&group.color), group.collapsed)
        })
        .collect();
    let ids: Vec<_> = saved
        .open
        .into_iter()
        .map(|tab| {
            let missing = !exists(&tab.path);
            let group = tab.group.and_then(|place| groups.get(place).copied());
            let id = tabs.restore(tab.path, page_of(&tab.view), tab.pinned, missing, group);
            tabs.set_view(id, tab.view);
            id
        })
        .collect();
    // A saved group none of whose tabs came back (all pinned, or edited by hand) goes.
    tabs.prune_groups();
    // Saved newest first; remembered oldest first.
    for closed in saved.closed.into_iter().rev() {
        tabs.remember_closed(Closed {
            page: page_of(&closed.tab.view),
            view: Some(closed.tab.view),
            path: closed.tab.path,
            pinned: closed.tab.pinned,
            index: closed.position,
        });
    }
    // Showing a tab expands its group: a collapsed group stays so unless it holds the tab shown.
    if let Some(&id) = saved.active.and_then(|index| ids.get(index)) {
        tabs.activate(id);
    }
    tabs
}

/// The page a saved View state is at; the first when it names none (a broken or foreign state).
pub(crate) fn page_of(view: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(view)
        .ok()
        .and_then(|view| view.get("page")?.as_u64())
        .and_then(|page| usize::try_from(page).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("C:/papers/{name}.pdf"))
    }

    /// Five tabs (the first pinned, the last two in a named, collapsed group), one closed, the second shown, each
    /// at a page of its own.
    fn session() -> Tabs {
        let mut tabs = Tabs::default();
        let a = tabs.open(path("a"), 0);
        let b = tabs.open(path("b"), 0);
        let c = tabs.open(path("c"), 0);
        let d = tabs.open(path("d"), 0);
        let e = tabs.open(path("e"), 0);
        let f = tabs.open(path("f"), 0);
        let group = tabs.new_group(e).expect("a group");
        tabs.step_tab(f, false);
        tabs.rename_group(group, "Thesis");
        tabs.recolor_group(group, GroupColor::Green);
        tabs.activate(b);
        assert!(tabs.toggle_group(group));
        tabs.set_pinned(a, true);
        tabs.set_page(a, 4);
        tabs.set_page(b, 12);
        tabs.set_page(c, 0);
        tabs.set_page(d, 7);
        tabs.close(d);
        tabs.activate(b);
        tabs
    }

    #[test]
    fn a_session_comes_back_with_every_tab_at_its_page() {
        let saved = to_saved(&session());
        let restored = from_saved(saved.clone(), |_| true);
        assert_eq!(to_saved(&restored), saved);
        let shown = restored.active().expect("a tab is shown");
        assert_eq!((shown.path.clone(), shown.page), (path("b"), 12));
        assert!(restored.all()[0].pinned);
        let closed = restored.closed().next().expect("a closed tab");
        assert_eq!((closed.path.clone(), closed.page), (path("d"), 7));
        assert!(restored.all().iter().all(|tab| !tab.missing));
        let group = restored.groups().first().expect("the group");
        assert_eq!(
            (group.name.as_str(), group.color, group.collapsed),
            ("Thesis", GroupColor::Green, true)
        );
        assert_eq!(restored.members(group.id).count(), 2);
    }

    #[test]
    fn a_tab_keeps_its_view_while_it_is_at_the_view_page() {
        let view = r#"{"page":12,"fit":"custom","zoom":1.5}"#;
        let mut tabs = session();
        let shown = tabs.active().expect("a tab is shown").id;
        tabs.set_view(shown, view.to_owned());
        let saved = to_saved(&tabs);
        assert_eq!(saved.open[1].view, view);
        // Restored and saved again, it is the same.
        assert_eq!(to_saved(&from_saved(saved, |_| true)).open[1].view, view);
        // Sent to another page alone (a launch's), the view no longer counts.
        tabs.set_page(shown, 30);
        assert_eq!(to_saved(&tabs).open[1].view, r#"{"page":30}"#);
    }

    #[test]
    fn a_split_comes_back_with_each_pane_s_tabs() {
        let mut layout = Layout::default();
        let split = layout.plan_split(crate::panes::Side::Right).expect("room");
        layout.apply(split);
        let mut second = Tabs::default();
        second.open(path("z"), 3);
        let tabs = [session(), second, Tabs::default(), Tabs::default()];
        let saved = to_saved_panes(&layout, &tabs);
        let restored = from_saved_panes(saved.clone(), |_| true);
        assert_eq!(restored.layout, layout);
        assert_eq!(to_saved_panes(&restored.layout, &restored.tabs), saved);
        let shown = restored.tabs[1].active().expect("the second pane's tab");
        assert_eq!((shown.path.clone(), shown.page), (path("z"), 3));
        // A session from before the split view: one pane, the first.
        let old = workspace::Panes {
            layout: None,
            panes: vec![to_saved(&session())],
        };
        let restored = from_saved_panes(old, |_| true);
        assert_eq!(restored.layout, Layout::default());
        assert_eq!(restored.tabs[0].all().len(), 5);
        assert!(restored.tabs[1].all().is_empty());
    }

    #[test]
    fn a_tab_whose_file_has_gone_comes_back_missing() {
        let restored = from_saved(to_saved(&session()), |p| p != path("c"));
        let missing: Vec<_> = restored
            .all()
            .iter()
            .filter(|tab| tab.missing)
            .map(|tab| tab.path.clone())
            .collect();
        assert_eq!(missing, [path("c")]);
    }

    #[test]
    fn a_view_state_without_a_page_starts_at_the_first() {
        assert_eq!(page_of(r#"{"page":7,"zoom":1.5}"#), 7);
        assert_eq!(page_of("{}"), 0);
        assert_eq!(page_of("not json"), 0);
        assert_eq!(page_of(r#"{"page":-3}"#), 0);
    }
}
