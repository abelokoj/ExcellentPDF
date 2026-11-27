//! The find bar over a pane's pages, and the sidebar's Results tab (F.1). The page canvas is a window over the app,
//! so the bar is a row of the document panel above it, not a box drawn over the pages.

use nsh_base::{button, input, theme, toggle};
use nsh_core::prelude::*;

use super::{FindAction, FindState, Hit, Toggle, status};
use crate::commands::{self, CommandId};
use crate::goto::PageState;
use crate::panes::Place;

/// The result list shows this many rows at most; the bar's count and F3 still reach every hit.
///
/// shortcut: a capped list instead of a virtualized one; upgrade when NSH's list virtualizes rows.
const LIST_CAP: usize = 500;

/// The bar: the query, its three options, where the search is, and previous, next and close.
pub(crate) fn find_bar(state: FindState) -> impl IntoElement {
    let focused = signal(false);
    let holder = element_ref();
    holder.on_focus_in(move || focused.set(true));
    holder.on_focus_out(move || focused.set(false));
    cleanup(move || focused.set(false));
    // Its own keys, only while it has the keyboard: the pages keep theirs (Escape leaves their modes).
    effect(move || {
        if !focused.get() {
            return;
        }
        on_key("escape", move |_: &str| state.run(FindAction::Close));
        on_key("shift+enter", move |_: &str| {
            state.run(FindAction::Previous)
        });
    });
    let field = input()
        .element_ref(&state.input())
        .value(move || state.query())
        .on_change(move |text: &str| state.set_query(text))
        .on_submit(move |_: &str| state.run(FindAction::Next))
        .placeholder(t!("find.placeholder"))
        .label(t!("find.placeholder"))
        .flex_1()
        .max_w(px(320.0))
        .h(px(28.0))
        .px_3()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .bg(move || theme().background)
        .text_size(px(13.0))
        .text_color(move || theme().foreground);
    let toggles = Toggle::ALL
        .into_iter()
        .fold(view().flex_row().gap(px(2.0)), |row, choice| {
            row.child(option_toggle(state, choice))
        });
    let found = move || status(&state.query(), state.progress());
    view()
        .element_ref(&holder)
        .label(t!("find.bar"))
        .h(px(40.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(field)
        .child(toggles)
        .child(
            text(found)
                .min_w(px(88.0))
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
        .child(view().flex_1())
        .child(bar_button(
            "find.previous",
            "↑",
            CommandId::Find(FindAction::Previous),
            move || state.run(FindAction::Previous),
        ))
        .child(bar_button(
            "find.next",
            "↓",
            CommandId::Find(FindAction::Next),
            move || state.run(FindAction::Next),
        ))
        .child(crate::shell::app_menu::close_x_button(
            format!(
                "{} ({})",
                t!("find.close"),
                commands::label_with_shortcut(CommandId::Find(FindAction::Close))
            ),
            28.0,
            12.0,
            6.0,
            move || state.run(FindAction::Close),
        ))
}

/// The glyph a toggle shows, and the localization key of its name.
fn toggle_face(choice: Toggle) -> (&'static str, &'static str) {
    match choice {
        Toggle::Case => ("Aa", "find.match_case"),
        Toggle::Words => ("ab|", "find.whole_word"),
        Toggle::Marks => ("é", "find.match_accents"),
    }
}

/// One option: pressed while it is on.
fn option_toggle(state: FindState, choice: Toggle) -> impl IntoElement {
    let (glyph, name) = toggle_face(choice);
    let on = move || choice.is_on(state.options());
    toggle()
        .pressed(on)
        .accessibility_label(t!(name))
        .on_change(move |pressed: &bool| state.set_toggle(choice, *pressed))
        .h(px(28.0))
        .min_w(px(30.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().sidebar)
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .pressed_style(|s| {
            s.bg(move || theme().background)
                .border_color(move || theme().primary)
        })
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(glyph)
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if on() {
                        theme().primary
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

/// Previous, next or close: a glyph, named with its shortcut for screen readers.
fn bar_button(
    name: &'static str,
    glyph: &'static str,
    command: CommandId,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(move || {
            let keys = commands::shortcut_label(command);
            Some(SharedString::from(if keys.is_empty() {
                t!(name)
            } else {
                t!("find.with_keys", name = t!(name), keys = keys)
            }))
        })
        .on_press(press)
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(glyph)
                .text_size(px(14.0))
                .text_color(move || theme().foreground),
        )
}

/// The Results tab: every hit with the words round it, in document order. A press goes to it.
/// The find results (F.1); `elsewhere` shows a hit's page in the other pane (V.9).
pub(crate) fn results_panel(
    state: FindState,
    pages: PageState,
    elsewhere: impl Fn(Place) + Copy + 'static,
) -> impl IntoElement {
    let searching = move || state.is_open() && !state.query().trim().is_empty();
    when(searching, move || {
        let header = move || status(&state.query(), state.progress());
        let shown = move || {
            let mut hits = state.hits();
            hits.truncate(LIST_CAP);
            hits
        };
        let more = move || state.hits().len().saturating_sub(LIST_CAP);
        view()
            .flex_1()
            .child(
                text(header)
                    .px_3()
                    .py_2()
                    .text_size(px(12.0))
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                scroll()
                    .flex_1()
                    .child(view().p_2().pt(px(0.0)).gap(px(2.0)).child(each(
                        shown,
                        |hit: &Hit| (hit.page, hit.nth),
                        move |hit, _| result_row(state, pages, hit, elsewhere),
                    )))
                    .child(when(
                        move || more() > 0,
                        move || {
                            text(move || t!("find.more", n = more()))
                                .px_3()
                                .pb_3()
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground)
                        },
                    )),
            )
    })
    .otherwise(move || {
        crate::shell::empty_message(
            || t!("panel.results"),
            || {
                t!(
                    "find.empty",
                    keys = commands::shortcut_label(CommandId::Find(FindAction::Open))
                )
            },
        )
    })
}

/// One hit: its page's label, and the hit in bold between the words round it.
fn result_row(
    state: FindState,
    pages: PageState,
    hit: ReadSignal<Hit>,
    elsewhere: impl Fn(Place) + Copy + 'static,
) -> impl IntoElement {
    let hit = hit.get_untracked();
    let key = (hit.page, hit.nth);
    let current = move || state.current_key() == Some(key);
    let page = move || pages.label(key.0);
    let spoken = format!("{}{}{}", hit.before, hit.hit, hit.after);
    let target = hit.clone();
    let row = button()
        .accessibility_label(move || {
            Some(SharedString::from(t!(
                "find.row",
                text = spoken.clone(),
                page = page()
            )))
        })
        .on_press(move || state.show(&target))
        .flex_row()
        .items_start()
        .gap_2()
        .px_2()
        .py(px(6.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .style_when(current, |s| s.bg(move || theme().sidebar_accent))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(hit.before)
                .flex_1()
                .number_of_lines(2u32)
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground)
                .child(
                    text(hit.hit)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().sidebar_foreground),
                )
                .child(text(hit.after)),
        )
        .child(
            text(page)
                .text_size(px(12.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        );
    // V.9: the middle button shows the hit's page in the other pane. The button takes only presses, which carry
    // no modifier keys, so the row around it listens.
    view()
        .on_mouse_up(move |event: &MouseEvent| {
            if event.button == Some(MouseButton::Middle) {
                elsewhere(Place {
                    page: key.0,
                    spot: None,
                });
            }
        })
        .child(row)
}
