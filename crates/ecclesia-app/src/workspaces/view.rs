//! The Vivaldi-style workspace switcher (W.1): a button at the left of the title bar with the shown workspace's
//! icon and name, and the workspace dropdown & dialogs:
//! - The switcher dropdown (`Dialog::Switch`): a header with a stacked-layers icon, `"Workspaces"` title, and
//!   expandable search icon; 2-line workspace rows with an automatic random vector icon (when no custom icon is
//!   set), name, smaller muted tab count underneath, right-side `=` drag handle for vertical reordering, and a
//!   right-click context menu (`Open in a New Window`, `Edit Workspace`, `Delete Workspace`); plus a `New Workspace`
//!   footer button.
//! - The send-tab / open-in-window pickers (`Dialog::Send`, `Dialog::Window`), the edit dialog (`Dialog::Edit`),
//!   and the delete confirmation (`Dialog::Delete`).

use std::sync::OnceLock;

use base64::Engine as _;
use ecclesia_store::workspace::WorkspaceId;
use ecclesia_store::workspaces::{Look, Workspace};
use nsh_base::{
    CommandItem, MenuItemElement, button, command, context_menu, input, menu_item, theme, toggle,
};
use nsh_core::EllipsizeMode;
use nsh_core::prelude::*;

use super::{Dialog, Start, Transfer, WorkspaceAction, Workspaces, display_name};
use crate::commands::{self, CommandId};
use crate::keymap;
use crate::list_dialog::{DialogPlacement, card, dismissible_dialog, list_dialog};
use crate::panes::ClosingTabs;
use crate::shell::app_menu::{ChromeIcon, chrome_icon};
use crate::tabs::{GroupColor, Step, Tone, action_button, color_name};

/// The icons offered in the edit dialog; clearing an icon falls back to an automatic vector icon.
const ICONS: [&str; 8] = ["📚", "📄", "🎓", "💼", "🧾", "🔬", "✏️", "⭐"];

/// Embedded SVG for the stacked-layers header icon.
const WS_LAYERS_SVG: &[u8] = include_bytes!("../../assets/icons/ws-layers.svg");
/// Embedded SVG for the active workspace checkmark indicator.
const WS_CHECK_SVG: &[u8] = include_bytes!("../../assets/icons/ws-check.svg");

/// Curated pool of 10 crisp Phosphor vector SVG icons automatically assigned when a workspace has no custom icon.
const WS_AUTO_SVGS: [&[u8]; 10] = [
    include_bytes!("../../assets/icons/ws-sailboat.svg"),
    include_bytes!("../../assets/icons/ws-plane.svg"),
    include_bytes!("../../assets/icons/ws-chart.svg"),
    include_bytes!("../../assets/icons/ws-book.svg"),
    include_bytes!("../../assets/icons/ws-compass.svg"),
    include_bytes!("../../assets/icons/ws-bulb.svg"),
    include_bytes!("../../assets/icons/ws-briefcase.svg"),
    include_bytes!("../../assets/icons/ws-globe.svg"),
    include_bytes!("../../assets/icons/ws-grad.svg"),
    include_bytes!("../../assets/icons/ws-rocket.svg"),
];

/// Approximate vertical stride of one workspace row in logical pixels for drag-to-reorder stepping.
const ROW_STRIDE_PX: f32 = 52.0;

/// Minimum vertical pointer movement before the `=` drag handle claims touches.
const DRAG_THRESHOLD_PX: f32 = 4.0;

/// Active vertical drag state inside the workspace dropdown.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WorkspaceDrag {
    id: WorkspaceId,
    anchor_y: f32,
}

fn svg_data_uri(bytes: &[u8]) -> String {
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn layers_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_data_uri(WS_LAYERS_SVG)).clone()
}

fn check_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| svg_data_uri(WS_CHECK_SVG)).clone()
}

/// Deterministic pseudo-random index into [`WS_AUTO_SVGS`] for a workspace with no custom icon,
/// spreading workspaces across varied icons while staying stable across restarts.
fn auto_icon_index(id: WorkspaceId) -> usize {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    auto_icon_index_from_hash(hasher.finish())
}

fn auto_icon_index_from_hash(hash: u64) -> usize {
    let mixed = hash.wrapping_mul(2_654_435_761) >> 24;
    (mixed as usize) % WS_AUTO_SVGS.len()
}

