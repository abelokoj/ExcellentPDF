//! The reading-theme bar (R.15): the reading themes as swatches in the title bar. The page canvas is a window over
//! the app's, so a dialog in the middle would hide the very page it should preview; the title bar lies outside it.
//! Moving through the swatches (the arrow keys, or a click) shows each theme on the open page at once; Enter or Keep
//! keeps it, Escape or Cancel goes back to the one in effect (as K.17's pickers do).

use excellent_core::darkmode::ReadingTheme;
use nsh_base::{button, radio, radio_group, theme};
use nsh_core::prelude::*;

use crate::reading::{Reading, hex};

/// The bar, in place of the palette button while `open`.
pub(crate) fn reading_theme_bar(open: Signal<bool>, reading: Reading) -> impl IntoElement {
    let kept = untracked(|| reading.reading_theme());
    let shown = signal(kept);
    // Pages in the document's own colours show the kept theme while the bar is open: the reader is choosing one.
    reading.preview(Some(kept));
    let close = move |keep: bool| {
        if keep {
            reading.set_reading_theme(shown.get_untracked());
        }
        reading.preview(None);
        open.set(false);
    };
    on_key("enter", move |_| close(true));
    on_key("escape", move |_| close(false));
    // Closed some other way (the window, a new document): the preview goes with it.
    cleanup(move || reading.preview(None));

    let first = element_ref();
    spawn(async move {
        // After a dialog that just closed (the palette, the settings) gives the focus back.
        next_frame().await;
        first.focus();
    });
    let swatches =
        ReadingTheme::ALL
            .into_iter()
            .fold(radio_group().flex_row().gap_1(), |group, choice| {
                let chip = swatch(choice, reading, shown);
                group.child(if choice == kept {
                    chip.element_ref(&first)
                } else {
                    chip
                })
            });
    view()
        .window_drag(false)
        .flex_row()
        .items_center()
        .gap_2()
        .label(t!("reading_theme.title"))
        .child(swatches)
        .child(bar_button("reading_theme.cancel", false, move || {
            close(false)
        }))
        .child(bar_button("reading_theme.keep", true, move || close(true)))
}

/// One theme: a small page in its paper with an "A" in its ink, and its name. Checking it previews it.
fn swatch(
    choice: ReadingTheme,
    reading: Reading,
    shown: Signal<ReadingTheme>,
) -> nsh_base::RadioElement {
    let palette = move || reading.palette(choice);
    let color = |rgb| Hsla::parse_hex(&hex(rgb)).expect("hex() writes #rrggbb");
    radio()
        .checked(move || shown.get() == choice)
        .accessibility_label(t!(label(choice)))
        .on_change(move |_: &bool| {
            shown.set(choice);
            reading.preview(Some(choice));
        })
        .h(px(28.0))
        .pl(px(4.0))
        .pr_2()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .checked_style(|s| s.bg(move || theme().background).shadow_sm())
        .focus_visible(|s| s.border_1().border_color(move || theme().ring))
        .child(
            view()
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || color(palette().paper))
                .items_center()
                .justify_center()
                .child(
                    text("A")
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || color(palette().ink)),
                ),
        )
        .child(
            text(move || t!(label(choice)))
                .text_size(px(12.0))
                .text_color(move || {
                    if shown.get() == choice {
                        theme().foreground
                    } else {
                        theme().sidebar_foreground
                    }
                }),
        )
}

/// Keep (`primary`) or Cancel.
fn bar_button(label: &'static str, primary: bool, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(t!(label))
        .on_press(press)
        .h(px(28.0))
        .px_3()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .bg(move || {
            if primary {
                theme().primary
            } else {
                theme().background
            }
        })
        .border_1()
        .border_color(move || {
            if primary {
                theme().primary
            } else {
                theme().border
            }
        })
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_color(move || theme().ring))
        .child(
            text(move || t!(label))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if primary {
                        theme().primary_foreground
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// The localization key of a reading theme's name.
pub(crate) fn label(choice: ReadingTheme) -> &'static str {
    match choice {
        ReadingTheme::App => "reading_theme.app",
        ReadingTheme::Light => "reading_theme.light",
        ReadingTheme::Sepia => "reading_theme.sepia",
        ReadingTheme::Dark => "reading_theme.dark",
        ReadingTheme::Oled => "reading_theme.oled",
        ReadingTheme::HighContrast => "reading_theme.contrast",
        ReadingTheme::Custom => "reading_theme.custom",
    }
}
