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
use ecclesia_store::workspaces::Workspace;
use nsh_base::{
    MenuItemElement, SubmenuElement, button, context_menu, menu_item, menu_separator, submenu,
    tabs, theme,
};
use nsh_core::prelude::*;

use super::group_dialog::color_name;
use super::model::Tab;
use super::{
    GroupColor, GroupId, TAB_STYLE, TabId, TabState, TabStyle, Tabs, min_width_px,
    tab_font_size_px, tab_height_px,
};
use crate::commands::{self, CommandId};
use crate::editing::Editing;
use crate::panes::{PaneAction, Panes, Side};
use crate::workspaces::Workspaces;

/// Width of pinned compact tabs and O(1) strip bound.
const PINNED_TAB_WIDTH: f32 = 44.0;
/// Maximum number of tab buttons mounted concurrently in the strip so 300+ open tabs stay O(1).
const MAX_STRIP_TABS: usize = 20;
/// Horizontal gap between tabs in Vivaldi pill mode (logical pixels).
const VIVALDI_TAB_GAP: f32 = 4.0;
/// Horizontal gap between tabs in Chrome tab mode (logical pixels).
const CHROME_TAB_GAP: f32 = 2.0;
/// Minimum horizontal pointer travel before a left-button press becomes a tab reorder drag.
const DRAG_START_THRESHOLD_PX: f32 = 6.0;
/// Fraction of one tab slot stride the pointer must cross before stepping one slot (prevents ping-pong).
const DRAG_STEP_THRESHOLD_RATIO: f32 = 0.55;

/// Live state of a tab being horizontally reordered by pointer drag.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TabDrag {
    tab: TabId,
    pinned: bool,
    /// Pointer X anchored to the tab's current slot (advances by ±slot stride on each reorder step).
    anchor_x: f32,
}

/// Returns the horizontal stride (tab width plus inter-tab gap) for one slot in the strip.
fn tab_slot_stride(pinned: bool, min_tab_px: f32, style: TabStyle) -> f32 {
    let width = if pinned { PINNED_TAB_WIDTH } else { min_tab_px };
    let gap = match style {
        TabStyle::Vivaldi => VIVALDI_TAB_GAP,
        TabStyle::Chrome => CHROME_TAB_GAP,
    };
    width + gap
}

/// Computes how many whole tab slots a horizontal drag delta (`delta_x`) should step, together
/// with the horizontal distance consumed by those steps.
///
/// Stepping activates once `|delta_x| >= slot_stride * 0.55` and consumes `steps * slot_stride`
/// from the drag anchor, providing hysteresis so a moved tab never ping-pongs back and forth.
fn compute_drag_steps(delta_x: f32, slot_stride: f32) -> (isize, f32) {
    if !delta_x.is_finite() || !slot_stride.is_finite() || slot_stride <= 0.0 {
        return (0, 0.0);
    }
    let threshold = slot_stride * DRAG_STEP_THRESHOLD_RATIO;
    let abs_dx = delta_x.abs();
    if abs_dx < threshold {
        return (0, 0.0);
    }
    let count = (((abs_dx - threshold) / slot_stride).floor().min(10_000.0) as isize) + 1;
    let steps = if delta_x.is_sign_positive() {
        count
    } else {
        -count
    };
    (steps, steps as f32 * slot_stride)
}

const HOME_ICON_SVG: &[u8] = include_bytes!("../../assets/icons/home.svg");

fn home_icon_src() -> String {
    static SRC: OnceLock<String> = OnceLock::new();
    SRC.get_or_init(|| {
        format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(HOME_ICON_SVG)
        )
    })
    .clone()
}

/// Shared strip handles passed to each tab item in the strip.
#[derive(Clone, Copy)]
struct TabStripCx {
    state: TabState,
    panes: Panes,
    home_open: Signal<bool>,
    editing: Editing,
    workspaces: Workspaces,
    drag: Signal<Option<TabDrag>>,
}

