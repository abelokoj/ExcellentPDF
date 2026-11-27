//! Page labels and go to page (plan R.6), the view: the reader's page in the title bar ("iv (4 of 210)"), which opens
//! the go-to box, and the box itself. The model (`ecclesia_features::read::goto`) finds the page; the page canvas
//! reports the pages and the page shown, and jumps.

use ecclesia_features::read::goto::Pages;
use nsh_base::{button, input, theme};
use nsh_core::JsonValue;
use nsh_core::element::SharedString;
use nsh_core::prelude::*;

use crate::list_dialog::{card, list_dialog};

/// What the canvas reported: the document's pages and the reader's page.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageState {
    pages: Signal<Pages>,
    current: Signal<usize>,
    /// Each page's size as laid out, in points (for the thumbnails' frames).
    sizes: Signal<Vec<(f32, f32)>>,
}

impl PageState {
    pub(crate) fn new() -> Self {
        Self {
            pages: signal(Pages::default()),
            current: signal(0),
            sizes: signal(Vec::new()),
        }
    }

    /// Takes the canvas's `pages` and `page` events.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        match &*event.name {
            "pages" => {
                let count = event.data.get("count").and_then(JsonValue::as_u64);
                let count = count.and_then(|count| usize::try_from(count).ok());
                let labels = event.data.get("labels").and_then(JsonValue::as_array);
                let labels = labels.map(|labels| {
                    let label = |value: &JsonValue| value.as_str().unwrap_or_default().to_owned();
                    labels.iter().map(label).collect()
                });
                let sizes = event.data.get("sizes").and_then(JsonValue::as_array);
                let sizes =
                    sizes.map_or_else(Vec::new, |sizes| sizes.iter().map(size_of).collect());
                self.sizes.set(sizes);
                self.pages.set(Pages::new(count.unwrap_or(0), labels));
                self.current.set(0);
            }
            "page" => {
                let index = event.data.get("index").and_then(JsonValue::as_u64);
                if let Some(index) = index.and_then(|index| usize::try_from(index).ok()) {
                    self.current.set(index);
                }
            }
            _ => {}
        }
    }

    /// How many pages the document has.
    pub(crate) fn count(self) -> usize {
        self.pages.with(Pages::count)
    }

    /// The reader's page.
    pub(crate) fn current(self) -> usize {
        self.current.get()
    }

    /// Page `index`'s label ("iv", or its number).
    pub(crate) fn label(self, index: usize) -> String {
        self.pages.with(|pages| pages.label(index))
    }

    /// Page `index`'s size in points, or zero while it isn't reported.
    pub(crate) fn size(self, index: usize) -> (f32, f32) {
        self.sizes
            .with(|sizes| sizes.get(index).copied().unwrap_or_default())
    }

    /// Every page's height as laid out, in points, untracked (V.8: how far through the document a place is).
    pub(crate) fn heights(self) -> Vec<f32> {
        self.sizes
            .with_untracked(|sizes| sizes.iter().map(|(_, height)| *height).collect())
    }

    /// Whether a document with pages is shown.
    fn has_pages(self) -> bool {
        self.pages.with(|pages| pages.count() > 0)
    }

    /// The reader's page as the title bar shows it: "iv (4 of 210)", or "4 of 210" when the label is the number.
    fn status(self) -> String {
        self.describe(self.current.get())
    }

    /// Page `index` as the reader reads it, as [`Self::status`] shows the reader's page.
    pub(crate) fn describe(self, index: usize) -> String {
        self.pages.with(|pages| describe(pages, index))
    }
}

/// A `[width, height]` the canvas reported; zero for anything else.
fn size_of(value: &JsonValue) -> (f32, f32) {
    let number = |at: usize| value.get(at).and_then(JsonValue::as_f64).unwrap_or(0.0) as f32;
    (number(0), number(1))
}

/// Page `index` of `pages`, as the reader reads it.
fn describe(pages: &Pages, index: usize) -> String {
    let (number, count) = (index + 1, pages.count());
    if pages.label_is_number(index) {
        t!("page.status", number = number, count = count)
    } else {
        let label = pages.label(index);
        t!(
            "page.status_labelled",
            label = label,
            number = number,
            count = count
        )
    }
}

