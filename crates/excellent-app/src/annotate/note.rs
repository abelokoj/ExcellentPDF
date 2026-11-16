//! The note editor (A.4): a markup's note, written in the inspector beside the pages so it never covers them (§7.3).
//! A click on a markup, or the markup bar's note tool, opens it; the text is kept as typed and saved into the PDF as
//! the markup's `/Contents`, one step of undo, when the reader is done or opens another note. A sticky note's (A.5)
//! also has its icon and whether other readers show it open. A text box's (A.6) is its text, typed here so input
//! methods work (in place on the page once U.8 gives the canvas the platform's input path), with its font, size,
//! alignment, colour and border. A line's (A.8) has its endings. The models are `excellent-features`'
//! `annotate::notes`, `annotate::sticky`, `annotate::freetext` and `annotate::shapes`.

use excellent_core::annot::{Align, Annotation, Font, LineEnd, Markup, Shape, TextStyle};
use excellent_features::annotate::freetext::{self, Kind};
use excellent_features::annotate::notes::{self, Target};
use excellent_features::annotate::shapes;
use excellent_features::annotate::sticky::{self, Icon};
use nsh_base::{input, theme};
use nsh_core::prelude::*;

use super::{AnnotateState, Placing};
use crate::bookmarks::{caption, field};
use crate::settings_window::{on_off, segment, segment_of, segmented};
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
    /// A text box's kind, text style, whether it has a border and a callout's ending at its tip; `None` for the
    /// others.
    text: Option<(Kind, TextStyle, bool, LineEnd)>,
    /// A line's endings, start first, and whether it is an arrow; `None` for the others.
    line: Option<([LineEnd; 2], bool)>,
    /// Whether it was just placed: a text box left empty is then taken away.
    pub(super) fresh: bool,
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
            // A text box's dot is its text's colour; its own (`style.color`) is its background.
            color: if annotation.is_text_box() {
                packed(annotation.text.color)
            } else {
                annotation.style.color.map_or(0xff_d54f, packed)
            },
            sticky: sticky::is_sticky(annotation)
                .then(|| (Icon::of(&annotation.icon), annotation.open)),
            text: freetext::kind_of(annotation).map(|kind| {
                (
                    kind,
                    annotation.text,
                    annotation.style.width > 0.0,
                    annotation.ends[0],
                )
            }),
            line: shapes::is_line(annotation)
                .then(|| (annotation.ends, shapes::is_arrow(annotation))),
            fresh: false,
        }
    }
}

/// Characters of the markup's text the panel quotes.
const QUOTE: usize = 160;

/// The inspector's content for a pane: where a sticky note or text box goes while one is being placed, the open
/// note or text, or what the inspector will show.
pub(crate) fn note_panel(state: AnnotateState) -> impl IntoElement {
    dynamic(move || match (state.placing.get(), state.noting.get()) {
        (Some(what), _) => placing(what).into_element(),
        (None, Some(noting)) => untracked(|| editor(state, noting).into_element()),
        (None, None) => {
            crate::shell::empty_message(|| t!("inspector.empty"), || t!("note.empty_hint"))
                .into_element()
        }
    })
}

/// While a sticky note or text box is being placed: where to click, and the key that calls it off.
fn placing(what: Placing) -> View {
    use crate::commands::{CommandId, shortcut_label};
    let (title, hint, command) = match what {
        Placing::Sticky => (
            "sticky.placing",
            "sticky.placing_hint",
            CommandId::StickyNote,
        ),
        Placing::Text(Kind::Box) => ("text.placing", "text.placing_hint", CommandId::TextBox),
        Placing::Text(Kind::Typewriter) => (
            "text.placing",
            "text.placing_typewriter_hint",
            CommandId::Typewriter,
        ),
        Placing::Shape(shapes::Kind::Line) => {
            ("line.placing", "line.placing_hint", CommandId::Line)
        }
        Placing::Shape(shapes::Kind::Arrow) => {
            ("line.placing", "line.placing_hint", CommandId::Arrow)
        }
        Placing::Text(Kind::Callout) => (
            "callout.placing",
            "callout.placing_hint",
            CommandId::Callout,
        ),
    };
    crate::shell::empty_message(
        move || t!(title),
        move || t!(hint, key = shortcut_label(command)),
    )
}

