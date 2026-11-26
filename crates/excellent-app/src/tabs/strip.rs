//! The tab strip (R.12, plan §10.1): the open files in the title bar, in NSH's tab list, so the arrow keys move
//! between them and screen readers hear one list. The shown tab takes the page's colour, as if the page came up into
//! it; a dot marks unsaved changes, a pin the pinned tabs (which keep no close button). A middle click closes a tab.
//! Then the open button and, at the right end, the button of the tab list.
//!
//! A tab group (R.13) shows as a chip in its colour before its tabs, which are underlined in that colour. The chip
//! names the group and, while it is collapsed, counts its hidden tabs; pressing it collapses or expands the group, a
//! right click opens its edit dialog.

use nsh_base::{button, tabs, theme};
use nsh_core::geometry::Radians;
use nsh_core::prelude::*;

use super::group_dialog::color_name;
use super::model::Tab;
use super::{GroupColor, GroupId, TabId, TabState, Tabs};
use crate::commands::{self, CommandId};

/// A tab's widths: it shrinks as tabs are added, down to `MIN`.
const TAB_WIDTH: f32 = 200.0;
const MIN_TAB_WIDTH: f32 = 56.0;
const TAB_HEIGHT: f32 = 30.0;

/// The strip: the tabs, the open button and the tab-list button; the app's name while nothing is open.
pub(crate) fn tab_strip(
    state: TabState,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let list = tabs()
        .flex_row()
        .items_end()
        .gap(px(2.0))
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .h_full()
        .child(each(
            move || state.tabs.with(items),
            |item: &Item| *item,
            move |item, _| match item.with_untracked(|item| *item) {
                Item::Group(id) => group_chip(state, id).into_element(),
                Item::Tab(id) => tab_view(state, id).into_element(),
            },
        ));
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || !state.has_tabs(),
            || {
                text(move || t!("app.name"))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().sidebar_foreground)
            },
        ))
        .child(list)
        .child(strip_button(
            CommandId::OpenFile,
            run,
            text("+")
                .text_size(px(18.0))
                .text_color(move || theme().sidebar_foreground),
        ))
        .child(view().flex_1())
        .child(when(
            move || state.has_any(),
            move || strip_button(CommandId::TabList, run, chevron()),
        ))
}

/// What the strip shows, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Item {
    /// A group's chip, before its tabs.
    Group(GroupId),
    /// A tab not hidden in a collapsed group.
    Tab(TabId),
}

/// The strip's items: the tabs in order, each group's chip before its first tab (a group's tabs sit together).
fn items(tabs: &Tabs) -> Vec<Item> {
    let mut items = Vec::new();
    let mut last = None;
    for tab in tabs.all() {
        if tab.group != last {
            items.extend(tab.group.map(Item::Group));
            last = tab.group;
        }
        if !tabs.hidden(tab.id) {
            items.push(Item::Tab(tab.id));
        }
    }
    items
}

/// The fill of `color`'s chip, and the text on it.
fn swatch(color: GroupColor) -> (Hsla, Hsla) {
    let (fill, ink) = color.swatch();
    let parse = |hex| Hsla::parse_hex(hex).expect("a built-in colour");
    (parse(fill), parse(ink))
}

/// A group's chip: its name on its colour, a dot when it has none; while collapsed, the number of its tabs.
fn group_chip(state: TabState, id: GroupId) -> impl IntoElement {
    let group = computed(move || {
        state.tabs.with(|tabs| {
            tabs.group(id)
                .cloned()
                .map(|group| (group, tabs.members(id).count()))
        })
    });
    let color = move || group.with(|g| g.as_ref().map_or(GroupColor::Grey, |(g, _)| g.color));
    let name = move || group.with(|g| g.as_ref().map(|(g, _)| g.name.clone()).unwrap_or_default());
    let collapsed = move || group.with(|g| g.as_ref().is_some_and(|(g, _)| g.collapsed));
    let count = move || group.with(|g| g.as_ref().map_or(0, |(_, count)| *count));
    let fill = move || swatch(color()).0;
    let ink = move || swatch(color()).1;
    // A screen reader hears the name (or the colour, for a group without one), the tab count and the state: NSH's
    // `Button` sets no expanded state, so the label says it.
    let label = move || {
        let name = name();
        let name = if name.is_empty() {
            color_name(color())
        } else {
            name
        };
        let key = if collapsed() {
            "tab_group.chip_collapsed"
        } else {
            "tab_group.chip_expanded"
        };
        SharedString::from(t!(key, name = name, count = count()))
    };
    let named = move || !name().is_empty();
    let chip = button()
        .accessibility_label(move || Some(label()))
        .on_press(move || state.toggle_group(id))
        .flex_shrink(0.0)
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(px(22.0))
        .min_w(px(22.0))
        .px(px(6.0))
        .rounded_full()
        .justify_center()
        .bg(fill)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(when(
            move || !named() && !collapsed(),
            move || view().w(px(8.0)).h(px(8.0)).rounded_full().bg(ink),
        ))
        .child(when(named, move || {
            text(name)
                .max_w(px(140.0))
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ink)
        }))
        .child(when(collapsed, move || {
            text(move || count().to_string())
                .tabular_nums()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ink)
        }));
    // A right click opens the edit dialog; the chip itself takes only presses.
    view()
        .window_drag(false)
        .on_mouse_up(move |event: &MouseEvent| {
            if event.button == Some(MouseButton::Right) {
                state.edit(id);
            }
        })
        .h_full()
        .items_center()
        .justify_center()
        .px(px(2.0))
        .child(chip)
}

