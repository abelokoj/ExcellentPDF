//! The workspace switcher (W.1): a button at the left of the title bar with the shown workspace's colour, icon and
//! name, and the workspace dialogs: the list (to switch, or to send the shown tab to another workspace, W.4), the
//! edit dialog (name, icon, colour, order; W.3) and "Delete this workspace?". They are dialogs, as the palette is:
//! the page canvas is a window over the app's, so a popover under the title bar would be drawn behind it.

use excellent_store::workspace::WorkspaceId;
use excellent_store::workspaces::{Look, Workspace};
use nsh_base::{CommandItem, button, command, input, theme, toggle};
use nsh_core::prelude::*;

use super::{Dialog, Start, Transfer, WorkspaceAction, Workspaces, display_name};
use crate::commands::{self, CommandId};
use crate::keymap;
use crate::list_dialog::{card, list_dialog, style};
use crate::panes::ClosingTabs;
use crate::tabs::{GroupColor, Step, Tone, action_button, color_name};

/// The icons offered in the edit dialog; typing another works too.
const ICONS: [&str; 8] = ["📚", "📄", "🎓", "💼", "🧾", "🔬", "✏️", "⭐"];

/// The switcher button: the shown workspace's colour, icon and name. Pressing it lists every workspace.
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
    let icon = move || workspaces.shown().and_then(|w| w.icon);
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
            .h(px(26.0))
            .max_w(px(200.0))
            .px_2()
            .gap(px(6.0))
            .flex_row()
            .items_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.7))
            .focus_visible(|s| s.border_2().border_color(move || theme().ring))
            .child(color_dot(move || workspaces.shown().and_then(|w| w.color)))
            .child(when(
                move || icon().is_some(),
                move || text(move || icon().unwrap_or_default()).text_size(px(13.0)),
            ))
            .child(
                text(name)
                    .truncate()
                    .min_w(px(0.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().sidebar_foreground),
            )
            .child(
                text("⌄")
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

/// A workspace's colour as a small dot; the muted colour when it has none.
fn color_dot(color: impl Fn() -> Option<String> + 'static) -> impl IntoElement {
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
            Some(Dialog::Switch) => workspace_list(workspaces, None, run).into_element(),
            Some(Dialog::Send(transfer)) => {
                workspace_list(workspaces, Some(transfer), run).into_element()
            }
            Some(Dialog::Edit) => edit_dialog(workspaces).into_element(),
            Some(Dialog::Delete) => delete_dialog(workspaces).into_element(),
            None => view().into_element(),
        })
    })
}

/// What a row of the list does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Go(WorkspaceId),
    Run(WorkspaceAction),
}

/// Every workspace, to switch to, or (while `sending`) to send the shown tab to; switching also offers the
/// workspace commands.
fn workspace_list(
    workspaces: Workspaces,
    sending: Option<Transfer>,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let rows = untracked(|| rows(workspaces, sending));
    let items: Vec<CommandItem> = rows.iter().map(|(item, _)| item.clone()).collect();
    let find = move |id: &str| {
        rows.iter()
            .find(|(item, _)| item.id.as_ref() == id)
            .map(|(_, row)| *row)
    };
    let title = match sending {
        None => t!("workspace.list"),
        Some(Transfer::Move) => t!("workspace.move_to"),
        Some(Transfer::Copy) => t!("workspace.copy_to"),
    };
    list_dialog(
        title.clone(),
        close,
        style(
            command()
                .items(move || items.clone())
                .placeholder(t!("workspace.search"))
                .accessibility_label(title)
                .auto_focus(true)
                .on_select(move |id: &SharedString| {
                    let Some(row) = find(id) else {
                        return;
                    };
                    close();
                    match (row, sending) {
                        (Row::Go(to), Some(transfer)) => workspaces.send_tab(transfer, to),
                        (Row::Go(to), None) => workspaces.switch(to),
                        (Row::Run(action), _) => run(CommandId::Workspace(action)),
                    }
                })
                .on_cancel(close),
        ),
    )
}

