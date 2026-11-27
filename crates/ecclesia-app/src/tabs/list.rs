//! The tab list (R.12 steps 3 and 4; reference `dev notes/images/ui-reference-vivaldi-tab-list.png`): every tab in
//! one searchable list, in counted sections: the open tabs, the files open in more than one tab (with "close the
//! copies"), the tabs closed lately, each other workspace's tabs (W.9), and the recent files not open (R.10). Typing
//! searches the names, the folders and the workspaces' names; Return shows, reopens or closes; Delete closes the
//! highlighted open tab. A dialog, as the palette is: the page canvas is a window over the app's, so a dropdown under
//! the title bar would be drawn behind it.
//!
//! shortcut: the "inactive" section of the plan is every open tab but the shown one (only the shown tab has a
//! document open), so it is not listed apart.

use std::path::{Path, PathBuf};

use ecclesia_store::workspace::WorkspaceId;
use ecclesia_store::workspaces::Workspace;
use nsh_base::{CommandItem, button, command, dialog, theme};
use nsh_core::prelude::*;

use super::model::Tabs;
use super::{TabId, TabState};
use crate::list_dialog::{DialogPlacement, dialog_backdrop, style};
use crate::recent::RecentState;
use crate::workspaces::{self, Workspaces};

/// Default compact width of the top-right tab-list dropdown.
const DEFAULT_WIDTH: f32 = 320.0;
/// Minimum user-resized width of the top-right tab-list dropdown.
const MIN_WIDTH: f32 = 220.0;
/// Maximum user-resized width of the top-right tab-list dropdown.
const MAX_WIDTH: f32 = 540.0;
/// Width delta applied by the quick shrink/widen header controls.
const WIDTH_STEP: f32 = 40.0;
/// Width of the left-edge interactive drag handle.
const HANDLE_WIDTH: f32 = 10.0;

/// What a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Show(TabId),
    CloseCopies(TabId),
    Reopen(PathBuf),
    /// A recent file, at its page.
    OpenRecent(PathBuf, usize),
    /// Another workspace's tab (W.9).
    Elsewhere(WorkspaceId, PathBuf),
}

/// One of the list's rows: its item, keyed by its id, and what it does.
type ListRow = (SharedString, CommandItem, Row);