/// The strip: always-accessible Home tab, horizontally scrollable O(1) window of tabs sized by
/// the configured minimum width, the open button, an overflow badge (`+N`), and blank drag space.
pub(crate) fn tab_strip(
    state: TabState,
    panes: Panes,
    home_open: Signal<bool>,
    editing: Editing,
    workspaces: Workspaces,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let tab_style = move || state.live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let strip = computed(move || state.tabs.with(|t| windowed_items(t, MAX_STRIP_TABS)));
    let overflow = move || strip.with(|(_, hidden)| *hidden);
    let drag = signal(None::<TabDrag>);
    let cx = TabStripCx {
        state,
        panes,
        home_open,
        editing,
        workspaces,
        drag,
    };
    let list = tabs()
        .flex_row()
        .flex_shrink(0.0)
        .h_full()
        .style_when(
            move || tab_style() == TabStyle::Vivaldi,
            |s| s.items_center().gap(px(VIVALDI_TAB_GAP)),
        )
        .style_when(
            move || tab_style() == TabStyle::Chrome,
            |s| s.items_end().gap(px(CHROME_TAB_GAP)),
        )
        .child(each(
            move || strip.with(|(items, _)| items.clone()),
            |item: &Item| *item,
            move |item, _| match item.with_untracked(|item| *item) {
                Item::Group(id) => group_chip(state, id).into_element(),
                Item::Tab(id) => tab_view(cx, id, run).into_element(),
            },
        ));
    let scroller = scroll()
        .horizontal()
        .scrollbar(nsh_core::ScrollbarSetting::Hidden)
        .flex_shrink(1.0)
        .min_w(px(0.0))
        .h_full()
        .overflow_hidden()
        .child(list);
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .style_when(
            move || drag.with(Option::is_some),
            |s| s.cursor(Cursor::Grabbing),
        )
        .on_mouse_move(move |event: &MouseEvent| {
            if drag.get_untracked().is_some() && event.pressed_button != Some(MouseButton::Left) {
                drag.set(None);
            }
        })
        .on_mouse_up(move |_: &MouseEvent| {
            if drag.get_untracked().is_some() {
                drag.set(None);
            }
        })
        .on_mouse_leave(move |event: &MouseEvent| {
            if drag.get_untracked().is_some() && event.pressed_button != Some(MouseButton::Left) {
                drag.set(None);
            }
        })
        .child(home_tab(state, home_open))
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

/// Compact icon-only Home button at the start of the strip so users can open or return to the Home page at any time.
fn home_tab(state: TabState, home_open: Signal<bool>) -> impl IntoElement {
    let active = move || !state.has_tabs() || home_open.get();
    view().window_drag(false).flex_shrink(0.0).child(
        button()
            .accessibility_label(t!("home.tab"))
            .on_press(move || home_open.set(true))
            .w(px(32.0))
            .h(px(30.0))
            .rounded(px(6.0))
            .items_center()
            .justify_center()
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
                    Hsla::transparent_black().into()
                }
            })
            .hover(|s| s.bg(move || theme().sidebar_accent))
            .active(|s| s.opacity(0.8))
            .focus_visible(|s| s.border_1().border_color(move || theme().ring))
            .child(
                image(home_icon_src())
                    .w(px(18.0))
                    .h(px(18.0))
                    .object_fit(ResizeMode::Contain)
                    .tint_color(move || {
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
    let chip_h = move || (state.live.read(tab_height_px) - 6.0).clamp(22.0, 34.0);
    let chip = button()
        .accessibility_label(move || Some(label()))
        .on_press(move || state.toggle_group(id))
        .flex_shrink(0.0)
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(move || px(chip_h()))
        .min_w(move || px(chip_h()))
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

/// One tab: Vivaldi rounded pill (default) or Chrome browser tab, sized by `tab_min_width_px` and `tab_height_px`,
/// with drag-to-reorder and a full SumatraPDF Enhanced right-click context menu.
fn tab_view(
    cx: TabStripCx,
    id: TabId,
    run: impl Fn(CommandId) + Copy + 'static,
) -> impl IntoElement {
    let TabStripCx {
        state,
        panes,
        home_open,
        editing,
        workspaces,
        drag,
    } = cx;
    let tab = computed(move || state.tabs.with(|tabs| tabs.get(id).cloned()));
    let tab_style = move || state.live.read(|s| TabStyle::parse(s.text(TAB_STYLE)));
    let min_tab_px = move || state.live.read(min_width_px);
    let tab_h = move || state.live.read(tab_height_px);
    let tab_font_px = move || state.live.read(tab_font_size_px);
    let close_pressed = signal(false);
    let is_dragging = move || drag.with(|active| active.is_some_and(|d| d.tab == id));
    cleanup(move || {
        if drag.get_untracked().is_some_and(|active| active.tab == id) {
            drag.set(None);
        }
    });
    let shown = move || {
        !home_open.get() && state.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) == Some(id)
    };
    let multi_selected = move || state.is_selected(id);
    let pinned = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.pinned));
    let missing = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.missing));
    let dirty = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.dirty));
    let title = move || tab.with(|tab| tab.as_ref().map(Tab::title).unwrap_or_default());
    let has_group = move || tab.with(|tab| tab.as_ref().is_some_and(|tab| tab.group.is_some()));
    let tab_color = move || tab.with(|tab| tab.as_ref().and_then(|tab| tab.color));
    let has_custom_color = move || tab_color().is_some();
    let custom_fill = move || tab_color().map(|color| swatch(color).0);
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
    let chrome_top_accent = move || {
        custom_fill()
            .map(AdaptiveColor::from)
            .or_else(mode_accent)
            .unwrap_or_else(|| Hsla::transparent_black().into())
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
        .h(move || {
            let h = tab_h();
            px(if tab_style() == TabStyle::Chrome {
                h + 2.0
            } else {
                h
            })
        })
        .style_when(
            move || tab_style() == TabStyle::Vivaldi,
            |s| {
                s.pl(px(10.0))
                    .pr(px(6.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(move || {
                        if let Some(fill) = custom_fill() {
                            fill.into()
                        } else if shown() {
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
            move || tab_style() == TabStyle::Vivaldi && has_custom_color(),
            |s| {
                s.border_t(px(2.5))
                    .border_t_color(move || custom_fill().unwrap_or_else(Hsla::transparent_black))
            },
        )
        .style_when(
            move || tab_style() == TabStyle::Vivaldi && has_group(),
            |s| s.border_b(px(2.0)).border_b_color(underline),
        )
        .style_when(
            move || tab_style() == TabStyle::Chrome,
            |s| {
                s.pl_3()
                    .pr_1()
                    .rounded_t(px(8.0))
                    .border_t(px(2.5))
                    .border_t_color(chrome_top_accent)
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
        .style_when(is_dragging, |s| {
            s.border_1()
                .border_color(move || theme().primary)
                .shadow_md()
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
        .child(when(has_custom_color, move || {
            view()
                .w(px(7.0))
                .h(px(7.0))
                .rounded_full()
                .flex_shrink(0.0)
                .bg(move || custom_fill().unwrap_or_else(Hsla::transparent_black))
        }))
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
                .text_size(move || px(tab_font_px()))
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
            move || close_button(state, id, title, close_pressed),
        ));
    // Horizontal pointer drag claims touches once past `DRAG_START_THRESHOLD_PX`, keeping the
    // gesture out of the enclosing scroll view and cancelling normal click activation.
    let drag_host = view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .hover(|s| s.cursor(Cursor::Grab))
        .style_when(is_dragging, |s| s.cursor(Cursor::Grabbing).opacity(0.88))
        .claim_touches(TouchClaim::on_move().ask(move |request: &ClaimRequest| {
            if close_pressed.get_untracked() {
                return;
            }
            let mods = request.event.modifiers;
            if mods.ctrl || mods.cmd || mods.shift {
                return;
            }
            if request.event.gesture.delta.x.0.abs() >= DRAG_START_THRESHOLD_PX {
                request.claim();
            }
        }))
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| {
            let x = event.page.x.0;
            drag.set(Some(TabDrag {
                tab: id,
                pinned: pinned(),
                anchor_x: x,
            }));
            home_open.set(false);
            state.show(id);
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(mut active) = drag.get_untracked() else {
                return;
            };
            if active.tab != id {
                return;
            }
            let x = event.page.x.0;
            let stride = tab_slot_stride(active.pinned, min_tab_px(), tab_style());
            let (steps, consumed_dx) = compute_drag_steps(x - active.anchor_x, stride);
            if steps != 0 {
                if state.step_tab_by(active.tab, steps) {
                    active.anchor_x += consumed_dx;
                } else {
                    active.anchor_x = x;
                }
                drag.set(Some(active));
            }
        })
        .on_responder_release(move |_: &ResponderEvent| {
            if drag.get_untracked().is_some_and(|active| active.tab == id) {
                drag.set(None);
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| {
            if drag.get_untracked().is_some_and(|active| active.tab == id) {
                drag.set(None);
            }
        })
        .child(tab_view);
    // The middle button closes, as in browsers; Ctrl/Cmd+Left click and Shift+Left click multi-select
    // tabs for split/tiling (V.12); plain left-click clears any multi-selection.
    let trigger = view()
        .window_drag(false)
        .on_mouse_up(move |event: &MouseEvent| {
            if event.button == Some(MouseButton::Middle) && !pinned() {
                state.close(id);
            } else if event.button == Some(MouseButton::Left) && drag.get_untracked().is_none() {
                if event.modifiers.shift {
                    state.select_range_to(id);
                } else if event.modifiers.ctrl || event.modifiers.cmd {
                    state.ctrl_toggle_selected(id);
                } else if state.selected_count() > 0 {
                    state.clear_selected();
                }
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
        .child(drag_host);

    context_menu()
        .trigger(trigger)
        .on_open(move || state.set_menu_open(true))
        .on_close(move || state.set_menu_open(false))
        .popup_class("w-64 p-1 rounded-lg border border-border bg-popover shadow-lg")
        .child(when(dirty, move || {
            view()
                .child(tab_menu_item(t!("tabs.menu.save_existing"), move || {
                    home_open.set(false);
                    state.show(id);
                    editing.save_now();
                }))
                .child(tab_menu_item(t!("tabs.menu.save_new"), move || {
                    home_open.set(false);
                    state.show(id);
                    editing.save_as_copy();
                }))
                .child(tab_menu_item(t!("tabs.menu.discard_changes"), move || {
                    home_open.set(false);
                    state.show(id);
                    editing.discard_unsaved();
                }))
                .child(tab_menu_separator())
        }))
        .child(tab_menu_item(t!("tabs.menu.properties"), move || {
            state.open_properties(id);
        }))
        .child(
            tab_menu_item(t!("tabs.menu.show_in_folder"), move || {
                state.show_in_folder(id);
            })
            .disabled(missing),
        )
        .child(tab_menu_item(t!("tabs.menu.copy_path"), move || {
            state.copy_file_path(id);
        }))
        .child(open_in_split_submenu(panes, state, home_open, id))
        .child(
            tab_menu_item(t!("tabs.menu.open_new_window"), move || {
                home_open.set(false);
                state.show(id);
                run(CommandId::Pane(PaneAction::PopOut));
            })
            .disabled(missing),
        )
        .child(move_to_workspace_submenu(state, workspaces, id))
        .child(tab_menu_item_reactive(
            move || {
                if pinned() {
                    t!("tabs.menu.unpin")
                } else {
                    t!("tabs.menu.pin")
                }
            },
            move || state.toggle_pin(id),
        ))
        .child(tab_color_submenu(state, id, tab_color))
        .child(tab_menu_separator())
        .child(tab_menu_item(t!("tabs.menu.close"), move || {
            state.close(id);
        }))
        .child(
            tab_menu_item(t!("tabs.menu.close_others"), move || {
                state.close_others_of(id);
            })
            .disabled(move || !state.can_close_others(id)),
        )
        .child(
            tab_menu_item(t!("tabs.menu.close_right"), move || {
                state.close_right_of(id);
            })
            .disabled(move || !state.can_close_right(id)),
        )
        .child(
            tab_menu_item(t!("tabs.menu.close_left"), move || {
                state.close_left_of(id);
            })
            .disabled(move || !state.can_close_left(id)),
        )
        .child(tab_menu_item(t!("tabs.menu.close_all"), move || {
            state.close_all();
        }))
        .child(tab_menu_separator())
        .child(tab_menu_item(t!("tabs.menu.save_group"), move || {
            state.save_tab_group();
        }))
        .child(
            tab_menu_item(t!("tabs.menu.restore_group"), move || {
                state.restore_tab_group();
            })
            .disabled(move || !state.has_saved_tab_group()),
        )
}

/// A horizontal separator line inside the tab context menu.
fn tab_menu_separator() -> impl IntoElement {
    menu_separator()
        .h(px(1.0))
        .my(px(4.0))
        .bg(move || theme().border)
}

/// A plain item in the tab context menu.
fn tab_menu_item(label: String, select: impl Fn() + 'static) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(label.clone())
        .flex_row()
        .items_center()
        .gap_3()
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

/// A tab context menu item whose label updates reactively (e.g., Pin Tab / Unpin Tab).
fn tab_menu_item_reactive(
    label: impl Fn() -> String + Copy + 'static,
    select: impl Fn() + 'static,
) -> MenuItemElement {
    menu_item()
        .on_select(select)
        .accessibility_label(move || Some(SharedString::from(label())))
        .flex_row()
        .items_center()
        .gap_3()
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

/// Direction of the softly rounded submenu triangle indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmenuArrow {
    Right,
    Left,
}

/// A trigger or back row inside a tab context menu submenu.
fn submenu_row(label: String, arrow: SubmenuArrow) -> impl IntoElement {
    submenu_row_reactive(move || label.clone(), arrow)
}

/// A trigger or back row inside a tab context menu submenu with a reactive label.
fn submenu_row_reactive(
    label: impl Fn() -> String + 'static,
    arrow: SubmenuArrow,
) -> impl IntoElement {
    let icon = match arrow {
        SubmenuArrow::Right => crate::shell::app_menu::submenu_arrow_right().into_element(),
        SubmenuArrow::Left => crate::shell::app_menu::submenu_arrow_left().into_element(),
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(
            text(label)
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(icon)
}

/// The "Open in Split" submenu for opening a single right-clicked tab or multiple selected tabs
/// side-by-side (vertical split) or top-and-bottom (horizontal split).
fn open_in_split_submenu(
    panes: Panes,
    state: TabState,
    home_open: Signal<bool>,
    id: TabId,
) -> SubmenuElement {
    let trigger_label = move || {
        let count = state.selected_count();
        if state.is_selected(id) && count >= 2 {
            t!("tabs.menu.open_selected_in_split", count = count)
        } else {
            t!("tabs.menu.open_in_split")
        }
    };
    submenu()
        .offset(px(2.0))
        .trigger(submenu_row_reactive(trigger_label, SubmenuArrow::Right))
        .back(submenu_row(
            t!("tabs.menu.open_in_split_back"),
            SubmenuArrow::Left,
        ))
        .child(tab_menu_item(t!("tabs.menu.split_vertically"), move || {
            home_open.set(false);
            panes.open_tabs_in_split(id, Side::Right);
        }))
        .child(tab_menu_item(
            t!("tabs.menu.split_horizontally"),
            move || {
                home_open.set(false);
                panes.open_tabs_in_split(id, Side::Down);
            },
        ))
        .child(when(
            move || panes.is_split(),
            move || {
                view().child(tab_menu_separator()).child(tab_menu_item(
                    t!("tabs.menu.untile_panes"),
                    move || {
                        panes.run(PaneAction::Untile);
                    },
                ))
            },
        ))
}

/// The "Move to Workspace" submenu listing every other workspace plus "Move to New Workspace".
fn move_to_workspace_submenu(state: TabState, workspaces: Workspaces, id: TabId) -> SubmenuElement {
    let has_other_workspaces = move || {
        let shown = workspaces.shown_id();
        workspaces.list().iter().any(|w| w.id != shown)
    };
    submenu()
        .offset(px(2.0))
        .trigger(submenu_row(
            t!("tabs.menu.move_to_workspace"),
            SubmenuArrow::Right,
        ))
        .back(submenu_row(
            t!("tabs.menu.move_to_workspace_back"),
            SubmenuArrow::Left,
        ))
        .child(each(
            move || {
                let shown = workspaces.shown_id();
                workspaces
                    .list()
                    .into_iter()
                    .filter(move |w| w.id != shown)
                    .collect::<Vec<_>>()
            },
            |w: &Workspace| w.id,
            move |ws, _| {
                let target_id = ws.with_untracked(|w| w.id);
                let label = move || ws.with(crate::workspaces::display_name);
                let icon = move || ws.with(|w| w.icon.clone());
                menu_item()
                    .on_select(move || {
                        let _ = workspaces.move_tab_to(state, id, target_id);
                    })
                    .accessibility_label(move || Some(SharedString::from(label())))
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h(px(30.0))
                    .px_2p5()
                    .rounded(px(6.0))
                    .hover(|s| s.bg(move || theme().accent))
                    .focus(|s| s.bg(move || theme().accent))
                    .child(crate::workspaces::color_dot(move || {
                        ws.with(|w| w.color.clone())
                    }))
                    .child(when(
                        move || icon().is_some(),
                        move || text(move || icon().unwrap_or_default()).text_size(px(13.0)),
                    ))
                    .child(
                        text(label)
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(move || theme().popover_foreground),
                    )
            },
        ))
        .child(when(
            move || !has_other_workspaces(),
            move || tab_menu_item(t!("tabs.menu.no_other_workspaces"), || {}).disabled(true),
        ))
        .child(tab_menu_separator())
        .child(tab_menu_item(
            t!("tabs.menu.move_to_new_workspace"),
            move || {
                let _ = workspaces.move_tab_to_new_workspace(state, id);
            },
        ))
}

/// The "Change Tab Color" submenu with Default + every [`GroupColor`] swatch.
fn tab_color_submenu(
    state: TabState,
    id: TabId,
    current: impl Fn() -> Option<GroupColor> + Copy + 'static,
) -> SubmenuElement {
    let base = submenu()
        .offset(px(2.0))
        .trigger(submenu_row(
            t!("tabs.menu.change_color"),
            SubmenuArrow::Right,
        ))
        .back(submenu_row(
            t!("tabs.menu.change_color_back"),
            SubmenuArrow::Left,
        ));
    Some(None)
        .into_iter()
        .chain(GroupColor::ALL.into_iter().map(Some))
        .fold(base, |menu, choice| {
            menu.child(tab_color_swatch_item(
                choice,
                move || current() == choice,
                move || state.set_color(id, choice),
            ))
        })
}

/// One colour swatch row inside the "Change Tab Color" submenu.
fn tab_color_swatch_item(
    choice: Option<GroupColor>,
    selected: impl Fn() -> bool + Copy + 'static,
    select: impl Fn() + 'static,
) -> MenuItemElement {
    let label = match choice {
        Some(color) => color_name(color),
        None => t!("tabs.menu.color_default"),
    };
    let dot = view()
        .w(px(12.0))
        .h(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().border);
    let dot = match choice {
        Some(color) => {
            let fill = swatch(color).0;
            dot.bg(move || fill)
        }
        None => dot,
    };
    menu_item()
        .on_select(select)
        .accessibility_label(label.clone())
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(30.0))
        .px_2p5()
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| s.bg(move || theme().accent))
        .child(dot)
        .child(
            text(label)
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(move || if selected() { "✓" } else { "" })
                .text_size(px(12.0))
                .text_color(move || theme().primary),
        )
}

/// A tab's close button: circular background turns red with a white `×` on hover.
fn close_button(
    state: TabState,
    id: TabId,
    title: impl Fn() -> String + Copy + 'static,
    close_pressed: Signal<bool>,
) -> impl IntoElement {
    let hovered = signal(false);
    button()
        .accessibility_label(move || Some(SharedString::from(t!("tabs.close", name = title()))))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press_down(move || close_pressed.set(true))
        .on_press_up(move || close_pressed.set(false))
        .on_press(move || state.close(id))
        .cursor(Cursor::Pointer)
        .w(px(20.0))
        .h(px(20.0))
        .flex_shrink(0.0)
        .rounded_full()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(crate::shell::app_menu::close_hover_red))
        .active(|s| s.opacity(0.82))
        .child(crate::shell::app_menu::chrome_icon(
            crate::shell::app_menu::ChromeIcon::CloseX,
            11.0,
            move || {
                if hovered.get() {
                    crate::shell::app_menu::close_hover_fg()
                } else {
                    theme().muted_foreground
                }
            },
        ))
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
    view().window_drag(false).flex_shrink(0.0).child(
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
    view().window_drag(false).flex_shrink(0.0).child(
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

    #[test]
    fn compute_drag_steps_stays_zero_below_threshold_and_for_invalid_inputs() {
        assert_eq!(compute_drag_steps(0.0, 100.0), (0, 0.0));
        assert_eq!(compute_drag_steps(54.9, 100.0), (0, 0.0));
        assert_eq!(compute_drag_steps(-54.9, 100.0), (0, 0.0));
        assert_eq!(compute_drag_steps(80.0, 0.0), (0, 0.0));
        assert_eq!(compute_drag_steps(80.0, -50.0), (0, 0.0));
        assert_eq!(compute_drag_steps(f32::NAN, 100.0), (0, 0.0));
        assert_eq!(compute_drag_steps(f32::INFINITY, 100.0), (0, 0.0));
        assert_eq!(compute_drag_steps(80.0, f32::NAN), (0, 0.0));
    }

    #[test]
    fn compute_drag_steps_steps_single_and_multiple_slots_both_directions() {
        assert_eq!(compute_drag_steps(55.0, 100.0), (1, 100.0));
        assert_eq!(compute_drag_steps(154.9, 100.0), (1, 100.0));
        assert_eq!(compute_drag_steps(155.0, 100.0), (2, 200.0));
        assert_eq!(compute_drag_steps(-55.0, 100.0), (-1, -100.0));
        assert_eq!(compute_drag_steps(-154.9, 100.0), (-1, -100.0));
        assert_eq!(compute_drag_steps(-260.0, 100.0), (-3, -300.0));
    }

    #[test]
    fn compute_drag_steps_has_hysteresis_without_ping_pong_oscillation() {
        let stride = 254.0;
        let mut anchor = 100.0;
        let trigger_x = anchor + stride * DRAG_STEP_THRESHOLD_RATIO + 0.5;
        let (steps, consumed) = compute_drag_steps(trigger_x - anchor, stride);
        assert_eq!(steps, 1);
        anchor += consumed;

        // Holding steady or wiggling slightly at the same pointer coordinate never oscillates back.
        for jitter in [-20.0_f32, -5.0, 0.0, 5.0, 20.0] {
            let (next_steps, _) = compute_drag_steps((trigger_x + jitter) - anchor, stride);
            assert_eq!(next_steps, 0, "unexpected oscillation at jitter {jitter}");
        }

        // Reversing past the hysteresis threshold cleanly steps back left by 1.
        let back_x = anchor - stride * DRAG_STEP_THRESHOLD_RATIO - 0.5;
        let (back_steps, back_consumed) = compute_drag_steps(back_x - anchor, stride);
        assert_eq!(back_steps, -1);
        assert!((back_consumed + stride).abs() < f32::EPSILON);
    }

    #[test]
    fn tab_slot_stride_matches_pinned_unpinned_and_tab_style() {
        assert!(
            (tab_slot_stride(true, 250.0, TabStyle::Vivaldi) - (PINNED_TAB_WIDTH + 4.0)).abs()
                < f32::EPSILON
        );
        assert!(
            (tab_slot_stride(true, 250.0, TabStyle::Chrome) - (PINNED_TAB_WIDTH + 2.0)).abs()
                < f32::EPSILON
        );
        assert!((tab_slot_stride(false, 250.0, TabStyle::Vivaldi) - 254.0).abs() < f32::EPSILON);
        assert!((tab_slot_stride(false, 250.0, TabStyle::Chrome) - 252.0).abs() < f32::EPSILON);
    }
}
