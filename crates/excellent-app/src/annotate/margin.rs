//! Margin notes (A.11): a column beside a pane's pages showing each note on the pages in view as a card, level with
//! what it is about and pushed apart from the others when they would overlap. A click on a card opens its note
//! there, to be written in place, and in the inspector (A.4); the canvas tells the column where its pages are as
//! they scroll and zoom (its `margin` events). Turned on and off by `annotate.margin_notes` and in the settings. The
//! model is `excellent-features`' `annotate::margin`.

use excellent_core::annot::page_annotations;
use excellent_features::annotate::margin::{self, Outline, ToScreen};
use excellent_features::annotate::notes::{self, Target};
use nsh_base::{button, input, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::AnnotateState;
use super::note::Noting;
use crate::bookmarks::field;
use crate::tabs::{Tone, action_button};

/// The setting's key: whether the margin column shows.
pub(crate) const MARGIN_NOTES: &str = "margin_notes";

/// The column's width, in the app's pixels.
const WIDTH: f32 = 280.0;
/// Between two cards, and above the first.
const GAP: f32 = 8.0;
/// A card's padding, its heading's height and its text's line height.
const PAD: f32 = 10.0;
const HEADING: f32 = 18.0;
const LINE: f32 = 18.0;
/// The lines of a note a closed card shows.
const LINES: usize = 4;
/// About how many characters of the cards' 13 px text fit on a line of a card.
const PER_LINE: usize = 34;
/// An open card's text field and the row of buttons under it.
const EDITOR: f32 = 120.0;
const BUTTONS: f32 = 40.0;

/// A note on a page in view, for its card.
#[derive(Debug, Clone, PartialEq)]
struct Card {
    noting: Noting,
    author: String,
    text: String,
    /// The outline of what the note is about.
    outline: Outline,
}

impl Card {
    fn target(&self) -> &Target {
        &self.noting.target
    }

    /// Its height, closed or `open` for writing.
    fn height(&self, open: bool) -> f32 {
        let body = if open {
            EDITOR + BUTTONS
        } else {
            margin::lines(&self.text, PER_LINE, LINES) as f32 * LINE
        };
        2.0 * PAD + HEADING + body
    }
}

impl AnnotateState {
    /// Whether the margin column shows (the setting).
    pub(crate) fn margin_shown(self) -> bool {
        self.live.read(|s| s.flag(MARGIN_NOTES)).unwrap_or(false)
    }

    /// The `annotate.margin_notes` command: shows the margin column, or hides it.
    pub(crate) fn toggle_margin(self) {
        let on = !untracked(|| self.margin_shown());
        self.live
            .change(|s| s.set_flag(MARGIN_NOTES, on.then_some(true)));
    }

    /// The canvas's `margin` prop: whether it reports its pages to the column.
    pub(crate) fn margin_prop(self) -> JsonValue {
        JsonValue::from(self.margin_shown())
    }

    /// The canvas's pages moved (`margin`: `{pages: [{page, m}]}`): the cards follow, and the notes are read again
    /// when other pages came into view.
    pub(super) fn margin_moved(self, data: &JsonValue) {
        let maps: Vec<(usize, ToScreen)> = data
            .get("pages")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let page = usize::try_from(entry.get("page")?.as_u64()?).ok()?;
                Some((page, ToScreen::from_json(entry.get("m")?)?))
            })
            .collect();
        let pages: Vec<usize> = maps.iter().map(|(page, _)| *page).collect();
        if untracked(|| self.shown_pages.with(|shown| *shown != pages)) {
            self.shown_pages.set(pages);
        }
        self.page_maps.set(maps);
    }

    /// The notes on `pages`, in page order, each page's in the order the page lists them.
    fn cards(self, pages: &[usize]) -> Vec<Card> {
        let Some(document) = self.editing.document() else {
            return Vec::new();
        };
        let Ok(shared) = document.mupdf() else {
            return Vec::new();
        };
        let mupdf = shared.lock().expect("document lock poisoned");
        pages
            .iter()
            .flat_map(|&page| {
                page_annotations(&mupdf, page)
                    .map_err(|err| {
                        log::warn!("margin: cannot read page {page}'s annotations: {err}")
                    })
                    .unwrap_or_default()
            })
            .filter(margin::has_card)
            .map(|annotation| Card {
                noting: Noting::of(&annotation),
                author: annotation.author.clone(),
                text: notes::normalize(&annotation.contents),
                outline: margin::outline(&annotation.shape),
            })
            .collect()
    }
}

