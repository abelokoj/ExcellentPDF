//! The note editor (A.4): a markup's note, written in the inspector beside the pages so it never covers them (§7.3).
//! A click on a markup, or the markup bar's note tool, opens it; the text is kept as typed and saved into the PDF as
//! the markup's `/Contents`, one step of undo, when the reader is done or opens another note. A sticky note's (A.5)
//! also has its icon and whether other readers show it open. The models are `excellent-features`' `annotate::notes`
//! and `annotate::sticky`.

use excellent_core::annot::{Annotation, Markup, Shape};
use excellent_features::annotate::notes::{self, Target};
use excellent_features::annotate::sticky::{self, Icon};
use nsh_base::{input, theme};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::settings_window::{on_off, segment, segmented};
use crate::tabs::{Tone, action_button};

/// The note open in the inspector: whose it is, and what the panel shows of its markup.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Noting {
    pub(super) target: Target,
    kind: Option<Markup>,
    /// The text under the markup, when it was kept (K.28).
    quote: String,
    color: u32,
    /// A sticky note's icon and open state; `None` for a markup.
    sticky: Option<(Icon, bool)>,
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
            sticky: sticky::is_sticky(annotation)
                .then(|| (Icon::of(&annotation.icon), annotation.open)),
        }
    }
}

/// Characters of the markup's text the panel quotes.
const QUOTE: usize = 160;

/// The inspector's content for a pane: where a sticky note goes while one is being placed, the open note, or what
/// the inspector will show.
pub(crate) fn note_panel(state: AnnotateState) -> impl IntoElement {
    dynamic(move || match (state.placing.get(), state.noting.get()) {
        (true, _) => placing().into_element(),
        (false, Some(noting)) => untracked(|| editor(state, noting).into_element()),
        (false, None) => {
            crate::shell::empty_message(|| t!("inspector.empty"), || t!("note.empty_hint"))
                .into_element()
        }
    })
}

/// While a sticky note is being placed: where to click, and the key that calls it off.
fn placing() -> View {
    crate::shell::empty_message(
        || t!("sticky.placing"),
        || {
            t!(
                "sticky.placing_hint",
                key = crate::commands::shortcut_label(crate::commands::CommandId::StickyNote)
            )
        },
    )
}

fn editor(state: AnnotateState, noting: Noting) -> View {
    let color = Hsla::from(rgb(noting.color));
    let heading = t!(
        "note.heading",
        kind = noting.kind.map_or_else(|| t!("sticky.name"), super::name),
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
    if let Some((icon, open)) = noting.sticky {
        panel = panel.child(sticky_controls(state, icon, open));
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

/// A sticky note's icon, chosen from the standard's seven, and whether other readers show its note open. Each change
/// is one step of undo.
fn sticky_controls(state: AnnotateState, icon: Icon, open: bool) -> View {
    let icon = signal(icon);
    let open = signal(open);
    let icons = Icon::ALL.into_iter().fold(segmented(), |group, choice| {
        group.child(segment(
            icon_label(choice),
            move || icon.get() == choice,
            move || {
                icon.set(choice);
                state.change_note(t!("sticky.icon_edit"), |note| {
                    sticky::set_icon(note, choice)
                });
            },
        ))
    });
    view()
        .gap_2()
        .child(caption("sticky.icon"))
        .child(icons)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    text(t!("sticky.open"))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                )
                .child(on_off(
                    "sticky.open",
                    move || open.get(),
                    move |on| {
                        open.set(on);
                        state
                            .change_note(t!("sticky.open_edit"), |note| sticky::set_open(note, on));
                    },
                )),
        )
}

/// The translation key of an icon's name.
fn icon_label(icon: Icon) -> &'static str {
    match icon {
        Icon::Comment => "sticky.icon.comment",
        Icon::Note => "sticky.icon.note",
        Icon::Help => "sticky.icon.help",
        Icon::Insert => "sticky.icon.insert",
        Icon::Key => "sticky.icon.key",
        Icon::Paragraph => "sticky.icon.paragraph",
        Icon::NewParagraph => "sticky.icon.new_paragraph",
    }
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
        assert_eq!(noting.sticky, None);
        let mut note = Annotation::new(
            0,
            Shape::Boxed(excellent_core::annot::Boxed::Note, Default::default()),
            Style::default(),
            "me",
        );
        note.icon = "Key".to_owned();
        note.open = true;
        assert_eq!(Noting::of(&note).sticky, Some((Icon::Key, true)));
    }
}