fn auto_icon_src(id: WorkspaceId) -> String {
    static CACHE: [OnceLock<String>; 10] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    let slot = auto_icon_index(id);
    CACHE[slot]
        .get_or_init(|| svg_data_uri(WS_AUTO_SVGS[slot]))
        .clone()
}

/// Renders either the user's chosen emoji/symbol icon or the workspace's automatic vector SVG icon.
fn workspace_icon_view(
    id: WorkspaceId,
    custom_icon: Option<String>,
    size: f32,
    tint: impl Fn() -> AdaptiveColor + Copy + 'static,
) -> AnyElement {
    if let Some(icon) = custom_icon.filter(|s| !s.trim().is_empty()) {
        text(icon)
            .text_size(px(size))
            .text_color(tint)
            .into_element()
    } else {
        image(auto_icon_src(id))
            .w(px(size))
            .h(px(size))
            .object_fit(ResizeMode::Contain)
            .tint_color(tint)
            .into_element()
    }
}

/// The switcher button: the shown workspace's icon and name. Pressing it opens the workspace dropdown.
pub(crate) fn switcher_button(
    workspaces: Workspaces,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let id = CommandId::Workspace(WorkspaceAction::Switcher);
    let name = move || {
        workspaces
            .shown()
            .map(|w| display_name(&w))
            .unwrap_or_default()
    };
    view().window_drag(false).flex_shrink(0.0).child(
        button()
            .accessibility_label(move || {
                Some(SharedString::from(t!(
                    "workspace.switcher",
                    name = name(),
                    command = commands::label_with_shortcut(id)
                )))
            })
            .on_press(move || run(id))
            .h(move || px((workspaces.live.read(crate::tabs::tab_font_size_px) + 14.0).max(26.0)))
            .max_w(px(240.0))
            .overflow_hidden()
            .px_2()
            .gap(px(6.0))
            .flex_row()
            .items_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.7))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(view().flex_shrink(0.0).child(dynamic(move || {
                let shown = workspaces.shown();
                let ws_id = shown.as_ref().map_or(WorkspaceId::DEFAULT, |w| w.id);
                let icon = shown.and_then(|w| w.icon);
                workspace_icon_view(ws_id, icon, 15.0, move || theme().sidebar_foreground)
            })))
            .child(
                text(name)
                    .flex_shrink(1.0)
                    .min_w(px(0.0))
                    .number_of_lines(1u32)
                    .text_overflow(EllipsizeMode::Middle)
                    .text_size(
                        move || px(workspaces.live.read(crate::tabs::tab_font_size_px) + 0.5),
                    )
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().sidebar_foreground),
            )
            .child(
                view()
                    .flex_shrink(0.0)
                    .child(crate::shell::app_menu::workspace_dropdown_arrow()),
            ),
    )
}

/// A workspace's colour as a small dot; the muted colour when it has none.
pub(crate) fn color_dot(color: impl Fn() -> Option<String> + 'static) -> impl IntoElement {
    view()
        .size(px(8.0))
        .flex_shrink(0.0)
        .rounded_full()
        .bg(move || {
            color().map_or(theme().muted_foreground, |id| {
                Hsla::parse_hex(GroupColor::parse(&id).swatch().0)
                    .expect("a built-in colour")
                    .into()
            })
        })
}

/// The workspace dialog that is open, if any.
pub(crate) fn workspace_dialogs(
    workspaces: Workspaces,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    dynamic(move || {
        let dialog = workspaces.dialog();
        untracked(|| match dialog {
            Some(Dialog::Switch) => workspace_dropdown(workspaces, run).into_element(),
            Some(list @ (Dialog::Send(_) | Dialog::Window)) => {
                workspace_command_list(workspaces, list, run).into_element()
            }
            Some(Dialog::Edit(id)) => edit_dialog(workspaces, id).into_element(),
            Some(Dialog::Delete(id)) => delete_dialog(workspaces, id).into_element(),
            Some(Dialog::Notice) => notice_dialog(workspaces).into_element(),
            None => view().into_element(),
        })
    })
}