/// The reader's page in the title bar; pressing it opens the go-to box. Shown only while a document has pages.
pub(crate) fn page_button(state: PageState, open: impl Fn() + Copy + 'static) -> impl IntoElement {
    when(
        move || state.has_pages(),
        move || {
            // Presses on the button must not drag the window.
            view().window_drag(false).child(
                button()
                    .accessibility_label(move || {
                        Some(SharedString::from(t!(
                            "page.button",
                            status = state.status()
                        )))
                    })
                    .on_press(open)
                    .h(px(26.0))
                    .px_2()
                    .rounded(px(6.0))
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(move || theme().sidebar_accent))
                    .active(|s| s.opacity(0.7))
                    .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                    .child(
                        text(move || state.status())
                            .text_size(px(12.0))
                            .tabular_nums()
                            .text_color(move || theme().sidebar_foreground),
                    ),
            )
        },
    )
}

/// The go-to box (Ctrl+G): a page number or label, checked as it is typed; Enter goes, Escape closes. `go` gets the
/// page's index.
pub(crate) fn goto_dialog(
    open: Signal<bool>,
    state: PageState,
    go: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let typed = signal(String::new());
    let close = move || open.set(false);
    let target = move || typed.with(|input| state.pages.with(|pages| pages.find(input)));
    let feedback = move || {
        typed.with(|input| {
            state.pages.with(|pages| match pages.find(input) {
                _ if input.trim().is_empty() => Feedback::Hint(t!("goto.hint")),
                Some(index) => Feedback::Target(t!("goto.target", page = describe(pages, index))),
                None => Feedback::Missing(t!("goto.missing", input = input.trim())),
            })
        })
    };
    let field = view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(48.0))
        .px_4()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            input()
                .value(move || typed.get())
                .on_change(move |text: &str| typed.set(text.to_owned()))
                .on_submit(move |_: &str| {
                    if let Some(index) = target() {
                        close();
                        go(index);
                    }
                })
                .placeholder(t!("goto.placeholder"))
                .label(t!("goto.title"))
                .auto_focus(true)
                .flex_1()
                .text_size(px(16.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(move || t!("goto.of", count = state.pages.with(Pages::count)))
                .text_size(px(13.0))
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        );
    let message = text(move || feedback().text())
        .px_4()
        .py_2()
        .text_size(px(12.0))
        .text_color(move || match feedback() {
            Feedback::Missing(_) => theme().destructive,
            Feedback::Hint(_) | Feedback::Target(_) => theme().muted_foreground,
        });
    list_dialog(
        t!("goto.title"),
        close,
        card(view().child(field).child(message)).w(px(400.0)),
    )
}

/// The line under the go-to field.
#[derive(Debug, Clone, PartialEq)]
enum Feedback {
    /// Nothing typed: how to use the box.
    Hint(String),
    /// Where Enter goes.
    Target(String),
    /// No page by that name: Enter does nothing.
    Missing(String),
}

impl Feedback {
    fn text(self) -> String {
        match self {
            Self::Hint(text) | Self::Target(text) | Self::Missing(text) => text,
        }
    }
}

/// Minimum allowed custom zoom percentage.
const MIN_CUSTOM_ZOOM_PCT: f32 = 10.0;

/// Maximum allowed custom zoom percentage.
const MAX_CUSTOM_ZOOM_PCT: f32 = 6400.0;

/// Quick preset zoom percentages shown inside [`custom_zoom_dialog`].
const ZOOM_PRESETS: [f32; 7] = [50.0, 75.0, 100.0, 125.0, 150.0, 200.0, 300.0];

/// Parses a custom zoom percentage (e.g. `"125"` or `"125%"`) within `10.0..=6400.0`.
fn parse_zoom_percent(raw: &str) -> Option<f32> {
    let trimmed = raw.trim().trim_end_matches('%').trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: f32 = trimmed.parse().ok()?;
    (value.is_finite() && (MIN_CUSTOM_ZOOM_PCT..=MAX_CUSTOM_ZOOM_PCT).contains(&value))
        .then_some(value)
}

/// Formats a zoom percentage without trailing `.0` for whole numbers.
fn format_zoom_percent(percent: f32) -> String {
    if percent.fract().abs() < 0.01 {
        format!("{percent:.0}")
    } else {
        format!("{percent:.1}")
    }
}

