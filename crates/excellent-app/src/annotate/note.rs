//! The note editor (A.4): a markup's note, written in the inspector beside the pages so it never covers them (§7.3).
//! A click on a markup, or the markup bar's note tool, opens it; the text is kept as typed and saved into the PDF as
//! the markup's `/Contents`, one step of undo, when the reader is done or opens another note. The model is
//! `excellent-features`' `annotate::notes`.

use excellent_core::annot::{Annotation, Markup, Shape};
use excellent_features::annotate::notes::{self, Target};
use nsh_base::{input, theme};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::tabs::{Tone, action_button};

/// The note open in the inspector: whose it is, and what the panel shows of its markup.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Noting {
    pub(super) target: Target,
    kind: Option<Markup>,
    /// The text under the markup, when it was kept (K.28).
    quote: String,
    color: u32,
}

impl Noting {
    pub(super) fn of(annotation: &Annotation) -> Self {
        let kind = match annotation.shape {
            Shape::Markup(kind, _) => Some(kind),
            _ => None,
        };
        Self {
            target: Target {
                page: annotation.page,
                id: annotation.id.clone(),
            },
            kind,
            quote: annotation
                .anchor
                .as_ref()
                .map(|quote| quote.exact.trim().to_owned())
                .unwrap_or_default(),
            color: annotation.style.color.map_or(0xff_d54f, packed),
        }
    }
}

/// Characters of the markup's text the panel quotes.
const QUOTE: usize = 160;

/// The inspector's content for a pane: the open note, or what the inspector will show.
pub(crate) fn note_panel(state: AnnotateState) -> impl IntoElement {
    dynamic(move || match state.noting.get() {
        Some(noting) => untracked(|| editor(state, noting).into_element()),
        None => crate::shell::empty_message(|| t!("inspector.empty"), || t!("note.empty_hint"))
            .into_element(),
    })
}

fn editor(state: AnnotateState, noting: Noting) -> View {
    let color = Hsla::from(rgb(noting.color));
    let heading = t!(
        "note.heading",
        kind = noting.kind.map(super::name).unwrap_or_default(),
        page = noting.target.page + 1
    );
    let mut panel = view().flex_1().p_4().gap_3().child(
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                view()
                    .w(px(10.0))
                    .h(px(10.0))
                    .rounded(px(5.0))
                    .bg(color)
                    .border_1()
                    .border_color(move || theme().border),
            )
            .child(
                text(heading)
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            ),
    );
    if !noting.quote.is_empty() {
        // The markup's text, set off by a rule in its colour, so the note is read beside what it is about.
        let quote = notes::first_line(&noting.quote.replace('\n', " "), QUOTE);
        panel = panel.child(
            view()
                .flex_row()
                .gap_2()
                .child(view().w(px(3.0)).rounded(px(2.0)).bg(color))
                .child(
                    text(quote)
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                ),
        );
    }
    let had_note = !untracked(|| state.draft.get()).is_empty();
    let note = field(input().value(move || state.draft.get()))
        .multiline(true)
        .min_h(px(120.0))
        .py_2()
        .on_change(move |text: &str| state.draft.set(text.to_owned()))
        .placeholder(t!("note.placeholder"))
        .label(t!("note.label"))
        .auto_focus(true);
    let mut buttons = view().flex_row().gap_2().items_center();
    if had_note {
        buttons = buttons.child(action_button("note.delete", Tone::Destructive, move || {
            state.draft.set(String::new());
            state.close_note();
        }));
    }
    panel
        .child(caption("note.label"))
        .child(note)
        .child(buttons.child(view().flex_1()).child(action_button(
            "note.close",
            Tone::Plain,
            move || state.close_note(),
        )))
        .child(
            text(t!("note.saved_hint"))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// A colour's channels, 0 to 1, as `0xrrggbb`.
fn packed(color: [f32; 3]) -> u32 {
    color
        .iter()
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32)
        .fold(0, |rgb, channel| rgb << 8 | channel)
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::Style;

    use super::*;

    #[test]
    fn the_panel_shows_the_markup_s_colour_and_text() {
        let mut highlight = Annotation::new(
            4,
            Shape::Markup(Markup::Underline, Vec::new()),
            Style {
                color: Some([1.0, 0.5, 0.0]),
                ..Style::default()
            },
            "me",
        );
        highlight.anchor = Some(excellent_core::anchor::Quote {
            prefix: String::new(),
            exact: " the words ".to_owned(),
            suffix: String::new(),
            start: 0,
        });
        let noting = Noting::of(&highlight);
        assert_eq!(noting.color, 0xff_8000);
        assert_eq!(noting.quote, "the words");
        assert_eq!(noting.kind, Some(Markup::Underline));
        assert_eq!(noting.target.page, 4);
    }
}