/// Vivaldi-style workspace dropdown: header (`Workspaces` title + search toggle), scrollable 2-line
/// workspace rows with icon, name, muted tab count, right-side `=` vertical drag handle, and
/// right-click context menu (`Open in a New Window`, `Edit Workspace`, `Delete Workspace`), plus a
/// `New Workspace` button at the bottom.
fn workspace_dropdown(
    workspaces: Workspaces,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let searching = signal(false);
    let query = signal(String::new());
    let drag = signal(None::<WorkspaceDrag>);

    let header = view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .h(px(46.0))
        .px(px(14.0))
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(dynamic(move || {
            if searching.get() {
                view()
                    .flex_1()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(chrome_icon(ChromeIcon::RailSearch, 15.0, move || {
                        theme().muted_foreground
                    }))
                    .child(
                        input()
                            .flex_1()
                            .min_w(px(0.0))
                            .value(move || query.get())
                            .placeholder(t!("workspace.search"))
                            .label(t!("workspace.search"))
                            .auto_focus(true)
                            .on_change(move |text: &str| query.set(text.to_owned()))
                            .text_size(px(13.5))
                            .text_color(move || theme().popover_foreground),
                    )
                    .child(crate::shell::app_menu::close_x_button(
                        t!("workspace.search_close"),
                        24.0,
                        12.0,
                        6.0,
                        move || {
                            query.set(String::new());
                            searching.set(false);
                        },
                    ))
                    .into_element()
            } else {
                view()
                    .flex_1()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        image(layers_icon_src())
                            .w(px(18.0))
                            .h(px(18.0))
                            .object_fit(ResizeMode::Contain)
                            .tint_color(move || theme().popover_foreground),
                    )
                    .child(
                        text(move || t!("workspace.dropdown_title"))
                            .flex_1()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().popover_foreground),
                    )
                    .child(
                        button()
                            .accessibility_label(t!("workspace.search"))
                            .on_press(move || searching.set(true))
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded(px(6.0))
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(move || theme().accent))
                            .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                            .child(chrome_icon(ChromeIcon::RailSearch, 15.0, move || {
                                theme().muted_foreground
                            })),
                    )
                    .into_element()
            }
        }));

    let rows_list = each(
        move || {
            let needle = query.with(|q| q.trim().to_ascii_lowercase());
            workspaces
                .list()
                .into_iter()
                .filter(move |w| {
                    needle.is_empty() || display_name(w).to_ascii_lowercase().contains(&needle)
                })
                .collect::<Vec<_>>()
        },
        |w: &Workspace| w.id,
        move |ws, _| {
            let ws_id = ws.with_untracked(|w| w.id);
            workspace_dropdown_row(workspaces, ws, ws_id, drag, close)
        },
    );

    let list_body = scroll().max_h(px(420.0)).child(
        view()
            .px(px(8.0))
            .py(px(6.0))
            .gap(px(2.0))
            .child(rows_list)
            .child(when(
                move || {
                    let needle = query.with(|q| q.trim().to_ascii_lowercase());
                    !needle.is_empty()
                        && !workspaces
                            .list()
                            .iter()
                            .any(|w| display_name(w).to_ascii_lowercase().contains(&needle))
                },
                move || {
                    view().px(px(12.0)).py(px(14.0)).child(
                        text(move || t!("workspace.search_empty"))
                            .text_size(px(12.5))
                            .text_color(move || theme().muted_foreground),
                    )
                },
            )),
    );

    let footer = view()
        .px(px(12.0))
        .py(px(10.0))
        .border_t_1()
        .border_t_color(move || theme().border)
        .flex_row()
        .items_center()
        .child(
            button()
                .accessibility_label(t!("workspace.new_button"))
                .on_press(move || {
                    close();
                    run(CommandId::Workspace(WorkspaceAction::New(Start::Empty)));
                })
                .h(px(32.0))
                .px(px(14.0))
                .rounded(px(7.0))
                .items_center()
                .justify_center()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .hover(|s| s.bg(move || theme().accent))
                .active(|s| s.opacity(0.8))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    text(move || t!("workspace.new_button"))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().popover_foreground),
                ),
        );

    let panel = view()
        .w(px(292.0))
        .rounded(px(12.0))
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg()
        .overflow_hidden()
        .child(header)
        .child(list_body)
        .child(footer);

    dismissible_dialog(
        t!("workspace.list"),
        DialogPlacement::TopLeft {
            top: px(42.0),
            left: px(56.0),
        },
        close,
        panel,
    )
}

