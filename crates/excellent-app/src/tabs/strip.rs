//! The tab strip (R.12, plan §10.1): the open files in the title bar, in NSH's tab list, so the arrow keys move
//! between them and screen readers hear one list. The shown tab takes the page's colour, as if the page came up into
//! it; a dot marks unsaved changes, a pin the pinned tabs (which keep no close button). A middle click closes a tab.
//! Then the open button and, at the right end, the button of the tab list.

use nsh_base::{button, tabs, theme};
use nsh_core::geometry::Radians;
use nsh_core::prelude::*;

use super::TabState;
use super::model::Tab;
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
    let items = move || state.tabs.with(|tabs| tabs.all().to_vec());
    let list = tabs()
        .flex_row()
        .items_end()
        .gap(px(2.0))
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .h_full()
        .child(each(
            items,
            |tab: &Tab| tab.id,
            move |tab, _| tab_view(state, tab),
        ));
    view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex_row()
        .items_center()
        .gap_1()
        .child(when(
            move || !state.has_shown(),
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

/// One tab: its file's name, a dot while it has unsaved changes, a pin or a close button.
fn tab_view(state: TabState, tab: ReadSignal<Tab>) -> impl IntoElement {
    let id = tab.with_untracked(|tab| tab.id);
    let shown = move || state.tabs.with(|tabs| tabs.active().map(|tab| tab.id)) == Some(id);
    let pinned = move || tab.with(|tab| tab.pinned);
    let title = move || tab.with(Tab::title);
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
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .selected_style(|s| s.bg(move || theme().background))
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(when(pinned, pin))
        .child(when(
            move || tab.with(|tab| tab.dirty),
            || {
                view()
                    .label(t!("tabs.unsaved"))
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(move || theme().primary)
            },
        ))
        .child(
            text(title)
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if shown() {
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
    // The middle button closes, as in browsers; the tab itself takes only presses.
    view()
        .window_drag(false)
        .on_mouse_up(move |event: &MouseEvent| {
            if event.button == Some(MouseButton::Middle) && !pinned() {
                state.close(id);
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
    id: super::TabId,
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

/// A pinned tab's mark: a small filled disc on a stem, drawn from views.
fn pin() -> impl IntoElement {
    view()
        .items_center()
        .flex_shrink(0.0)
        .accessibility_elements_hidden(true)
        .child(
            view()
                .w(px(7.0))
                .h(px(7.0))
                .rounded_full()
                .bg(move || theme().sidebar_foreground),
        )
        .child(
            view()
                .w(px(1.5))
                .h(px(5.0))
                .bg(move || theme().sidebar_foreground),
        )
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
