//! The look of the app's quick dialogs: the command palette (K.16), the theme pickers (K.17) and the go-to box (R.6).
//! One look for all, so they read as one family of tools: a popover card near the top of the window, its field on
//! top, rows that show the highlight in the accent colour.

use nsh_base::{CommandElement, dialog, theme};
use nsh_core::prelude::*;

/// `list` (a `command()` with its items and handlers) styled as the app's list.
pub(crate) fn style(list: CommandElement) -> CommandElement {
    list.search_bar_class("flex-row items-center h-12 px-4 border-b border-border")
        .search_class("flex-1 text-base")
        .list_class("max-h-96 py-2")
        .group_class("px-4 pt-3 pb-1 text-xs text-muted-foreground")
        .item_class(
            "mx-2 px-3 py-2 rounded-md text-sm text-popover-foreground \
             focus:bg-accent focus:text-accent-foreground checked:font-medium",
        )
        .w(px(560.0))
        .rounded(CARD_RADIUS)
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg()
}

/// The quick dialogs' corner radius.
const CARD_RADIUS: nsh_core::geometry::Points = px(12.0);

/// `content` on the card the lists are drawn on, for a quick dialog that isn't a list.
pub(crate) fn card(content: View) -> View {
    content
        .rounded(CARD_RADIUS)
        .overflow_hidden()
        .bg(move || theme().popover)
        .border_1()
        .border_color(move || theme().border)
        .shadow_lg()
}

/// A dialog holding `list` (or another card) near the top of the window, labelled `label` for screen readers. Escape
/// or a press outside it calls `cancel`. It draws no scrim: what it changes (a theme) shows around it.
pub(crate) fn list_dialog(
    label: String,
    cancel: impl Fn() + Copy + 'static,
    list: impl IntoElement,
) -> impl IntoElement {
    // Since NSH 0.7 a focused text field keeps Escape from the dialog's own cancel: bind it in text fields too.
    let in_fields = KeyOptions {
        when: nsh_core::keymap::When::Always,
        ..KeyOptions::default()
    };
    on_key_with("escape", in_fields, move |_| cancel());
    dialog()
        .label(label)
        .animation(ModalAnimation::Fade)
        .on_cancel(cancel)
        .child(view().flex_1().items_center().pt(px(72.0)).child(list))
}