/// One row inside the Vivaldi workspace dropdown, wrapped in its right-click context menu.
fn workspace_dropdown_row(
    workspaces: Workspaces,
    ws: ReadSignal<Workspace>,
    ws_id: WorkspaceId,
    drag: Signal<Option<WorkspaceDrag>>,
    close: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let is_active = move || workspaces.shown_id() == ws_id;
    let is_dragging = move || drag.with(|d| d.is_some_and(|active| active.id == ws_id));
    let name = move || ws.with(display_name);
    let subtitle = move || {
        let count = workspaces.tab_count_of(ws_id);
        let tabs_text = if count == 1 {
            t!("workspace.tab_count_one")
        } else {
            t!("workspace.tab_count_many", count = count)
        };
        if workspaces.in_other_window(ws_id) {
            format!("{tabs_text} · {}", t!("workspace.other_window_suffix"))
        } else {
            tabs_text
        }
    };
    let can_delete = move || workspaces.list().len() > 1 && !workspaces.in_other_window(ws_id);

    // Right-side slot: shows an accent checkmark for the active workspace (matching Vivaldi),
    // switching to the two-line `=` drag handle on hover/drag or for inactive workspaces so every
    // workspace can be reordered vertically.
    let handle_hovered = signal(false);
    let drag_handle = view()
        .label(t!("workspace.reorder_handle"))
        .w(px(28.0))
        .h(px(34.0))
        .flex_shrink(0.0)
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .cursor(Cursor::Grab)
        .style_when(is_dragging, |s| s.cursor(Cursor::Grabbing))
        .on_mouse_enter(move |_: &MouseEvent| handle_hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| handle_hovered.set(false))
        .hover(|s| s.bg(move || theme().muted))
        .claim_touches(TouchClaim::on_move().ask(move |req: &ClaimRequest| {
            if req.event.gesture.delta.y.0.abs() >= DRAG_THRESHOLD_PX {
                req.claim();
            }
        }))
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| {
            drag.set(Some(WorkspaceDrag {
                id: ws_id,
                anchor_y: event.page.y.0,
            }));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(mut active) = drag.get_untracked() else {
                return;
            };
            if active.id != ws_id {
                return;
            }
            let dy = event.page.y.0 - active.anchor_y;
            let steps = compute_vertical_drag_steps(dy, ROW_STRIDE_PX);
            if steps != 0 {
                if workspaces.step_workspace_by(ws_id, steps) {
                    active.anchor_y += steps as f32 * ROW_STRIDE_PX;
                } else {
                    active.anchor_y = event.page.y.0;
                }
                drag.set(Some(active));
            }
        })
        .on_responder_release(move |_: &ResponderEvent| {
            if drag.get_untracked().is_some_and(|d| d.id == ws_id) {
                drag.set(None);
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| {
            if drag.get_untracked().is_some_and(|d| d.id == ws_id) {
                drag.set(None);
            }
        })
        .child(dynamic(move || {
            if is_active() && !handle_hovered.get() && !is_dragging() {
                image(check_icon_src())
                    .w(px(16.0))
                    .h(px(16.0))
                    .object_fit(ResizeMode::Contain)
                    .tint_color(move || theme().primary)
                    .into_element()
            } else {
                view()
                    .items_center()
                    .justify_center()
                    .gap(px(4.0))
                    .child(
                        view()
                            .w(px(14.0))
                            .h(px(1.5))
                            .rounded_full()
                            .bg(move || theme().muted_foreground),
                    )
                    .child(
                        view()
                            .w(px(14.0))
                            .h(px(1.5))
                            .rounded_full()
                            .bg(move || theme().muted_foreground),
                    )
                    .into_element()
            }
        }));

    // Clickable main part of the row: left icon + stacked 2-line title & tab count.
    let main_hit = button()
        .accessibility_label(move || {
            Some(SharedString::from(format!("{}, {}", name(), subtitle())))
        })
        .on_press(move || {
            close();
            workspaces.switch(ws_id);
        })
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .py(px(6.0))
        .pl(px(8.0))
        .pr(px(4.0))
        .child(
            view()
                .w(px(28.0))
                .h(px(28.0))
                .flex_shrink(0.0)
                .items_center()
                .justify_center()
                .child(dynamic(move || {
                    let custom = ws.with(|w| w.icon.clone());
                    workspace_icon_view(ws_id, custom, 20.0, move || theme().popover_foreground)
                })),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(1.0))
                .child(
                    text(name)
                        .truncate()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    text(subtitle)
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(move || theme().muted_foreground),
                ),
        );

    let row = view()
        .flex_row()
        .items_center()
        .min_h(px(48.0))
        .pr(px(6.0))
        .rounded(px(8.0))
        .bg(move || {
            if is_active() {
                theme().accent
            } else {
                theme().popover
            }
        })
        .hover(|s| s.bg(move || theme().accent))
        .style_when(is_dragging, |s| {
            s.border_1()
                .border_color(move || theme().primary)
                .opacity(0.88)
        })
        .on_file_drop(move |event: &FileDropEvent| {
            let docs = crate::open::dropped_documents(&event.uris);
            if !docs.is_empty() {
                close();
                workspaces.open_paths_in_workspace(ws_id, docs);
            }
        })
        .child(main_hit)
        .child(drag_handle);

    context_menu()
        .trigger(row)
        .popup_class("w-52 p-1 rounded-xl border border-border bg-popover shadow-lg")
        .child(
            ws_menu_item(t!("workspace.menu.open_new_window"), move || {
                close();
                workspaces.open_in_window(ws_id);
            })
            .disabled(is_active),
        )
        .child(ws_menu_item(t!("workspace.menu.edit"), move || {
            workspaces.set_dialog(Some(Dialog::Edit(ws_id)));
        }))
        .child(
            ws_menu_item(t!("workspace.menu.delete"), move || {
                workspaces.set_dialog(Some(Dialog::Delete(ws_id)));
            })
            .disabled(move || !can_delete()),
        )
}