/// The custom zoom dialog (`Ctrl+Y`): enter a zoom percentage (`10–6400%`) or pick a quick preset;
/// Enter applies and closes, Escape closes.
pub(crate) fn custom_zoom_dialog(
    open: Signal<bool>,
    apply: impl Fn(f32) + Copy + 'static,
) -> impl IntoElement {
    let typed = signal(String::new());
    let close = move || open.set(false);
    let parsed = move || typed.with(|raw| parse_zoom_percent(raw));
    let feedback = move || {
        typed.with(|raw| {
            if raw.trim().is_empty() {
                Feedback::Hint(t!("zoom.custom.hint"))
            } else if let Some(pct) = parse_zoom_percent(raw) {
                Feedback::Target(t!("zoom.custom.target", percent = format_zoom_percent(pct)))
            } else {
                Feedback::Missing(t!("zoom.custom.invalid"))
            }
        })
    };
    let field = view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(48.0))
        .px_4()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            input()
                .value(move || typed.get())
                .on_change(move |text: &str| typed.set(text.to_owned()))
                .on_submit(move |_: &str| {
                    if let Some(pct) = parsed() {
                        close();
                        apply(pct);
                    }
                })
                .placeholder(t!("zoom.custom.placeholder"))
                .label(t!("zoom.custom.title"))
                .auto_focus(true)
                .flex_1()
                .text_size(px(16.0))
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text("%")
                .text_size(px(14.0))
                .text_color(move || theme().muted_foreground),
        );
    let mut presets = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .px_4()
        .pt_3()
        .pb_1();
    for preset in ZOOM_PRESETS {
        let label = format!("{preset:.0}%");
        presets = presets.child(
            button()
                .accessibility_label(label.clone())
                .on_press(move || {
                    close();
                    apply(preset);
                })
                .h(px(28.0))
                .px_2p5()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().secondary)
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().accent))
                .active(|s| s.opacity(0.75))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(
                    text(label)
                        .text_size(px(12.0))
                        .tabular_nums()
                        .text_color(move || theme().secondary_foreground),
                ),
        );
    }
    let message = text(move || feedback().text())
        .px_4()
        .py_2()
        .text_size(px(12.0))
        .text_color(move || match feedback() {
            Feedback::Missing(_) => theme().destructive,
            Feedback::Hint(_) | Feedback::Target(_) => theme().muted_foreground,
        });
    list_dialog(
        t!("zoom.custom.title"),
        close,
        card(view().child(field).child(presets).child(message)).w(px(420.0)),
    )
}

/// The rename file dialog (`F2`): pre-filled with the active document's file name; Enter renames
/// the file on disk and updates the tab, Escape closes.
pub(crate) fn rename_dialog(open: Signal<bool>, tabs: crate::tabs::TabState) -> impl IntoElement {
    let initial = untracked(|| tabs.shown_name()).unwrap_or_default();
    let typed = signal(initial);
    let error = signal(None::<String>);
    let close = move || open.set(false);
    let field = view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(48.0))
        .px_4()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            input()
                .value(move || typed.get())
                .on_change(move |text: &str| {
                    typed.set(text.to_owned());
                    if untracked(|| error.with(Option::is_some)) {
                        error.set(None);
                    }
                })
                .on_submit(move |_: &str| {
                    let candidate = untracked(|| typed.get());
                    let trimmed = candidate.trim();
                    if trimmed.is_empty() {
                        error.set(Some(t!("rename.empty")));
                        return;
                    }
                    match tabs.rename_shown_file(trimmed) {
                        Ok(_) => close(),
                        Err(message) => error.set(Some(message)),
                    }
                })
                .placeholder(t!("rename.placeholder"))
                .label(t!("rename.title"))
                .auto_focus(true)
                .flex_1()
                .text_size(px(15.0))
                .text_color(move || theme().popover_foreground),
        );
    let message = text(move || error.get().unwrap_or_else(|| t!("rename.hint")))
        .px_4()
        .py_2()
        .text_size(px(12.0))
        .text_color(move || {
            if error.with(Option::is_some) {
                theme().destructive
            } else {
                theme().muted_foreground
            }
        });
    list_dialog(
        t!("rename.title"),
        close,
        card(view().child(field).child(message)).w(px(440.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_zoom_parser_validates_range_and_percent_suffix() {
        assert_eq!(parse_zoom_percent("125"), Some(125.0));
        assert_eq!(parse_zoom_percent(" 125% "), Some(125.0));
        assert_eq!(parse_zoom_percent("10"), Some(10.0));
        assert_eq!(parse_zoom_percent("6400%"), Some(6400.0));
        assert_eq!(parse_zoom_percent("9.9"), None);
        assert_eq!(parse_zoom_percent("6401"), None);
        assert_eq!(parse_zoom_percent(""), None);
        assert_eq!(parse_zoom_percent("abc"), None);
        assert_eq!(parse_zoom_percent("NaN"), None);
    }
}
