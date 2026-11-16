//! The annotation inspector's own rows (A.12), under the note and its kind's controls (`note`): who made it, its
//! colour and opacity, when it was made and changed, and its lock. Each edit is one step of undo through
//! `AnnotateState::change_note`; the rows are built again from the annotation as the document has it after every
//! edit, so an undone change shows in the next frame. A locked annotation shows its facts and its lock, and why the
//! rest is gone. The model is `excellent-features`' `annotate::inspector`.

use excellent_core::annot::{Annotation, Markup, Shape};
use excellent_features::annotate::colors::{Tool, ToolStyle};
use excellent_features::annotate::inspector::{self, Details, OPACITIES};
use nsh_base::{button, input, theme};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::settings_window::{on_off, segment_of, segmented};

/// Who made `annotation`, its colour and its opacity: what a lock keeps as it is.
pub(super) fn look(state: AnnotateState, annotation: &Annotation) -> View {
    let details = Details::of(annotation);
    let author = signal(details.author.clone());
    let commit = move || {
        let name = untracked(|| author.get());
        state.change_note(t!("inspector.author_edit"), |a| {
            inspector::set_author(a, &name)
        });
    };
    let author_field = field(input().value(move || author.get()))
        .on_change(move |text: &str| author.set(text.to_owned()))
        .on_submit(move |_: &str| commit())
        .on_blur(commit)
        .label(t!("inspector.author"));
    let mut rows = view()
        .gap_2()
        .child(caption("inspector.author"))
        .child(author_field);
    if inspector::has_color(annotation) {
        rows = rows.child(caption("inspector.color")).child(swatches(
            state,
            palette_tool(annotation),
            details.color,
        ));
    }
    let opacity = details.opacity;
    let opacities = OPACITIES.into_iter().fold(segmented(), |group, choice| {
        group.child(segment_of(
            move || {
                t!(
                    "inspector.percent",
                    value = (choice * 100.0).round().to_string()
                )
            },
            move || (opacity - choice).abs() < 0.005,
            move || {
                state.change_note(t!("inspector.opacity_edit"), |a| {
                    inspector::set_opacity(a, choice)
                });
            },
        ))
    });
    rows.child(caption("inspector.opacity")).child(opacities)
}

/// When `annotation` was made and last changed, and its lock.
pub(super) fn facts(state: AnnotateState, annotation: &Annotation) -> View {
    let details = Details::of(annotation);
    let mut rows = view().gap_2();
    for (key, seconds) in [
        ("inspector.created", details.created),
        ("inspector.modified", details.modified),
    ] {
        let when = inspector::date(seconds);
        if when.is_empty() {
            continue;
        }
        rows = rows.child(
            view()
                .flex_row()
                .gap_3()
                .child(
                    text(t!(key))
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(t!("inspector.utc", when = when))
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground),
                ),
        );
    }
    let locked = details.locked;
    rows.child(
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .child(
                view()
                    .flex_1()
                    .child(
                        text(t!("inspector.lock"))
                            .text_size(px(13.0))
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(t!("inspector.lock_hint"))
                            .text_size(px(12.0))
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(on_off(
                "inspector.lock",
                move || locked,
                move |on| {
                    let name = if on {
                        "inspector.lock_edit"
                    } else {
                        "inspector.unlock_edit"
                    };
                    state.change_note(t!(name), |a| inspector::set_locked(a, on));
                },
            )),
    )
}

/// Why a locked annotation's look and place can't be changed, and how to change them.
pub(super) fn locked_hint() -> View {
    view()
        .p_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted)
        .child(
            text(t!("inspector.locked"))
                .text_size(px(13.0))
                .text_color(move || theme().foreground),
        )
}

/// The palette `annotation` takes its colours from: a highlighter's light ones under text, else the strong ones.
fn palette_tool(annotation: &Annotation) -> Tool {
    match annotation.shape {
        Shape::Markup(Markup::Highlight, _) => Tool::Highlight,
        _ => Tool::Shape,
    }
}

/// The palette's swatches for `tool`, the one `current` marked; a press gives the annotation that colour.
fn swatches(state: AnnotateState, tool: Tool, current: Option<[f32; 3]>) -> View {
    let current = current.map(super::note::packed);
    super::colors::palette(state.live)
        .swatches(tool)
        .into_iter()
        .fold(view().flex_row().gap_2(), |row, color| {
            let chosen = current == Some(color);
            let label = if chosen {
                "inspector.swatch_chosen"
            } else {
                "inspector.swatch"
            };
            row.child(
                button()
                    .accessibility_label(t!(
                        label,
                        color = excellent_features::annotate::colors::hex(color)
                    ))
                    .on_press(move || {
                        let rgb = ToolStyle::new(color, 1.0).rgb();
                        state.change_note(t!("inspector.color_edit"), |a| {
                            inspector::set_color(a, rgb)
                        });
                    })
                    .w(px(28.0))
                    .h(px(28.0))
                    .rounded_full()
                    .bg(Hsla::from(rgb(color)))
                    .border_2()
                    .border_color(move || {
                        if chosen {
                            theme().foreground
                        } else {
                            theme().border
                        }
                    })
                    .focus_visible(|s| s.border_color(move || theme().ring)),
            )
        })
}