/// Computes whole vertical slot steps from cumulative drag delta `dy` and `stride`.
fn compute_vertical_drag_steps(dy: f32, stride: f32) -> isize {
    if stride <= 0.0 {
        return 0;
    }
    let threshold = stride * 0.55;
    let abs_dy = dy.abs();
    if abs_dy < threshold {
        return 0;
    }
    let count = (((abs_dy - threshold) / stride).floor().min(1000.0) as isize) + 1;
    if dy.is_sign_positive() { count } else { -count }
}

fn ws_menu_item(label: String, select: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(label.clone())
        .flex_row()
        .items_center()
        .h(px(32.0))
        .px_3()
        .rounded(px(7.0))
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

/// What a row of the command-palette picker does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Go(WorkspaceId),
}

/// Command-palette list for sending a tab to another workspace (`Dialog::Send`) or picking a workspace to
/// open in a new window (`Dialog::Window`, W.8).
fn workspace_command_list(
    workspaces: Workspaces,
    list: Dialog,
    _run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let rows = untracked(|| command_rows(workspaces));
    let items: Vec<CommandItem> = rows.iter().map(|(item, _)| item.clone()).collect();
    let find = move |id: &str| {
        rows.iter()
            .find(|(item, _)| item.id.as_ref() == id)
            .map(|(_, row)| *row)
    };
    let title = match list {
        Dialog::Send(Transfer::Move) => t!("workspace.move_to"),
        Dialog::Send(Transfer::Copy) => t!("workspace.copy_to"),
        Dialog::Window => t!("workspace.window_title"),
        Dialog::Switch | Dialog::Edit(_) | Dialog::Delete(_) | Dialog::Notice => {
            t!("workspace.list")
        }
    };
    let panel = command()
        .items(move || items.clone())
        .placeholder(t!("workspace.search"))
        .accessibility_label(title.clone())
        .auto_focus(true)
        .on_select(move |id: &SharedString| {
            let Some(Row::Go(to)) = find(id) else {
                return;
            };
            close();
            match list {
                Dialog::Send(transfer) => workspaces.send_tab(transfer, to),
                Dialog::Window => workspaces.open_in_window(to),
                _ => workspaces.switch(to),
            }
        })
        .on_cancel(close)
        .search_bar_class("flex-row items-center h-10 px-3 border-b border-border")
        .search_class("flex-1 text-sm")
        .list_class("max-h-96 py-1.5")
        .group_class("px-3 pt-2.5 pb-1 text-xs text-muted-foreground")
        .item_class(
            "mx-1.5 px-2.5 py-1.5 rounded-md text-sm text-popover-foreground \
             focus:bg-accent focus:text-accent-foreground checked:font-medium",
        )
        .w(px(260.0))
        .rounded(px(12.0))
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg();
    dismissible_dialog(
        title,
        DialogPlacement::TopLeft {
            top: px(42.0),
            left: px(56.0),
        },
        close,
        panel,
    )
}

