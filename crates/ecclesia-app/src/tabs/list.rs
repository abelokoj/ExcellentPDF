//! The Vivaldi-style top-right tab-list dropdown (R.12 steps 3 and 4): a searchable, resizable card
//! with a `"Search all tabs"` bar + `…` quick-actions menu at the top, followed by four counted,
//! collapsible section cards — **Open Tabs**, **Inactive Tabs**, **Duplicate Tabs**, and
//! **Recently Closed Tabs** — each row showing a document icon, truncated title, workspace pill
//! badge, and a hover-revealed red-hover close (`×`) button.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine as _;
use ecclesia_store::workspace::WorkspaceId;
use ecclesia_store::workspaces::Workspace;
use nsh_base::{MenuItemElement, button, dialog, input, menu, menu_item, menu_separator, theme};
use nsh_core::prelude::*;

use super::model::Tabs;
use super::{TabId, TabState};
use crate::list_dialog::{DialogPlacement, dialog_backdrop};
use crate::recent::RecentState;
use crate::shell::app_menu::{ChromeIcon, chrome_icon, close_x_button};
use crate::workspaces::{self, Workspaces};

/// Default width of the top-right Vivaldi tab-list dropdown.
const DEFAULT_WIDTH: f32 = 384.0;
/// Minimum user-resized width of the tab-list dropdown.
const MIN_WIDTH: f32 = 280.0;
/// Maximum user-resized width of the tab-list dropdown.
const MAX_WIDTH: f32 = 580.0;
/// Width delta applied by the overflow menu's Narrower / Wider actions.
const WIDTH_STEP: f32 = 40.0;
/// Width of the left-edge interactive drag handle.
const HANDLE_WIDTH: f32 = 8.0;

const TAB_OPEN_SVG: &[u8] = include_bytes!("../../assets/icons/tab-open.svg");
const TAB_INACTIVE_SVG: &[u8] = include_bytes!("../../assets/icons/tab-inactive.svg");
const TAB_DUPLICATE_SVG: &[u8] = include_bytes!("../../assets/icons/tab-duplicate.svg");
const DOTS_THREE_SVG: &[u8] = include_bytes!("../../assets/icons/dots-three.svg");
const TRIANGLE_RIGHT_SVG: &[u8] = include_bytes!("../../assets/icons/triangle-right.svg");
const TRIANGLE_DOWN_SVG: &[u8] = include_bytes!("../../assets/icons/triangle-down.svg");

fn svg_uri(bytes: &[u8]) -> String {
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn tab_open_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(TAB_OPEN_SVG)).clone()
}

fn tab_inactive_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(TAB_INACTIVE_SVG)).clone()
}

fn tab_duplicate_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(TAB_DUPLICATE_SVG)).clone()
}

fn dots_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(DOTS_THREE_SVG)).clone()
}

fn disclosure_right_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(TRIANGLE_RIGHT_SVG)).clone()
}

fn disclosure_down_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_uri(TRIANGLE_DOWN_SVG)).clone()
}

/// Which of the four Vivaldi tab-list section cards a header belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Open = 0,
    Inactive = 1,
    Duplicates = 2,
    Closed = 3,
}

/// One displayed tab entry inside any of the four collapsible sections.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TabEntry {
    key: String,
    title: String,
    search_blob: String,
    workspace_badge: String,
    active: bool,
    action: RowAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RowAction {
    Show(TabId),
    Elsewhere(WorkspaceId, PathBuf),
    CloseCopies(TabId),
    Reopen(PathBuf),
}

impl TabEntry {
    fn matches(&self, needle: &str) -> bool {
        needle.is_empty() || self.search_blob.contains(needle)
    }
}

/// Shared reactive handles passed into each collapsible section card.
#[derive(Clone, Copy)]
struct TabListCx {
    expanded: Signal<[bool; 4]>,
    query: Signal<String>,
    state: TabState,
    workspaces: Workspaces,
}