/// One tab: its file's name, a dot while it has unsaved changes, a pin or a close button; underlined in its group's
/// colour.
fn tab_view(state: TabState, id: TabId) -> impl IntoElement {
    let tab = computed(move || state.tabs.with(|tabs| tabs.get(id).cloned()));
    let shown = move || state.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) == Some(id);
    let multi_selected = move || state.is_selected(id);
    let pinned = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.pinned));
    let missing = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.missing));
    let dirty = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.dirty));
    let title = move || tab.with(|tab| tab.as_ref().map(Tab::title).unwrap_or_default());
    let underline = move || {
        state.tabs.with(|tabs| {
            tabs.get(id)
                .and_then(|tab| tab.group)
                .and_then(|group| tabs.group(group))
                .map_or(Hsla::transparent_black(), |group| swatch(group.color).0)
        })
    };
    let tab_view = nsh_base::tab()
        .selected(shown)
        .on_press(move || state.show(id))
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(TAB_HEIGHT))
        .pl_3()
        .pr_1()
        .rounded_t(px(8.0))
        .border_t(px(2.0))
        .border_t_color(
            move || match use_context::<crate::modes::Modes>().filter(|_| shown()) {
                Some(modes) => crate::modes::accent(modes.current()),
                None => Hsla::transparent_black().into(),
            },
        )
        .border_b(px(2.0))
        .border_b_color(underline)
        .style_when(multi_selected, |s| {
            s.border_1().border_color(move || theme().primary)
        })
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .selected_style(|s| s.bg(move || theme().background))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(when(pinned, || {
            pin_mark(move || theme().sidebar_foreground)
        }))
        .child(when(dirty, || {
            view()
                .label(t!("tabs.unsaved"))
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(move || theme().primary)
        }))
        // A file that has gone: a hollow ring where the unsaved dot would be, and the name greyed (R.9).
        .child(when(missing, || {
            view()
                .label(t!("tabs.missing"))
                .w(px(7.0))
                .h(px(7.0))
                .rounded_full()
                .border_1()
                .border_color(move || theme().muted_foreground)
        }))
        .child(
            text(title)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if missing() {
                        theme().muted_foreground
                    } else if shown() {
                        theme().foreground
                    } else {
                        theme().sidebar_foreground
                    }
                }),
        )
        .child(when(
            move || !pinned(),
            move || close_button(state, id, title),
        ));
    // The middle button closes, as in browsers; Ctrl/Cmd+Left click multi-selects tabs for tiling (V.12).
    view()
        .window_drag(false)
        .on_mouse_up(move |event: &MouseEvent| {
            if event.button == Some(MouseButton::Middle) && !pinned() {
                state.close(id);
            } else if event.button == Some(MouseButton::Left)
                && (event.modifiers.ctrl || event.modifiers.cmd || event.modifiers.shift)
            {
                state.toggle_selected(id);
            }
        })
        .flex_row()
        .w(px(TAB_WIDTH))
        .min_w(px(MIN_TAB_WIDTH))
        .flex_shrink(1.0)
        .child(tab_view)
}

/// A tab's close button.
fn close_button(
    state: TabState,
    id: TabId,
    title: impl Fn() -> String + Copy + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(move || Some(SharedString::from(t!("tabs.close", name = title()))))
        .on_press(move || state.close(id))
        .w(px(20.0))
        .h(px(20.0))
        .flex_shrink(0.0)
        .rounded(px(4.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.7))
        .child(
            text("×")
                .text_size(px(14.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// The mark of something pinned (a tab, a recent file): a small filled disc on a stem, drawn from views in `color`.
pub(crate) fn pin_mark(color: impl Fn() -> AdaptiveColor + Copy + 'static) -> impl IntoElement {
    view()
        .items_center()
        .flex_shrink(0.0)
        .accessibility_elements_hidden(true)
        .child(view().w(px(7.0)).h(px(7.0)).rounded_full().bg(color))
        .child(view().w(px(1.5)).h(px(5.0)).bg(color))
}

/// A downward chevron, drawn as the corner of a rotated square.
fn chevron() -> impl IntoElement {
    view()
        .w(px(7.0))
        .h(px(7.0))
        .border_r(px(2.0))
        .border_b(px(2.0))
        .border_color(move || theme().sidebar_foreground)
        .rotate(Radians(std::f32::consts::FRAC_PI_4))
}

/// A small button of the strip that runs `id`, named after the command and its shortcut.
fn strip_button(
    id: CommandId,
    run: impl Fn(CommandId) + Copy + 'static,
    glyph: impl IntoElement,
) -> impl IntoElement {
    view().window_drag(false).child(
        button()
            .accessibility_label(move || {
                Some(SharedString::from(commands::label_with_shortcut(id)))
            })
            .on_press(move || run(id))
            .w(px(28.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .items_center()
            .justify_center()
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.7))
            .child(glyph),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_group_shows_its_chip_before_its_tabs_and_hides_them_collapsed() {
        let mut tabs = Tabs::default();
        let ids: Vec<TabId> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|name| tabs.open(PathBuf::from(format!("{name}.pdf")), 0))
            .collect();
        let group = tabs.new_group(ids[1]).expect("group");
        tabs.step_tab(ids[2], false);
        let open = vec![
            Item::Tab(ids[0]),
            Item::Group(group),
            Item::Tab(ids[1]),
            Item::Tab(ids[2]),
            Item::Tab(ids[3]),
        ];
        assert_eq!(items(&tabs), open);
        assert!(tabs.toggle_group(group));
        assert_eq!(
            items(&tabs),
            [Item::Tab(ids[0]), Item::Group(group), Item::Tab(ids[3])]
        );
    }
}