fn editor(state: AnnotateState, noting: Noting) -> View {
    let color = Hsla::from(rgb(noting.color));
    let kind = match (noting.kind, noting.text, noting.line) {
        (Some(kind), ..) => super::name(kind),
        (None, Some((kind, ..)), _) => super::text_name(kind),
        (None, None, Some((_, true))) => t!("arrow.name"),
        (None, None, Some((_, false))) => t!("line.name"),
        (None, None, None) => t!("sticky.name"),
    };
    let heading = t!("note.heading", kind = kind, page = noting.target.page + 1);
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
    let boxed = noting.text.is_some();
    let (label, placeholder, saved_hint) = if boxed {
        ("text.label", "text.placeholder", "text.saved_hint")
    } else {
        ("note.label", "note.placeholder", "note.saved_hint")
    };
    let had_note = !untracked(|| state.draft.get()).is_empty();
    let note = field(input().value(move || state.draft.get()))
        .multiline(true)
        .min_h(px(120.0))
        .py_2()
        .on_change(move |text: &str| state.draft.set(text.to_owned()))
        .placeholder(t!(placeholder))
        .label(t!(label))
        .auto_focus(true);
    let mut buttons = view().flex_row().gap_2().items_center();
    if boxed {
        buttons = buttons.child(action_button("text.delete", Tone::Destructive, move || {
            state.delete_open(t!("text.delete"));
        }));
    } else if noting.line.is_some() {
        buttons = buttons.child(action_button("line.delete", Tone::Destructive, move || {
            state.delete_open(t!("line.delete"));
        }));
    } else if had_note {
        buttons = buttons.child(action_button("note.delete", Tone::Destructive, move || {
            state.draft.set(String::new());
            state.close_note();
        }));
    }
    if let Some((icon, open)) = noting.sticky {
        panel = panel.child(sticky_controls(state, icon, open));
    }
    panel = panel.child(caption(label)).child(note);
    if let Some((kind, style, border, end)) = noting.text {
        panel = panel.child(text_controls(state, kind, style, border, end));
    }
    if let Some((ends, _)) = noting.line {
        panel = panel.child(line_controls(state, ends));
    }
    panel
        .child(buttons.child(view().flex_1()).child(action_button(
            "note.close",
            Tone::Plain,
            move || state.close_note(),
        )))
        .child(
            text(t!(saved_hint))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// The text sizes offered, in points.
const SIZES: [f32; 6] = [9.0, 10.0, 12.0, 14.0, 18.0, 24.0];

/// The text colours offered, with their names: black first, then three that keep AA contrast on white paper.
const COLORS: [([f32; 3], &str); 4] = [
    ([0.0, 0.0, 0.0], "text.color.black"),
    ([0.08, 0.32, 0.78], "text.color.blue"),
    ([0.78, 0.12, 0.12], "text.color.red"),
    ([0.06, 0.45, 0.22], "text.color.green"),
];

/// A text box's font, size, alignment and colour, for a boxed one its border, and for a callout the ending at its
/// tip. Each change is one step of undo and fits the box to the text again.
fn text_controls(
    state: AnnotateState,
    kind: Kind,
    style: TextStyle,
    border: bool,
    end: LineEnd,
) -> View {
    let style = signal(style);
    let restyle = move |change: &dyn Fn(&mut TextStyle)| {
        let mut new = untracked(|| style.get());
        change(&mut new);
        style.set(new);
        state.change_note(t!("text.style_edit"), |text_box| {
            freetext::set_style(text_box, new)
        });
    };
    let fonts = Font::ALL.into_iter().fold(segmented(), |group, font| {
        group.child(segment(
            font_label(font),
            move || style.get().font == font,
            move || restyle(&|s| s.font = font),
        ))
    });
    let sizes = SIZES.into_iter().fold(segmented(), |group, size| {
        group.child(segment_of(
            move || t!("text.size_pt", size = size),
            move || style.get().size == size,
            move || restyle(&|s| s.size = size),
        ))
    });
    let aligns =
        [Align::Left, Align::Center, Align::Right]
            .into_iter()
            .fold(segmented(), |group, align| {
                group.child(segment(
                    align_label(align),
                    move || style.get().align == align,
                    move || restyle(&|s| s.align = align),
                ))
            });
    let colors = COLORS
        .into_iter()
        .fold(segmented(), |group, (color, name)| {
            group.child(segment(
                name,
                move || style.get().color == color,
                move || restyle(&|s| s.color = color),
            ))
        });
    let mut controls = view()
        .gap_2()
        .child(caption("text.font"))
        .child(fonts)
        .child(caption("text.size"))
        .child(sizes)
        .child(caption("text.align"))
        .child(aligns)
        .child(caption("text.color"))
        .child(colors);
    if kind == Kind::Callout {
        let end = signal(end);
        let ends = freetext::CALLOUT_ENDS
            .into_iter()
            .fold(segmented(), |group, choice| {
                group.child(segment(
                    end_label(choice),
                    move || end.get() == choice,
                    move || {
                        end.set(choice);
                        state.change_note(t!("callout.end_edit"), |callout| {
                            freetext::set_end(callout, choice)
                        });
                    },
                ))
            });
        controls = controls.child(caption("callout.end")).child(ends);
    }
    if kind != Kind::Typewriter {
        let border = signal(border);
        controls = controls.child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    text(t!("text.border"))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                )
                .child(on_off(
                    "text.border",
                    move || border.get(),
                    move |on| {
                        border.set(on);
                        state.change_note(t!("text.border_edit"), |text_box| {
                            freetext::set_border(text_box, on)
                        });
                    },
                )),
        );
    }
    controls
}