fn command_rows(workspaces: Workspaces) -> Vec<(CommandItem, Row)> {
    let shown = workspaces.shown_id();
    let list = workspaces.list();
    let keymap = keymap::current();
    let group: SharedString = t!("workspace.group.all", count = list.len()).into();
    list.iter()
        .enumerate()
        .filter(|(_, w)| w.id != shown)
        .map(|(n, w)| {
            let label = if workspaces.in_other_window(w.id) {
                t!(
                    "workspace.row_elsewhere",
                    name = row_name(w),
                    count = w.tabs
                )
            } else {
                t!("workspace.row", name = row_name(w), count = w.tabs)
            };
            let mut item = CommandItem::new(format!("workspace:{n}"), label);
            item.group = Some(group.clone());
            if let Some(keys) = keymap.first_keys(CommandId::Workspace(WorkspaceAction::Go(n))) {
                item = item.shortcut(keys.to_owned());
            }
            (item, Row::Go(w.id))
        })
        .collect()
}

/// A workspace's name with its icon before it.
fn row_name(workspace: &Workspace) -> String {
    match &workspace.icon {
        Some(icon) => format!("{icon}  {}", display_name(workspace)),
        None => display_name(workspace),
    }
}

/// Workspace `id`'s edit dialog (W.3): name, icon and colour, which apply as they are made; its place in the
/// list; and Delete.
fn edit_dialog(workspaces: Workspaces, id: WorkspaceId) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let start = untracked(|| workspaces.workspace(id)).map_or_else(Look::default, |w| Look {
        name: w.name,
        icon: w.icon,
        color: w.color,
    });
    let look = signal(start);
    let change = move |f: &dyn Fn(&mut Look)| {
        look.update(|look| f(look));
        workspaces.restyle_id(id, look.get_untracked());
    };
    let name = input()
        .value(move || look.with(|look| look.name.clone()))
        .on_change(move |text: &str| change(&|look| look.name = text.to_owned()))
        .on_submit(move |_: &str| close())
        .placeholder(t!("workspace.default_name"))
        .label(t!("workspace.name"))
        .auto_focus(true)
        .h(px(36.0))
        .px_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(14.0))
        .text_color(move || theme().popover_foreground);
    let icons = ICONS
        .into_iter()
        .fold(view().flex_row().gap_1(), |row, icon| {
            row.child(icon_button(
                icon,
                move || look.with(|look| look.icon.as_deref() == Some(icon)),
                move || {
                    change(&|look| {
                        look.icon = (look.icon.as_deref() != Some(icon)).then(|| icon.to_owned());
                    });
                },
            ))
        });
    let swatches = GroupColor::ALL
        .into_iter()
        .fold(view().flex_row().gap_2(), |row, swatch| {
            row.child(swatch_button(
                swatch,
                move || look.with(|look| look.color.as_deref() == Some(swatch.id())),
                move || change(&|look| look.color = Some(swatch.id().to_owned())),
            ))
        });
    let deletable = untracked(|| workspaces.list().len() > 1 && !workspaces.in_other_window(id));
    let form = view()
        .w(px(360.0))
        .p_4()
        .gap_3()
        .child(heading(t!("workspace.edit_title")))
        .child(name)
        .child(caption("workspace.icon"))
        .child(icons)
        .child(caption("workspace.color"))
        .child(swatches)
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(action_button(
                    "workspace.move_earlier",
                    Tone::Plain,
                    move || {
                        workspaces.shift_id(id, Step::Previous);
                    },
                ))
                .child(action_button(
                    "workspace.move_later",
                    Tone::Plain,
                    move || {
                        workspaces.shift_id(id, Step::Next);
                    },
                ))
                .child(view().flex_1())
                .child(when(
                    move || deletable,
                    move || {
                        action_button("workspace.delete", Tone::Destructive, move || {
                            workspaces.set_dialog(Some(Dialog::Delete(id)));
                        })
                    },
                ))
                .child(action_button("workspace.done", Tone::Plain, close)),
        );
    list_dialog(t!("workspace.edit_title"), close, card(form))
}

