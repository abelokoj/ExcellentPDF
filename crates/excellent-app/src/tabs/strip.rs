//! The tab strip (R.12, plan §10.1): the open files in the title bar, in NSH's tab list, so the arrow keys move
//! between them and screen readers hear one list. The shown tab takes the page's colour, as if the page came up into
//! it; a dot marks unsaved changes, a pin the pinned tabs (which keep no close button). A middle click closes a tab.
//! Then the open button and, at the right end, the button of the tab list.
//!
//! A tab group (R.13) shows as a chip in its colour before its tabs, which are underlined in that colour. The chip
//! names the group and, while it is collapsed, counts its hidden tabs; pressing it collapses or expands the group, a
//! right click opens its edit dialog.

use std::sync::OnceLock;

use base64::Engine as _;
use nsh_base::{button, tabs, theme};
use nsh_core::prelude::*;

use super::group_dialog::color_name;
use super::model::Tab;
use super::{GroupColor, GroupId, TAB_STYLE, TabId, TabState, TabStyle, Tabs, min_width_px};
use crate::commands::{self, CommandId};

/// Width of pinned compact tabs, heights for Vivaldi/Chrome tab styles, and O(1) strip bound.
const PINNED_TAB_WIDTH: f32 = 44.0;
const TAB_HEIGHT_VIVALDI: f32 = 28.0;
const TAB_HEIGHT_CHROME: f32 = 30.0;
/// Maximum number of tab buttons mounted concurrently in the strip so 300+ open tabs stay O(1).
const MAX_STRIP_TABS: usize = 20;

const HOME_ICON_PNG: &[u8] = include_bytes!("../../assets/icons/home.png");

fn home_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(HOME_ICON_PNG)
        )
    })
    .clone()
}

/// The strip: always-accessible Home tab, horizontally scrollable O(1) window of tabs sized by
/// the configured minimum width, the open button, an overflow badge (`+N`), and blank drag space.
pub(crate) fn tab_strip(
    state: TabState,
    home_open: Signal<bool>,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let tab_style = move || state.live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let strip = computed(move || state.tabs.with(|t| windowed_items(t, MAX_STRIP_TABS)));
    let overflow = move || strip.with(|(_, hidden)| *hidden);
    let list = tabs()
        .flex_row()
        .flex_shrink(0.0)
        .h_full()
        .style_when(
            move || tab_style() == TabStyle::Vivaldi,
            |s| s.items_center().gap(px(4.0)),
        )
        .style_when(
            move || tab_style() == TabStyle::Chrome,
            |s| s.items_end().gap(px(2.0)),
        )
        .child(each(
            move || strip.with(|(items, _)| items.clone()),
            |item: &Item| *item,
            move |item, _| match item.with_untracked(|item| *item) {
                Item::Group(id) => group_chip(state, id).into_element(),
                Item::Tab(id) => tab_view(state, home_open, id).into_element(),
            },
        ));
    let scroller = scroll()
        .horizontal()
        .scrollbar(nsh_core::ScrollbarSetting::Hidden)
        .flex_shrink(1.0)
        .min_w(px(0.0))
        .h_full()
        .child(list);
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .items_center()
        .gap_1()
        .child(home_tab(state, home_open, tab_style))
        .child(scroller)
        .child(strip_button(
            CommandId::OpenFile,
            run,
            text("+")
                .text_size(px(18.0))
                .text_color(move || theme().sidebar_foreground),
        ))
        .child(when(
            move || overflow() > 0,
            move || overflow_pill(overflow, run),
        ))
        .child(view().flex_1().min_w(px(0.0)).h_full())
}