/// The Vivaldi-style top-right tab-list dropdown, while `open`.
pub(crate) fn tab_list(
    open: Signal<bool>,
    state: TabState,
    recent: RecentState,
    workspaces: Workspaces,
) -> impl IntoElement {
    let close = move || open.set(false);
    recent.load();
    // Snapshot other workspaces' open tabs once when the dropdown opens.
    let elsewhere = stored(workspaces.tabs_elsewhere());
    let query = signal(String::new());
    // Open Tabs starts expanded; clicking any section card header toggles it cleanly.
    let expanded = signal([true, false, false, false]);
    let width = signal(DEFAULT_WIDTH);
    let drag_start = signal(None::<(f32, f32)>);

    let in_fields = KeyOptions {
        when: nsh_core::keymap::When::Always,
        ..KeyOptions::default()
    };
    on_key_with("escape", in_fields, move |_| {
        if query.with_untracked(|q| !q.is_empty()) {
            query.set(String::new());
        } else {
            close();
        }
    });

    let current_ws_name = move || {
        workspaces
            .shown()
            .as_ref()
            .map_or_else(|| t!("workspace.default_name"), workspaces::display_name)
    };

    let open_entries = move || {
        let ws_name = current_ws_name();
        let needle = query.with(|q| q.trim().to_ascii_lowercase());
        let mut entries = state.tabs.with(|tabs| collect_open_entries(tabs, &ws_name));
        elsewhere.with(|other| {
            entries.extend(collect_elsewhere_entries(other));
        });
        entries.retain(|e| e.matches(&needle));
        entries
    };

    let inactive_entries = move || {
        let ws_name = current_ws_name();
        let needle = query.with(|q| q.trim().to_ascii_lowercase());
        let mut entries = state
            .tabs
            .with(|tabs| collect_inactive_entries(tabs, &ws_name));
        elsewhere.with(|other| {
            entries.extend(collect_elsewhere_entries(other));
        });
        entries.retain(|e| e.matches(&needle));
        entries
    };

    let duplicate_entries = move || {
        let ws_name = current_ws_name();
        let needle = query.with(|q| q.trim().to_ascii_lowercase());
        let mut entries = state.tabs.with(|tabs| {
            collect_duplicate_entries(tabs, &ws_name, |name, count| {
                t!("tabs.close_copies", name = name, count = count)
            })
        });
        entries.retain(|e| e.matches(&needle));
        entries
    };

    let closed_entries = move || {
        let ws_name = current_ws_name();
        let needle = query.with(|q| q.trim().to_ascii_lowercase());
        let mut entries = state
            .tabs
            .with(|tabs| collect_closed_entries(tabs, &ws_name));
        entries.retain(|e| e.matches(&needle));
        entries
    };

    let has_duplicates = move || state.tabs.with(|tabs| !tabs.duplicates().is_empty());

    let overflow_menu = menu()
        .align(Align::End)
        .offset(px(4.0))
        .popup_class("w-52 p-1 rounded-xl border border-border bg-popover shadow-lg")
        .trigger(
            view()
                .label(t!("tabs.list.more"))
                .w(px(30.0))
                .h(px(30.0))
                .rounded(px(6.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().accent))
                .child(
                    image(dots_icon_src())
                        .w(px(18.0))
                        .h(px(18.0))
                        .object_fit(ResizeMode::Contain)
                        .tint_color(move || theme().popover_foreground),
                ),
        )
        .child(overflow_item(t!("tabs.list.expand_all"), move || {
            expanded.set([true, true, true, true]);
        }))
        .child(overflow_item(t!("tabs.list.collapse_all"), move || {
            expanded.set([false, false, false, false]);
        }))
        .child(
            menu_separator()
                .h(px(1.0))
                .my(px(4.0))
                .bg(move || theme().border),
        )
        .child(
            overflow_item(t!("tabs.list.close_all_duplicates"), move || {
                state.run(super::Action::CloseDuplicates);
            })
            .disabled(move || !has_duplicates()),
        )
        .child(
            menu_separator()
                .h(px(1.0))
                .my(px(4.0))
                .bg(move || theme().border),
        )
        .child(
            overflow_item(t!("tabs.list.shrink"), move || {
                width.update(|w| *w = (*w - WIDTH_STEP).clamp(MIN_WIDTH, MAX_WIDTH));
            })
            .disabled(move || width.get() <= MIN_WIDTH),
        )
        .child(
            overflow_item(t!("tabs.list.widen"), move || {
                width.update(|w| *w = (*w + WIDTH_STEP).clamp(MIN_WIDTH, MAX_WIDTH));
            })
            .disabled(move || width.get() >= MAX_WIDTH),
        );

    let search_bar = view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .pt(px(12.0))
        .pb(px(10.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h(px(36.0))
                .px(px(10.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .rounded(px(8.0))
                .bg(move || theme().background)
                .border_1()
                .border_color(move || theme().border)
                .child(chrome_icon(ChromeIcon::RailSearch, 16.0, move || {
                    theme().foreground
                }))
                .child(
                    input()
                        .flex_1()
                        .min_w(px(0.0))
                        .value(move || query.get())
                        .placeholder(t!("tabs.list.search_placeholder"))
                        .label(t!("tabs.list.search_placeholder"))
                        .auto_focus(true)
                        .on_change(move |next: &str| query.set(next.to_owned()))
                        .text_size(px(13.5))
                        .text_color(move || theme().foreground),
                )
                .child(when(
                    move || query.with(|q| !q.is_empty()),
                    move || {
                        close_x_button(t!("settings.search_clear"), 20.0, 10.0, 10.0, move || {
                            query.set(String::new());
                        })
                    },
                )),
        )
        .child(overflow_menu);

    let cx = TabListCx {
        expanded,
        query,
        state,
        workspaces,
    };

    let sections = scroll().max_h(px(540.0)).child(
        view()
            .px(px(12.0))
            .pb(px(12.0))
            .gap(px(6.0))
            .child(section_card(
                Section::Open,
                t!("tabs.section.open"),
                tab_open_icon_src(),
                cx,
                open_entries,
                close,
            ))
            .child(section_card(
                Section::Inactive,
                t!("tabs.section.inactive"),
                tab_inactive_icon_src(),
                cx,
                inactive_entries,
                close,
            ))
            .child(section_card(
                Section::Duplicates,
                t!("tabs.section.duplicates"),
                tab_duplicate_icon_src(),
                cx,
                duplicate_entries,
                close,
            ))
            .child(section_card(
                Section::Closed,
                t!("tabs.section.closed"),
                String::new(),
                cx,
                closed_entries,
                close,
            )),
    );

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
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(search_bar)
                .child(sections),
        );

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

fn section_card(
    section: Section,
    title: String,
    icon_src: String,
    cx: TabListCx,
    entries: impl Fn() -> Vec<TabEntry> + Copy + 'static,
    close: impl Fn() + Copy + 'static,
) -> View {
    let TabListCx {
        expanded,
        query,
        state,
        workspaces,
    } = cx;
    let idx = section as usize;
    let is_open = move || {
        let filtering = query.with(|q| !q.trim().is_empty());
        if filtering {
            !entries().is_empty()
        } else {
            expanded.with(|arr| arr[idx])
        }
    };
    let count = move || entries().len();

    let header_fg = move || {
        if is_open() {
            theme().primary_foreground
        } else {
            theme().popover_foreground
        }
    };

    let section_icon = dynamic(move || {
        if section == Section::Closed {
            chrome_icon(ChromeIcon::Trash, 16.0, header_fg).into_element()
        } else {
            image(icon_src.clone())
                .w(px(16.0))
                .h(px(16.0))
                .object_fit(ResizeMode::Contain)
                .tint_color(header_fg)
                .into_element()
        }
    });

    let disclosure = dynamic(move || {
        let src = if is_open() {
            disclosure_down_src()
        } else {
            disclosure_right_src()
        };
        let (w, h) = if is_open() { (9.0, 7.0) } else { (7.0, 9.0) };
        image(src)
            .w(px(w))
            .h(px(h))
            .object_fit(ResizeMode::Contain)
            .tint_color(header_fg)
            .into_element()
    });

    let count_badge = view()
        .h(px(22.0))
        .min_w(px(24.0))
        .px(px(7.0))
        .rounded(px(5.0))
        .items_center()
        .justify_center()
        .bg(move || {
            if is_open() {
                Hsla::parse_hex("#ffffff38").expect("valid hex").into()
            } else {
                theme().muted
            }
        })
        .child(
            text(move || count().to_string())
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .tabular_nums()
                .text_color(move || {
                    if is_open() {
                        theme().primary_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        );

    let header = button()
        .accessibility_label(title.clone())
        .on_press(move || {
            expanded.update(|arr| arr[idx] = !arr[idx]);
        })
        .w_full()
        .h(px(38.0))
        .px(px(10.0))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .bg(move || {
            if is_open() {
                theme().primary
            } else {
                theme().background
            }
        })
        .hover(|s| s.opacity(0.94))
        .child(
            view()
                .w(px(12.0))
                .items_center()
                .justify_center()
                .child(disclosure),
        )
        .child(section_icon)
        .child(
            text(title)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(header_fg),
        )
        .child(count_badge);

    let body = when(is_open, move || {
        let rows = each(
            entries,
            |entry: &TabEntry| entry.key.clone(),
            move |entry, _| tab_entry_row(entry, state, workspaces, close),
        );
        scroll()
            .max_h(px(360.0))
            .child(view().py(px(4.0)).child(rows).child(when(
                move || entries().is_empty(),
                move || {
                    view().px(px(12.0)).py(px(10.0)).child(
                        text(move || t!("tabs.list.empty_section"))
                            .text_size(px(12.0))
                            .text_color(move || theme().muted_foreground),
                    )
                },
            )))
    });

    view()
        .rounded(px(8.0))
        .overflow_hidden()
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(header)
        .child(body)
}

fn tab_entry_row(
    entry: ReadSignal<TabEntry>,
    state: TabState,
    workspaces: Workspaces,
    close: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let hovered = signal(false);
    let title = move || entry.with(|e| e.title.clone());
    let badge = move || entry.with(|e| e.workspace_badge.clone());
    let active = move || entry.with(|e| e.active);
    let closable_tab = move || {
        entry.with(|e| match e.action {
            RowAction::Show(id) => Some((id, false)),
            RowAction::CloseCopies(id) => Some((id, true)),
            RowAction::Elsewhere(_, _) | RowAction::Reopen(_) => None,
        })
    };

    let main_btn = button()
        .accessibility_label(move || Some(SharedString::from(title())))
        .on_press(move || {
            let action = entry.with_untracked(|e| e.action.clone());
            match action {
                RowAction::Show(id) => {
                    close();
                    state.show(id);
                }
                RowAction::Elsewhere(ws_id, path) => {
                    close();
                    workspaces.show_tab(ws_id, path);
                }
                RowAction::CloseCopies(id) => {
                    state.show(id);
                    state.run(super::Action::CloseDuplicates);
                }
                RowAction::Reopen(path) => {
                    close();
                    state.reopen(&path);
                }
            }
        })
        .flex_1()
        .min_w(px(0.0))
        .h(px(34.0))
        .pl(px(10.0))
        .pr(px(4.0))
        .flex_row()
        .items_center()
        .gap(px(9.0))
        .child(chrome_icon(ChromeIcon::RailPages, 15.0, move || {
            if active() {
                theme().primary
            } else {
                theme().muted_foreground
            }
        }))
        .child(
            text(title)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(13.0))
                .font_weight(if active() {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .h(px(22.0))
                .max_w(px(104.0))
                .px(px(9.0))
                .flex_shrink(0.0)
                .rounded(px(5.0))
                .items_center()
                .justify_center()
                .bg(move || theme().muted)
                .child(
                    text(badge)
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(move || theme().muted_foreground),
                ),
        );

    view()
        .flex_row()
        .items_center()
        .pr(px(6.0))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .hover(|s| s.bg(move || theme().accent))
        .child(main_btn)
        .child(when(
            move || hovered.get() && closable_tab().is_some(),
            move || {
                close_x_button(t!("tabs.list.close_tab"), 22.0, 10.0, 5.0, move || {
                    if let Some((id, is_dup)) = closable_tab() {
                        if is_dup {
                            state.show(id);
                            state.run(super::Action::CloseDuplicates);
                        } else {
                            state.close(id);
                        }
                    }
                })
            },
        ))
}

fn overflow_item(label: String, select: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(label.clone())
        .flex_row()
        .items_center()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

fn collect_open_entries(tabs: &Tabs, workspace_name: &str) -> Vec<TabEntry> {
    let shown = tabs.active().map(|t| t.id);
    tabs.all()
        .iter()
        .map(|tab| {
            let title = tab.title();
            TabEntry {
                key: format!("open:{}", tab.id.0),
                search_blob: format!(
                    "{} {} {}",
                    title.to_ascii_lowercase(),
                    tab.path.to_string_lossy().to_ascii_lowercase(),
                    workspace_name.to_ascii_lowercase()
                ),
                title,
                workspace_badge: workspace_name.to_owned(),
                active: Some(tab.id) == shown,
                action: RowAction::Show(tab.id),
            }
        })
        .collect()
}

fn collect_inactive_entries(tabs: &Tabs, workspace_name: &str) -> Vec<TabEntry> {
    let shown = tabs.active().map(|t| t.id);
    tabs.all()
        .iter()
        .filter(|tab| Some(tab.id) != shown)
        .map(|tab| {
            let title = tab.title();
            TabEntry {
                key: format!("inactive:{}", tab.id.0),
                search_blob: format!(
                    "{} {} {}",
                    title.to_ascii_lowercase(),
                    tab.path.to_string_lossy().to_ascii_lowercase(),
                    workspace_name.to_ascii_lowercase()
                ),
                title,
                workspace_badge: workspace_name.to_owned(),
                active: false,
                action: RowAction::Show(tab.id),
            }
        })
        .collect()
}

fn collect_elsewhere_entries(elsewhere: &[(Workspace, Vec<PathBuf>)]) -> Vec<TabEntry> {
    let mut entries = Vec::new();
    for (workspace, paths) in elsewhere {
        let name = workspaces::display_name(workspace);
        for path in paths {
            let title = file_name(path);
            entries.push(TabEntry {
                key: format!("elsewhere:{}:{}", workspace.id.to_key(), entries.len()),
                search_blob: format!(
                    "{} {} {}",
                    title.to_ascii_lowercase(),
                    path.to_string_lossy().to_ascii_lowercase(),
                    name.to_ascii_lowercase()
                ),
                title,
                workspace_badge: name.clone(),
                active: false,
                action: RowAction::Elsewhere(workspace.id, path.clone()),
            });
        }
    }
    entries
}

fn collect_duplicate_entries(
    tabs: &Tabs,
    workspace_name: &str,
    label: impl Fn(&str, usize) -> String,
) -> Vec<TabEntry> {
    tabs.duplicates()
        .into_iter()
        .filter_map(|group| {
            let tab = tabs.get(group[0])?;
            let title = label(&tab.title(), group.len());
            Some(TabEntry {
                key: format!("dup:{}", tab.id.0),
                search_blob: format!(
                    "{} {} {}",
                    title.to_ascii_lowercase(),
                    tab.path.to_string_lossy().to_ascii_lowercase(),
                    workspace_name.to_ascii_lowercase()
                ),
                title,
                workspace_badge: workspace_name.to_owned(),
                active: false,
                action: RowAction::CloseCopies(tab.id),
            })
        })
        .collect()
}

fn collect_closed_entries(tabs: &Tabs, workspace_name: &str) -> Vec<TabEntry> {
    tabs.closed()
        .enumerate()
        .map(|(index, tab)| {
            let title = tab.title();
            TabEntry {
                key: format!("closed:{index}"),
                search_blob: format!(
                    "{} {} {}",
                    title.to_ascii_lowercase(),
                    tab.path.to_string_lossy().to_ascii_lowercase(),
                    workspace_name.to_ascii_lowercase()
                ),
                title,
                workspace_badge: workspace_name.to_owned(),
                active: false,
                action: RowAction::Reopen(tab.path.clone()),
            }
        })
        .collect()
}

/// Stable string helper for `WorkspaceId` keys without accessing private fields.
trait WorkspaceKey {
    fn to_key(self) -> u64;
}

impl WorkspaceKey for WorkspaceId {
    fn to_key(self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// A file's name, or its whole path when it has none.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
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
        let rows = collect_elsewhere_entries(&elsewhere);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "2025.pdf");
        assert_eq!(rows[0].workspace_badge, "Tax");
        assert!(rows[0].matches("tax"));
        assert!(rows[0].matches("2025"));
        assert_eq!(
            rows[0].action,
            RowAction::Elsewhere(tax, PathBuf::from("C:/Tax/2025.pdf"))
        );
    }

    #[test]
    fn inactive_and_duplicate_sections_partition_open_tabs_cleanly() {
        let mut tabs = Tabs::default();
        for name in ["a.pdf", "b.pdf", "a.pdf"] {
            tabs.open(PathBuf::from(name), 0);
        }
        let open = collect_open_entries(&tabs, "FSU");
        let inactive = collect_inactive_entries(&tabs, "FSU");
        let duplicates =
            collect_duplicate_entries(&tabs, "FSU", |name, count| format!("{name} ({count})"));

        assert_eq!(open.len(), 3);
        assert_eq!(inactive.len(), 2);
        assert_eq!(duplicates.len(), 1);
        assert_eq!(open[0].workspace_badge, "FSU");
        assert!(open.iter().any(|e| e.active));
        assert!(inactive.iter().all(|e| !e.active));
    }
}