/// "Delete this workspace?": its tabs move to the workspace before it, or close.
fn delete_dialog(workspaces: Workspaces, id: WorkspaceId) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let (name, count, neighbour) = untracked(|| {
        (
            workspaces
                .workspace(id)
                .map(|w| display_name(&w))
                .unwrap_or_default(),
            workspaces.tab_count_of(id),
            workspaces
                .neighbour_for(id)
                .map(|w| display_name(&w))
                .unwrap_or_default(),
        )
    });
    let delete = move |tabs_go: ClosingTabs| {
        close();
        workspaces.delete_id(id, tabs_go);
    };
    let form = view()
        .w(px(400.0))
        .p_4()
        .gap_3()
        .child(heading(t!("workspace.delete_title", name = name)))
        .child(
            text(t!(
                "workspace.delete_hint",
                count = count,
                other = neighbour
            ))
            .text_size(px(13.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .justify_end()
                .child(action_button("workspace.cancel", Tone::Plain, close))
                .child(action_button(
                    "workspace.delete_move",
                    Tone::Plain,
                    move || {
                        delete(ClosingTabs::Move);
                    },
                ))
                .child(action_button(
                    "workspace.delete_close",
                    Tone::Destructive,
                    move || {
                        delete(ClosingTabs::Close);
                    },
                )),
        );
    list_dialog(t!("workspace.delete_title", name = name), close, card(form))
}

/// What came of an export or an import (W.11), until the reader closes it.
fn notice_dialog(workspaces: Workspaces) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let message = untracked(|| workspaces.notice());
    let form = view()
        .w(px(400.0))
        .p_4()
        .gap_3()
        .child(heading(t!("workspace.file_title")))
        .child(
            text(message)
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(view().flex_row().justify_end().child(action_button(
            "workspace.done",
            Tone::Plain,
            close,
        )));
    list_dialog(t!("workspace.file_title"), close, card(form))
}

/// A dialog's heading.
fn heading(label: String) -> impl IntoElement {
    text(label)
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().popover_foreground)
}

/// A field's caption, by its translation key.
fn caption(key: &'static str) -> impl IntoElement {
    text(move || t!(key))
        .text_size(px(12.0))
        .text_color(move || theme().muted_foreground)
}

/// One icon to pick; pressing the chosen one again takes it off.
fn icon_button(
    icon: &'static str,
    chosen: impl Fn() -> bool + 'static,
    pick: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(t!("workspace.icon_choice", icon = icon))
        .on_change(move |_: &bool| pick())
        .size(px(32.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .border_2()
        .border_color(move || theme().popover)
        .hover(|s| s.bg(move || theme().muted))
        .pressed_style(|s| s.border_color(move || theme().foreground))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(text(icon).text_size(px(16.0)))
}

/// One colour to pick, named for screen readers; the chosen one is ringed.
fn swatch_button(
    swatch: GroupColor,
    chosen: impl Fn() -> bool + 'static,
    pick: impl Fn() + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(color_name(swatch))
        .on_change(move |_: &bool| pick())
        .size(px(28.0))
        .rounded_full()
        .bg(Hsla::parse_hex(swatch.swatch().0).expect("a built-in colour"))
        .border_2()
        .border_color(move || theme().popover)
        .pressed_style(|s| s.border_color(move || theme().foreground))
        .focus_visible(|s| s.border_color(move || theme().ring))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_drag_steps_require_threshold_and_preserve_sign() {
        assert_eq!(compute_vertical_drag_steps(0.0, 52.0), 0);
        assert_eq!(compute_vertical_drag_steps(20.0, 52.0), 0);
        assert_eq!(compute_vertical_drag_steps(30.0, 52.0), 1);
        assert_eq!(compute_vertical_drag_steps(-30.0, 52.0), -1);
        assert_eq!(compute_vertical_drag_steps(90.0, 52.0), 2);
    }

    #[test]
    fn auto_icons_are_stable_and_spread_across_pool() {
        assert_eq!(
            auto_icon_index(WorkspaceId::DEFAULT),
            auto_icon_index(WorkspaceId::DEFAULT)
        );
        let distinct: std::collections::HashSet<usize> =
            (1u64..=10).map(auto_icon_index_from_hash).collect();
        assert!(
            distinct.len() >= 6,
            "auto icons should vary across workspaces"
        );
    }
}