/// Pinned Home tab at the start of the strip so users can open or return to the Home page at any time.
fn home_tab(
    state: TabState,
    home_open: Signal<bool>,
    tab_style: impl Fn() -> TabStyle + Copy + 'static,
) -> impl IntoElement {
    let active = move || !state.has_tabs() || home_open.get();
    view().window_drag(false).flex_shrink(0.0).child(
        button()
            .accessibility_label(t!("home.tab"))
            .on_press(move || home_open.set(true))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .style_when(
                move || tab_style() == TabStyle::Vivaldi,
                |s| {
                    s.h(px(TAB_HEIGHT_VIVALDI))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || {
                            if active() {
                                theme().border
                            } else {
                                Hsla::transparent_black().into()
                            }
                        })
                        .bg(move || {
                            if active() {
                                theme().background
                            } else {
                                theme().secondary
                            }
                        })
                },
            )
            .style_when(
                move || tab_style() == TabStyle::Chrome,
                |s| {
                    s.h(px(TAB_HEIGHT_CHROME))
                        .rounded_t(px(8.0))
                        .border_t(px(2.0))
                        .border_t_color(move || {
                            if active() {
                                theme().primary
                            } else {
                                Hsla::transparent_black().into()
                            }
                        })
                },
            )
            .style_when(
                move || tab_style() == TabStyle::Chrome && active(),
                |s| s.bg(move || theme().background),
            )
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.8))
            .focus_visible(|s| s.border_1().border_color(move || theme().ring))
            .child(
                image(home_icon_src())
                    .w(px(14.0))
                    .h(px(14.0))
                    .object_fit(ResizeMode::Contain)
                    .tint_color(move || {
                        if active() {
                            theme().foreground
                        } else {
                            theme().sidebar_foreground
                        }
                    }),
            )
            .child(
                text(move || t!("home.tab"))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if active() {
                            theme().foreground
                        } else {
                            theme().sidebar_foreground
                        }
                    }),
            ),
    )
}

/// What the strip shows, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Item {
    /// A group's chip, before its tabs.
    Group(GroupId),
    /// A tab not hidden in a collapsed group.
    Tab(TabId),
}

/// Computes all uncollapsed strip items (used for ordering/testing).
#[cfg(test)]
fn items(tabs: &Tabs) -> Vec<Item> {
    windowed_items(tabs, usize::MAX).0
}

/// Computes at most `max_tabs` uncollapsed tabs (always keeping pinned tabs and a sliding window
/// centered on the active tab) plus their leading group chips, and returns `(items, omitted_tabs)`.
fn windowed_items(tabs: &Tabs, max_tabs: usize) -> (Vec<Item>, usize) {
    let all = tabs.all();
    let visible_indices: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, tab)| !tabs.hidden(tab.id))
        .map(|(i, _)| i)
        .collect();
    let total_visible = visible_indices.len();
    if total_visible <= max_tabs {
        let mut keep = vec![false; all.len()];
        for &idx in &visible_indices {
            keep[idx] = true;
        }
        return (build_items(tabs, all, &keep, true), 0);
    }
    let active_id = tabs.active().map(|tab| tab.id);
    let mut keep = vec![false; all.len()];
    let mut kept = 0usize;
    // Keep active tab first so it is guaranteed visible even under extreme pinning.
    if let Some(pos) = visible_indices
        .iter()
        .copied()
        .find(|&i| Some(all[i].id) == active_id)
    {
        keep[pos] = true;
        kept += 1;
    }
    for &i in &visible_indices {
        if kept >= max_tabs {
            break;
        }
        if !keep[i] && all[i].pinned {
            keep[i] = true;
            kept += 1;
        }
    }
    let center = visible_indices
        .iter()
        .position(|&i| Some(all[i].id) == active_id)
        .unwrap_or(0);
    for step in 0..visible_indices.len() {
        if kept >= max_tabs {
            break;
        }
        let offsets = [
            center.checked_add(step),
            (step > 0).then(|| center.checked_sub(step)).flatten(),
        ];
        for cand in offsets.into_iter().flatten() {
            if kept >= max_tabs {
                break;
            }
            if let Some(&idx) = visible_indices.get(cand)
                && !keep[idx]
            {
                keep[idx] = true;
                kept += 1;
            }
        }
    }
    let omitted = total_visible.saturating_sub(kept);
    (build_items(tabs, all, &keep, false), omitted)
}

