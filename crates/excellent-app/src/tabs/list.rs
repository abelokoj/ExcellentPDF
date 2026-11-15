//! The tab list (R.12 steps 3 and 4; reference `dev notes/images/ui-reference-vivaldi-tab-list.png`): every tab in
//! one searchable list, in counted sections: the open tabs, the files open in more than one tab (with "close the
//! copies"), the tabs closed lately, and the recent files not open (R.10). Typing searches the names and the folders; Return shows, reopens or closes;
//! Delete closes the highlighted open tab. A dialog, as the palette is: the page canvas is a window over the app's,
//! so a dropdown under the title bar would be drawn behind it.
//!
//! shortcut: the "inactive" section of the plan is every open tab but the shown one (only the shown tab has a
//! document open), so it is not listed apart; workspaces (W.1) will add their badge and their own tabs.

use nsh_base::{CommandItem, command};
use nsh_core::prelude::*;

use super::model::Tabs;
use super::{TabId, TabState};
use crate::list_dialog::{list_dialog, style};
use crate::recent::RecentState;

/// What a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Show(TabId),
    CloseCopies(TabId),
    Reopen(std::path::PathBuf),
    /// A recent file, at its page.
    OpenRecent(std::path::PathBuf, usize),
}

/// The tab list, while `open`.
pub(crate) fn tab_list(
    open: Signal<bool>,
    state: TabState,
    recent: RecentState,
) -> impl IntoElement {
    let close = move || open.set(false);
    recent.load();
    let rows = move || {
        let mut rows = state.tabs.with(rows);
        rows.extend(state.tabs.with(|tabs| recent_rows(tabs, &recent.files())));
        rows
    };
    let highlighted = signal(None::<SharedString>);
    let find = move |id: &str| rows().into_iter().find(|(key, _, _)| key == id);
    on_key("delete", move |_| {
        if let Some((_, _, Row::Show(tab))) = highlighted.get_untracked().and_then(|id| find(&id)) {
            state.close(tab);
        }
    });
    list_dialog(
        t!("tabs.list"),
        close,
        style(
            command()
                .items(move || {
                    rows()
                        .into_iter()
                        .map(|(_, item, _)| item)
                        .collect::<Vec<_>>()
                })
                .placeholder(t!("tabs.search"))
                .accessibility_label(t!("tabs.list"))
                .auto_focus(true)
                .on_highlight_change(move |id: &Option<SharedString>| highlighted.set(id.clone()))
                .on_select(move |id: &SharedString| {
                    let Some((_, _, row)) = find(id) else {
                        return;
                    };
                    match row {
                        Row::Show(tab) => {
                            close();
                            state.show(tab);
                        }
                        // Stays open: the list shows what is left.
                        Row::CloseCopies(tab) => {
                            state.show(tab);
                            state.run(super::Action::CloseDuplicates);
                        }
                        Row::Reopen(path) => {
                            close();
                            state.reopen(&path);
                        }
                        Row::OpenRecent(path, page) => {
                            close();
                            state.open_at(path, page);
                        }
                    }
                })
                .on_cancel(close),
        ),
    )
}

/// The list's rows: its item, keyed by its id, and what it does.
fn rows(tabs: &Tabs) -> Vec<(SharedString, CommandItem, Row)> {
    let shown = tabs.active().map(|tab| tab.id);
    let open_group: SharedString = t!("tabs.group.open", count = tabs.all().len()).into();
    let mut rows: Vec<_> = tabs
        .all()
        .iter()
        .map(|tab| {
            let id: SharedString = format!("open:{}", tab.id.0).into();
            let mut item = CommandItem::new(id.clone(), tab.title());
            item.keywords = vec![tab.path.to_string_lossy().into_owned().into()];
            item.group = Some(open_group.clone());
            item.checked = Some(tab.id) == shown;
            (id, item, Row::Show(tab.id))
        })
        .collect();

    let duplicates = tabs.duplicates();
    let copies_group: SharedString = t!("tabs.group.duplicates", count = duplicates.len()).into();
    for group in duplicates {
        let Some(tab) = tabs.get(group[0]) else {
            continue;
        };
        let id: SharedString = format!("copies:{}", tab.id.0).into();
        let label = t!("tabs.close_copies", name = tab.title(), count = group.len());
        let mut item = CommandItem::new(id.clone(), label);
        item.group = Some(copies_group.clone());
        rows.push((id, item, Row::CloseCopies(tab.id)));
    }

    let closed: Vec<_> = tabs.closed().collect();
    let closed_group: SharedString = t!("tabs.group.closed", count = closed.len()).into();
    for (index, tab) in closed.into_iter().enumerate() {
        let id: SharedString = format!("closed:{index}").into();
        let mut item = CommandItem::new(id.clone(), tab.title());
        item.keywords = vec![tab.path.to_string_lossy().into_owned().into()];
        item.group = Some(closed_group.clone());
        rows.push((id, item, Row::Reopen(tab.path.clone())));
    }
    rows
}

/// The recent files no tab has open (R.10), pinned first.
fn recent_rows(
    tabs: &Tabs,
    files: &[excellent_store::recent::RecentFile],
) -> Vec<(SharedString, CommandItem, Row)> {
    let files: Vec<_> = files
        .iter()
        .filter(|file| tabs.find(&file.path).is_none())
        .collect();
    let group: SharedString = t!("tabs.group.recent", count = files.len()).into();
    files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            let id: SharedString = format!("recent:{index}").into();
            let name = file.path.file_name().map_or_else(
                || file.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            let mut item = CommandItem::new(id.clone(), name);
            item.keywords = vec![file.path.to_string_lossy().into_owned().into()];
            item.group = Some(group.clone());
            (id, item, Row::OpenRecent(file.path.clone(), file.page))
        })
        .collect()
}