/// The list's rows: each workspace with its tab count (the shown one checked, or left out while sending), then the
/// commands.
fn rows(workspaces: Workspaces, sending: Option<Transfer>) -> Vec<(CommandItem, Row)> {
    let shown = workspaces.shown_id();
    let list = workspaces.list();
    let keymap = keymap::current();
    let group: SharedString = t!("workspace.group.all", count = list.len()).into();
    let mut rows: Vec<(CommandItem, Row)> = list
        .iter()
        .enumerate()
        .filter(|(_, w)| sending.is_none() || w.id != shown)
        .map(|(n, w)| {
            let tabs = if w.id == shown {
                workspaces.shown_tab_count()
            } else {
                w.tabs
            };
            let label = t!("workspace.row", name = row_name(w), count = tabs);
            let mut item = CommandItem::new(format!("workspace:{n}"), label);
            item.group = Some(group.clone());
            item.checked = sending.is_none() && w.id == shown;
            if let Some(keys) = keymap.first_keys(CommandId::Workspace(WorkspaceAction::Go(n))) {
                item = item.shortcut(keys.to_owned());
            }
            (item, Row::Go(w.id))
        })
        .collect();
    if sending.is_none() {
        let actions: SharedString = t!("workspace.group.actions").into();
        for action in [
            WorkspaceAction::New(Start::Empty),
            WorkspaceAction::New(Start::AllTabs),
            WorkspaceAction::New(Start::ShownTab),
            WorkspaceAction::SendTab(Transfer::Move),
            WorkspaceAction::Edit,
        ] {
            let id = CommandId::Workspace(action);
            let available = workspaces.panes.active().tabs.has_shown();
            let needs_tab = matches!(
                action,
                WorkspaceAction::New(Start::ShownTab | Start::AllTabs)
                    | WorkspaceAction::SendTab(_)
            );
            if needs_tab && !available {
                continue;
            }
            let mut item = CommandItem::new(
                format!("action:{rows}", rows = rows.len()),
                commands::label(id),
            );
            item.group = Some(actions.clone());
            if let Some(keys) = keymap.first_keys(id) {
                item = item.shortcut(keys.to_owned());
            }
            rows.push((item, Row::Run(action)));
        }
    }
    rows
}

/// A workspace's name with its icon before it.
fn row_name(workspace: &Workspace) -> String {
    match &workspace.icon {
        Some(icon) => format!("{icon}  {}", display_name(workspace)),
        None => display_name(workspace),
    }
}

/// The shown workspace's edit dialog (W.3): name, icon and colour, which apply as they are made; its place in the
/// list; and Delete.
fn edit_dialog(workspaces: Workspaces) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let start = untracked(|| workspaces.shown()).map_or_else(Look::default, |w| Look {
        name: w.name,
        icon: w.icon,
        color: w.color,
    });
    let look = signal(start);
    let change = move |f: &dyn Fn(&mut Look)| {
        look.update(|look| f(look));
        workspaces.restyle(look.get_untracked());
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
    let deletable = untracked(|| workspaces.list().len() > 1);
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
                        workspaces.run(WorkspaceAction::Shift(Step::Previous));
                    },
                ))
                .child(action_button(
                    "workspace.move_later",
                    Tone::Plain,
                    move || {
                        workspaces.run(WorkspaceAction::Shift(Step::Next));
                    },
                ))
                .child(view().flex_1())
                .child(when(
                    move || deletable,
                    move || {
                        action_button("workspace.delete", Tone::Destructive, move || {
                            workspaces.set_dialog(Some(Dialog::Delete));
                        })
                    },
                ))
                .child(action_button("workspace.done", Tone::Plain, close)),
        );
    list_dialog(t!("workspace.edit_title"), close, card(form))
}

/// "Delete this workspace?": its tabs move to the workspace before it, or close.
fn delete_dialog(workspaces: Workspaces) -> impl IntoElement {
    let close = move || workspaces.set_dialog(None);
    let (name, count, neighbour) = untracked(|| {
        (
            workspaces
                .shown()
                .map(|w| display_name(&w))
                .unwrap_or_default(),
            workspaces.shown_tab_count(),
            workspaces
                .neighbour()
                .map(|w| display_name(&w))
                .unwrap_or_default(),
        )
    });
    let delete = move |tabs_go: ClosingTabs| {
        close();
        workspaces.delete_shown(tabs_go);
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
