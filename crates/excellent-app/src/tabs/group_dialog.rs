//! A tab group's edit dialog (R.13): its name, its colour, and "Ungroup" and "Close group". Changes apply as they
//! are made, so the chip in the strip shows them behind the dialog; Enter, Escape or a press outside closes it.

use nsh_base::{button, input, theme, toggle};
use nsh_core::prelude::*;

use super::{GroupColor, GroupId, TabState};
use crate::list_dialog::{card, list_dialog};

/// The dialog of the group being edited ([`TabState::editing`]); it closes itself when the group goes.
pub(crate) fn group_dialog(state: TabState) -> impl IntoElement {
    let close = move || state.stop_editing();
    let id = untracked(|| state.editing());
    // The group can go while the dialog is open (its last tab closed by a shortcut): the dialog goes with it.
    effect(move || {
        if id.is_none_or(|id| state.group(id).is_none()) {
            close();
        }
    });
    let content = match id {
        Some(id) => form(state, id, close),
        None => view(),
    };
    list_dialog(t!("tab_group.title"), close, card(content))
}

/// The dialog's fields and buttons.
fn form(state: TabState, id: GroupId, close: impl Fn() + Copy + 'static) -> View {
    let typed = signal(
        untracked(|| state.group(id))
            .map(|group| group.name)
            .unwrap_or_default(),
    );
    let color = move || {
        state
            .group(id)
            .map_or(GroupColor::Grey, |group| group.color)
    };
    let name = input()
        .value(move || typed.get())
        .on_change(move |text: &str| {
            typed.set(text.to_owned());
            state.rename_group(id, text);
        })
        .on_submit(move |_: &str| close())
        .placeholder(t!("tab_group.name_placeholder"))
        .label(t!("tab_group.name"))
        .auto_focus(true)
        .h(px(36.0))
        .px_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || theme().input)
        .focus(|s| s.border_color(move || theme().ring))
        .text_size(px(14.0))
        .text_color(move || theme().popover_foreground);
    let swatches = GroupColor::ALL
        .into_iter()
        .fold(view().flex_row().gap_2(), |row, swatch| {
            row.child(swatch_button(state, id, swatch, move || color() == swatch))
        });
    view()
        .w(px(340.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("tab_group.title"))
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(name)
        .child(
            text(move || t!("tab_group.color"))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(swatches)
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .gap_2()
                .justify_end()
                .child(action_button("tab_group.ungroup", Tone::Plain, move || {
                    close();
                    state.ungroup(id);
                }))
                .child(action_button(
                    "tab_group.close",
                    Tone::Destructive,
                    move || {
                        close();
                        state.close_group(id);
                    },
                )),
        )
}

/// One colour to pick, named for screen readers; the chosen one is ringed.
fn swatch_button(
    state: TabState,
    id: GroupId,
    swatch: GroupColor,
    chosen: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    toggle()
        .pressed(chosen)
        .accessibility_label(color_name(swatch))
        .on_change(move |_: &bool| state.recolor_group(id, swatch))
        .size(px(28.0))
        .rounded_full()
        .bg(Hsla::parse_hex(swatch.swatch().0).expect("a built-in colour"))
        .border_2()
        .border_color(move || theme().popover)
        .pressed_style(|s| s.border_color(move || theme().foreground))
        .focus_visible(|s| s.border_color(move || theme().ring))
}

/// The name of `color`, as screen readers and the chip of a group without a name say it.
pub(super) fn color_name(color: GroupColor) -> String {
    t!(&format!("tab_group.color.{}", color.id()))
}

/// How a dialog button reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Plain,
    /// It loses work: closing tabs.
    Destructive,
}

fn action_button(key: &'static str, tone: Tone, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(t!(key))
        .on_press(on_press)
        .h(px(32.0))
        .px_3()
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(move || t!(key))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || match tone {
                    Tone::Plain => theme().popover_foreground,
                    Tone::Destructive => theme().destructive,
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_has_an_english_name() {
        for color in GroupColor::ALL {
            let key = format!("\"tab_group.color.{}\":", color.id());
            assert!(
                crate::l10n::EN.contains(&key),
                "{key} is missing from app/locales/en.json"
            );
        }
    }
}