/// The tab list, while `open`.
pub(crate) fn tab_list(
    open: Signal<bool>,
    state: TabState,
    recent: RecentState,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || open.set(false);
    recent.load();
    // Read once as the list opens: the other workspaces don't change while it is up.
    let section = move |workspace: &Workspace, name: &str, count: usize| -> SharedString {
        let key = if workspaces.in_other_window(workspace.id) {
            "tabs.group.workspace_window"
        } else {
            "tabs.group.workspace"
        };
        t!(key, name = name, count = count).into()
    };
    let elsewhere = stored(elsewhere_rows(&workspaces.tabs_elsewhere(), section));
    let current_section = move |name: &str, count: usize, hibernated: usize| -> SharedString {
        if hibernated > 0 {
            t!(
                "tabs.group.workspace_current_hibernated",
                name = name,
                count = count,
                hibernated = hibernated
            )
        } else {
            t!("tabs.group.workspace_current", name = name, count = count)
        }
        .into()
    };
    let rows = move || {
        let workspace_name = workspaces
            .shown()
            .as_ref()
            .map_or_else(|| t!("workspace.default_name"), workspaces::display_name);
        let mut rows = state
            .tabs
            .with(|tabs| open_rows(tabs, &workspace_name, current_section));
        rows.extend(elsewhere.with(|rows| rows.clone()));
        rows.extend(state.tabs.with(maintenance_rows));
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
    let in_fields = KeyOptions {
        when: nsh_core::keymap::When::Always,
        ..KeyOptions::default()
    };
    on_key_with("escape", in_fields, move |_| close());
    let width = signal(DEFAULT_WIDTH);
    let drag_start = signal(None::<(f32, f32)>);
    let panel = style(
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
                    Row::Elsewhere(id, path) => {
                        close();
                        workspaces.show_tab(id, path);
                    }
                }
            })
            .on_cancel(close),
    )
    .w(move || px((width.get() - HANDLE_WIDTH - 2.0).max(MIN_WIDTH - HANDLE_WIDTH - 2.0)))
    .rounded(px(0.0))
    .border(px(0.0));

    let resize_handle = view()
        .w(px(HANDLE_WIDTH))
        .flex_shrink(0.0)
        .items_center()
        .justify_center()
        .cursor(Cursor::ColResize)
        .hover(|s| s.bg(move || theme().accent))
        .style_when(
            move || drag_start.with(Option::is_some),
            |s| s.bg(move || theme().accent),
        )
        .on_press_down(move |event: &PressEvent| {
            drag_start.set(Some((event.page_position.x.0, width.get_untracked())));
        })
        .on_press_up(move |_: &PressEvent| drag_start.set(None))
        .on_mouse_up(move |_: &MouseEvent| drag_start.set(None))
        .child(
            view()
                .w(px(2.0))
                .h(px(32.0))
                .rounded_full()
                .bg(move || theme().border),
        );

    let header = view()
        .h(px(30.0))
        .px_2p5()
        .flex_row()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(t!("tabs.list"))
                .flex_1()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(width_step_button(
            "−",
            t!("tabs.list.shrink"),
            move || width.get() <= MIN_WIDTH,
            move || width.update(|w| *w = (*w - WIDTH_STEP).clamp(MIN_WIDTH, MAX_WIDTH)),
        ))
        .child(width_step_button(
            "+",
            t!("tabs.list.widen"),
            move || width.get() >= MAX_WIDTH,
            move || width.update(|w| *w = (*w + WIDTH_STEP).clamp(MIN_WIDTH, MAX_WIDTH)),
        ));

    let card = view()
        .w(move || px(width.get()))
        .flex_row()
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg()
        .child(resize_handle)
        .child(view().flex_1().min_w(px(0.0)).child(header).child(panel));

    let placement = DialogPlacement::TopRight {
        top: px(42.0),
        right: px(136.0),
    };
    dialog()
        .label(t!("tabs.list"))
        .animation(ModalAnimation::Fade)
        .on_cancel(close)
        .child(
            view()
                .flex_1()
                .relative()
                .on_mouse_move(move |event: &MouseEvent| {
                    let Some((start_x, start_w)) = drag_start.get_untracked() else {
                        return;
                    };
                    if event.pressed_button.is_none() {
                        drag_start.set(None);
                        return;
                    }
                    let next =
                        (start_w + (start_x - event.page_position.x.0)).clamp(MIN_WIDTH, MAX_WIDTH);
                    width.set(next);
                })
                .on_mouse_up(move |_: &MouseEvent| {
                    if drag_start.get_untracked().is_some() {
                        drag_start.set(None);
                    }
                })
                .on_mouse_leave(move |_: &MouseEvent| {
                    if drag_start.get_untracked().is_some() {
                        drag_start.set(None);
                    }
                })
                .child(dialog_backdrop(move || {
                    if drag_start.get_untracked().is_none() {
                        close();
                    }
                }))
                .child(placement.wrap(card)),
        )
}

/// Compact one-click width step button (`−` / `+`) in the tab-list dropdown header.
fn width_step_button(
    glyph: &'static str,
    label: String,
    disabled: impl Fn() -> bool + Copy + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(label)
        .disabled(disabled)
        .on_press(press)
        .w(px(22.0))
        .h(px(20.0))
        .rounded(px(4.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.7))
        .disabled_style(|s| s.opacity(0.4))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            text(glyph)
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
}

/// The active workspace's open tabs, labelled with its name and hibernated background count.
fn open_rows(
    tabs: &Tabs,
    workspace_name: &str,
    section: impl Fn(&str, usize, usize) -> SharedString,
) -> Vec<ListRow> {
    let shown = tabs.active().map(|tab| tab.id);
    let total = tabs.all().len();
    let hibernated = total.saturating_sub(usize::from(shown.is_some()));
    let open_group = section(workspace_name, total, hibernated);
    tabs.all()
        .iter()
        .map(|tab| {
            let id: SharedString = format!("open:{}", tab.id.0).into();
            let mut item = CommandItem::new(id.clone(), tab.title());
            item.keywords = vec![
                tab.path.to_string_lossy().into_owned().into(),
                workspace_name.to_owned().into(),
            ];
            item.group = Some(open_group.clone());
            item.checked = Some(tab.id) == shown;
            (id, item, Row::Show(tab.id))
        })
        .collect()
}