/// The margin column beside a pane's pages, while the setting says so.
pub(crate) fn margin_column(state: AnnotateState) -> impl IntoElement {
    // Read again when other pages come into view or the document is edited, not as the pages scroll.
    let cards = computed(move || {
        let pages = state.shown_pages.get();
        state.editing.revision();
        state.cards(&pages)
    });
    let tops = computed(move || {
        let open = state
            .noting
            .with(|noting| noting.as_ref().map(|noting| noting.target.clone()));
        let maps = state.page_maps.get();
        cards.with(|cards| {
            let slots: Vec<margin::Card> = cards
                .iter()
                .map(|card| margin::Card {
                    anchor: maps
                        .iter()
                        .find(|(page, _)| *page == card.target().page)
                        .and_then(|(_, map)| card.outline.top(*map))
                        .unwrap_or_default(),
                    height: card.height(open.as_ref() == Some(card.target())),
                })
                .collect();
            margin::layout(&slots, GAP, GAP)
        })
    });
    let list = dynamic(move || {
        cards.with(|cards| {
            cards
                .iter()
                .enumerate()
                .map(|(at, card)| {
                    card_view(state, card.clone(), move || {
                        tops.with(|tops| tops.get(at).copied())
                    })
                })
                .collect::<Vec<_>>()
        })
    });
    view()
        .w(px(WIDTH))
        .h_full()
        .overflow_hidden()
        .border_l_1()
        .border_color(move || theme().border)
        .bg(move || theme().sidebar)
        .relative()
        .style_when(move || !state.margin_shown(), |s| s.hidden())
        .label(move || SharedString::from(t!("margin.label")))
        .child(list)
}

/// One note's card at its laid-out `top`: closed, its heading and first lines, a button that opens it; open, its
/// heading and its note to write.
fn card_view(
    state: AnnotateState,
    card: Card,
    top: impl Fn() -> Option<f32> + Copy + 'static,
) -> AnyElement {
    let target = card.target().clone();
    let open = {
        let target = target.clone();
        computed(move || {
            state.noting.with(|noting| {
                noting
                    .as_ref()
                    .is_some_and(|noting| noting.target == target)
            })
        })
    };
    let open = move || open.get();
    let color = card.noting.color();
    let kind = card.noting.kind_name();
    let heading = if card.author.is_empty() {
        kind.clone()
    } else {
        t!(
            "margin.heading",
            kind = kind.clone(),
            author = card.author.clone()
        )
    };
    let spoken = t!(
        "margin.card",
        kind = kind,
        page = target.page + 1,
        text = card.text.clone()
    );
    let height = {
        let card = card.clone();
        move || card.height(open())
    };
    let body = dynamic(move || {
        if open() {
            let editor = field(input().value(move || state.draft.get()))
                .multiline(true)
                .h(px(EDITOR - 8.0))
                .py_2()
                .on_change(move |text: &str| state.draft.set(text.to_owned()))
                .label(t!("note.label"))
                .auto_focus(true);
            view()
                .gap_2()
                .child(card_heading(color, heading.clone()))
                .child(editor)
                .child(
                    view()
                        .flex_row()
                        .child(view().flex_1())
                        .child(action_button("note.close", Tone::Plain, move || {
                            state.close_note();
                        })),
                )
                .into_element()
        } else {
            let target = target.clone();
            button()
                .accessibility_label(spoken.clone())
                .on_press(move || state.open_note(&target))
                .w_full()
                .items_start()
                .rounded(px(6.0))
                .focus_visible(|s| s.border_2().border_color(move || theme().ring))
                .child(card_heading(color, heading.clone()))
                .child(
                    text(card.text.clone())
                        .number_of_lines(LINES as u32)
                        .text_size(px(13.0))
                        .line_height(px(LINE))
                        .text_color(move || theme().card_foreground),
                )
                .into_element()
        }
    });
    view()
        .absolute()
        .left(px(GAP))
        .right(px(GAP))
        // A card whose place isn't known yet waits out of sight.
        .top(move || Length::from(px(top().unwrap_or(-10_000.0))))
        .h(move || Length::from(px(height())))
        .overflow_hidden()
        .p(px(PAD))
        .rounded(px(8.0))
        .border_1()
        .border_color(move || if open() { theme().ring } else { theme().border })
        .bg(move || theme().card)
        .shadow_xs()
        .child(body)
        .into_element()
}

/// A card's first line: its annotation's colour, kind and author.
fn card_heading(color: u32, heading: String) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(HEADING))
        .child(
            view()
                .w(px(8.0))
                .h(px(8.0))
                .rounded(px(4.0))
                .bg(Hsla::from(rgb(color))),
        )
        .child(
            text(heading)
                .flex_1()
                .number_of_lines(1u32)
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
}