/// The translation key of a font's name.
fn font_label(font: Font) -> &'static str {
    match font {
        Font::Sans => "text.font.sans",
        Font::Serif => "text.font.serif",
        Font::Mono => "text.font.mono",
    }
}

/// A line's endings, at its start and at its end (A.8); each change is one step of undo.
fn line_controls(state: AnnotateState, ends: [LineEnd; 2]) -> View {
    let ends = signal(ends);
    let row = move |side: usize| {
        shapes::ENDS
            .into_iter()
            .fold(segmented(), move |group, choice| {
                group.child(segment(
                    line_end_label(choice),
                    move || ends.get()[side] == choice,
                    move || {
                        let mut new = untracked(|| ends.get());
                        new[side] = choice;
                        ends.set(new);
                        state.change_note(t!("line.ends_edit"), |line| shapes::set_ends(line, new));
                    },
                ))
            })
    };
    view()
        .gap_2()
        .child(caption("line.start"))
        .child(row(0))
        .child(caption("line.end"))
        .child(row(1))
}

/// The translation key of a line ending's name.
fn line_end_label(end: LineEnd) -> &'static str {
    match end {
        LineEnd::OpenArrow => "callout.end.open_arrow",
        LineEnd::ClosedArrow => "callout.end.closed_arrow",
        LineEnd::Circle => "callout.end.circle",
        LineEnd::Square => "callout.end.square",
        LineEnd::Diamond => "line.end.diamond",
        LineEnd::Slash => "line.end.slash",
        _ => "callout.end.none",
    }
}

/// The translation key of a callout ending's name.
fn end_label(end: LineEnd) -> &'static str {
    match end {
        LineEnd::OpenArrow => "callout.end.open_arrow",
        LineEnd::ClosedArrow => "callout.end.closed_arrow",
        LineEnd::Circle => "callout.end.circle",
        LineEnd::Square => "callout.end.square",
        _ => "callout.end.none",
    }
}

/// The translation key of an alignment's name.
fn align_label(align: Align) -> &'static str {
    match align {
        Align::Left => "text.align.left",
        Align::Center => "text.align.center",
        Align::Right => "text.align.right",
    }
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
        assert_eq!(Noting::of(&note).text, None);
        let mut boxed = Annotation::new(
            2,
            Shape::Boxed(excellent_core::annot::Boxed::FreeText, Default::default()),
            Style {
                width: 1.0,
                ..Style::default()
            },
            "me",
        );
        boxed.text.size = 18.0;
        let noting = Noting::of(&boxed);
        assert_eq!(
            noting.text,
            Some((Kind::Box, boxed.text, true, LineEnd::None))
        );
        assert!(!noting.fresh, "only a box just placed is");
    }
}
