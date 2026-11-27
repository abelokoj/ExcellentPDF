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