/// Duplicate and recently closed tab sections for the shown workspace.
fn maintenance_rows(tabs: &Tabs) -> Vec<ListRow> {
    let mut rows = Vec::new();
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

/// A section per other workspace with tabs (W.9), headed by `section` (given the workspace, its name and its tab
/// count). A row is found by its file's name, its folder or its workspace's name.
fn elsewhere_rows(
    elsewhere: &[(Workspace, Vec<PathBuf>)],
    section: impl Fn(&Workspace, &str, usize) -> SharedString,
) -> Vec<ListRow> {
    let mut rows = Vec::new();
    for (workspace, paths) in elsewhere {
        let name = workspaces::display_name(workspace);
        let group = section(workspace, &name, paths.len());
        for path in paths {
            let id: SharedString = format!("elsewhere:{}", rows.len()).into();
            let mut item = CommandItem::new(id.clone(), file_name(path));
            item.keywords = vec![
                path.to_string_lossy().into_owned().into(),
                name.clone().into(),
            ];
            item.group = Some(group.clone());
            rows.push((id, item, Row::Elsewhere(workspace.id, path.clone())));
        }
    }
    rows
}

/// A file's name, or its whole path when it has none.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The recent files no tab has open (R.10), pinned first.
fn recent_rows(tabs: &Tabs, files: &[ecclesia_store::recent::RecentFile]) -> Vec<ListRow> {
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
            let mut item = CommandItem::new(id.clone(), file_name(&file.path));
            item.keywords = vec![file.path.to_string_lossy().into_owned().into()];
            item.group = Some(group.clone());
            (id, item, Row::OpenRecent(file.path.clone(), file.page))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: WorkspaceId, name: &str) -> Workspace {
        Workspace {
            id,
            name: name.to_owned(),
            icon: None,
            color: None,
            tabs: 0,
        }
    }

    #[test]
    fn other_workspaces_tabs_are_listed_by_workspace() {
        let tax = WorkspaceId::DEFAULT;
        let elsewhere = vec![(
            workspace(tax, "Tax"),
            vec![
                PathBuf::from("C:/Tax/2025.pdf"),
                PathBuf::from("C:/Tax/2024.pdf"),
            ],
        )];
        let section = |_: &Workspace, name: &str, count: usize| {
            SharedString::from(format!("{name} ({count})"))
        };
        let rows = elsewhere_rows(&elsewhere, section);
        assert_eq!(rows.len(), 2);
        let (_, item, row) = &rows[0];
        assert_eq!(item.label.as_ref(), "2025.pdf");
        assert!(
            item.keywords
                .iter()
                .any(|keyword| keyword.as_ref() == "Tax"),
            "found by its workspace's name"
        );
        assert_eq!(*row, Row::Elsewhere(tax, PathBuf::from("C:/Tax/2025.pdf")));
        assert_ne!(rows[0].0, rows[1].0, "each row its own id");
        assert_eq!(
            rows[0].1.group.as_deref(),
            Some("Tax (2)"),
            "one counted section per workspace"
        );
        assert_eq!(rows[0].1.group, rows[1].1.group);
    }

    #[test]
    fn current_workspace_rows_include_name_and_hibernated_count() {
        let mut tabs = Tabs::default();
        for name in ["a.pdf", "b.pdf", "c.pdf"] {
            tabs.open(PathBuf::from(name), 0);
        }
        let section = |name: &str, count: usize, hibernated: usize| -> SharedString {
            format!("{name} ({count} · {hibernated} hibernated)").into()
        };
        let rows = open_rows(&tabs, "Research", section);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].1.group.as_deref(),
            Some("Research (3 · 2 hibernated)")
        );
        assert!(
            rows[0]
                .1
                .keywords
                .iter()
                .any(|keyword| keyword.as_ref() == "Research")
        );
    }
}