fn build_items(tabs: &Tabs, all: &[Tab], keep: &[bool], include_all_collapsed: bool) -> Vec<Item> {
    let mut items = Vec::with_capacity(keep.iter().filter(|&&k| k).count() + 4);
    let mut last_emitted_group = None;
    let first_kept = keep.iter().position(|&k| k).unwrap_or(0);
    let last_kept = keep.iter().rposition(|&k| k).unwrap_or(0);
    for (idx, tab) in all.iter().enumerate() {
        let kept = keep.get(idx).copied().unwrap_or(false);
        if let Some(group_id) = tab.group {
            let collapsed = tabs.group(group_id).is_some_and(|group| group.collapsed);
            let near_window = include_all_collapsed || (idx >= first_kept && idx <= last_kept);
            if Some(group_id) != last_emitted_group && (kept || (collapsed && near_window)) {
                items.push(Item::Group(group_id));
                last_emitted_group = Some(group_id);
            }
        } else {
            last_emitted_group = None;
        }
        if kept {
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

/// One tab: Vivaldi rounded pill (default) or Chrome browser tab, sized by `tab_min_width_px`.
fn tab_view(state: TabState, home_open: Signal<bool>, id: TabId) -> impl IntoElement {
    let tab = computed(move || state.tabs.with(|tabs| tabs.get(id).cloned()));
    let tab_style = move || state.live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let min_tab_px = move || state.live.read(min_width_px);
    let shown = move || {
        !home_open.get() && state.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) == Some(id)
    };
    let multi_selected = move || state.is_selected(id);
    let pinned = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.pinned));
    let missing = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.missing));
    let dirty = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.dirty));
    let title = move || tab.with(|tab| tab.as_ref().map(Tab::title).unwrap_or_default());
    let has_group = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.group.is_some()));
    let underline = move || {
        state.tabs.with(|tabs| {
            tabs.get(id)
                .and_then(|tab| tab.group)
                .and_then(|group| tabs.group(group))
                .map_or(Hsla::transparent_black(), |group| swatch(group.color).0)
        })
    };
    let mode_accent = move || {
        use_context::<crate::modes::Modes>()
            .filter(|_| shown())
            .map(|modes| crate::modes::accent(modes.current()))
    };
    let tab_view = nsh_base::tab()
        .selected(shown)
        .on_press(move || {
            home_open.set(false);
            state.show(id);
        })
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .style_when(
            move || tab_style() == TabStyle::Vivaldi,
            |s| {
                s.h(px(TAB_HEIGHT_VIVALDI))
                    .pl(px(10.0))
                    .pr(px(6.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(move || {
                        if shown() {
                            theme().border
                        } else {
                            Hsla::transparent_black().into()
                        }
                    })
                    .bg(move || {
                        if shown() {
                            theme().background
                        } else {
                            theme().secondary
                        }
                    })
            },
        )
        .style_when(
            move || tab_style() == TabStyle::Vivaldi && has_group(),
            |s| s.border_b(px(2.0)).border_b_color(underline),
        )
        .style_when(
            move || tab_style() == TabStyle::Chrome,
            |s| {
                s.h(px(TAB_HEIGHT_CHROME))
                    .pl_3()
                    .pr_1()
                    .rounded_t(px(8.0))
                    .border_t(px(2.0))
                    .border_t_color(move || {
                        mode_accent().unwrap_or_else(|| Hsla::transparent_black().into())
                    })
                    .border_b(px(2.0))
                    .border_b_color(underline)
            },
        )
        .style_when(
            move || tab_style() == TabStyle::Chrome && shown(),
            |s| s.bg(move || theme().background),
        )
        .style_when(multi_selected, |s| {
            s.border_1().border_color(move || theme().primary)
        })
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(when(
            move || tab_style() == TabStyle::Vivaldi && shown(),
            move || {
                view()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .flex_shrink(0.0)
                    .bg(move || mode_accent().unwrap_or(theme().primary))
            },
        ))
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
        .w(move || {
            if pinned() {
                px(PINNED_TAB_WIDTH)
            } else {
                px(min_tab_px())
            }
        })
        .min_w(move || {
            if pinned() {
                px(PINNED_TAB_WIDTH)
            } else {
                px(min_tab_px())
            }
        })
        .flex_shrink(0.0)
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

/// Compact overflow indicator (`+N`) opening the searchable tab list when more tabs are open than fit in the strip.
fn overflow_pill(
    count: impl Fn() -> usize + Copy + 'static,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    view().window_drag(false).child(
        button()
            .accessibility_label(move || {
                Some(SharedString::from(t!("tabs.overflow", count = count())))
            })
            .on_press(move || run(CommandId::TabList))
            .h(px(22.0))
            .px_2()
            .rounded(px(6.0))
            .items_center()
            .justify_center()
            .bg(move || theme().sidebar_accent)
            .hover(|s| s.opacity(0.9))
            .active(|s| s.opacity(0.75))
            .child(
                text(move || format!("+{}", count()))
                    .tabular_nums()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().sidebar_foreground),
            ),
    )
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

    #[test]
    fn strip_bounds_visible_tabs_to_constant_window_for_350_open_tabs() {
        let mut tabs = Tabs::default();
        let ids: Vec<TabId> = (0..350)
            .map(|i| tabs.open(PathBuf::from(format!("doc_{i:03}.pdf")), 0))
            .collect();
        let active = ids[180];
        tabs.activate(active);
        let (window, omitted) = windowed_items(&tabs, MAX_STRIP_TABS);
        assert_eq!(window.len(), MAX_STRIP_TABS);
        assert_eq!(omitted, 350 - MAX_STRIP_TABS);
        assert!(window.contains(&Item::Tab(active)));
    }
}
