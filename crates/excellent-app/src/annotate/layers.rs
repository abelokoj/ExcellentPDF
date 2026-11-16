//! Annotation layers (A.22) in the app: the Layers section at the top of the annotation list, where each layer is
//! shown or hidden and locked, and the annotations are sorted into one layer per author; the Layer row of the
//! inspector; and the page leaving out, for presses and the margin, what a hidden layer holds (MuPDF already leaves it
//! out of the drawing). Each change is one step of undo. The model is `excellent-features`' `annotate::layers`.

use std::slice;

use excellent_core::annot::{Annotation, load_all, page_annotations};
use excellent_core::edit::{Change, Operation};
use excellent_core::shared::pool;
use excellent_features::annotate::layers;
use excellent_mupdf::Layer;
use nsh_base::{input, switch, theme, toggle};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::settings_window::{SWITCH_THUMB, SWITCH_TRACK, segment_of, segmented};
use crate::tabs::{Tone, action_button};

impl AnnotateState {
    /// The shown document's layers; none when it has none or can't be read.
    pub(super) fn layers(self) -> Vec<Layer> {
        let Some(document) = self.editing.document() else {
            return Vec::new();
        };
        let Ok(shared) = document.mupdf() else {
            return Vec::new();
        };
        let mupdf = shared.lock().expect("document lock poisoned");
        mupdf
            .layers()
            .map_err(|err| log::warn!("layers: cannot read them: {err}"))
            .unwrap_or_default()
    }

    /// Makes `change` of a layer, one step of undo named `name`; nothing for none.
    fn edit_layer(self, name: String, change: Option<Change>) {
        if let Some(change) = change {
            self.editing.apply(&Operation::new(name, vec![change]));
        }
    }

    /// Puts every annotation of the shown document that is on no layer on its author's (A.22), one step of undo.
    /// The annotations are read on the render workers.
    pub(crate) fn layers_by_author(self) {
        let Some(document) = self.editing.document() else {
            return;
        };
        let mupdf = match document.mupdf() {
            Ok(mupdf) => mupdf,
            Err(err) => {
                log::warn!("layers: cannot read the document: {err}");
                return;
            }
        };
        let job = load_all(pool(), mupdf);
        spawn(async move {
            match job.await {
                Ok(annotations) => {
                    let changes = layers::by_author(&annotations, &t!("layers.unnamed"));
                    if !changes.is_empty() {
                        self.editing
                            .apply(&Operation::new(t!("layers.by_author_edit"), changes));
                    }
                }
                Err(err) if !err.is_aborted() => log::warn!("layers: {err}"),
                Err(_) => {}
            }
        });
    }
}

/// Page `page`'s annotations that are drawn: those on a hidden layer are left out, as MuPDF leaves them out of the
/// page. An error reading the page reads as none, and one reading the layers as none hidden.
pub(super) fn shown_on_page(mupdf: &excellent_mupdf::Document, page: usize) -> Vec<Annotation> {
    let mut annotations = page_annotations(mupdf, page)
        .map_err(|err| log::warn!("annotate: cannot read page {page}'s annotations: {err}"))
        .unwrap_or_default();
    let all = mupdf
        .layers()
        .map_err(|err| log::warn!("layers: cannot read them: {err}"))
        .unwrap_or_default();
    annotations.retain(|annotation| layers::shown(annotation, &all));
    annotations
}

/// The Layers section of the annotation list: a line per layer of `all` (shown, its name, how many annotations
/// `count` says it holds, locked) and "One layer per author".
pub(super) fn layers_section(
    state: AnnotateState,
    all: Memo<Vec<Layer>>,
    count: impl Fn(&str) -> usize + Copy + 'static,
) -> impl IntoElement {
    let header = view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().child(caption("layers.title")))
        .child(action_button("layers.by_author", Tone::Plain, move || {
            state.layers_by_author();
        }));
    let lines = dynamic(move || {
        let layers = all.get();
        if layers.is_empty() {
            return view().child(
                text(t!("layers.empty"))
                    .text_size(px(13.0))
                    .text_color(move || theme().muted_foreground),
            );
        }
        layers.into_iter().fold(view().gap_1(), |lines, layer| {
            lines.child(layer_line(state, layer, count))
        })
    });
    view()
        .px_3()
        .pt_2()
        .pb_1()
        .gap_2()
        .child(header)
        .child(lines)
        .child(view().h(px(1.0)).bg(move || theme().border))
}

/// One layer: whether it is shown (fixed while it is locked), its name, how many annotations are on it, its lock.
fn layer_line(
    state: AnnotateState,
    layer: Layer,
    count: impl Fn(&str) -> usize + Copy + 'static,
) -> View {
    let (visible, locked) = (layer.visible, layer.locked);
    let held = count(&layer.name);
    let shown_control = if locked {
        // A locked layer can't be shown or hidden here, as in Acrobat: its state reads instead of a switch.
        view().w(px(36.0)).child(
            text(t!(if visible {
                "layers.shown"
            } else {
                "layers.hidden"
            }))
            .text_size(px(12.0))
            .text_color(move || theme().muted_foreground),
        )
    } else {
        let to_show = layer.clone();
        view().child(
            switch()
                .checked(move || visible)
                .accessibility_label(t!("layers.show", name = layer.name.clone()))
                .on_change(move |on: &bool| {
                    state.edit_layer(t!("layers.show_edit"), layers::show(&to_show, *on));
                })
                .track_class(SWITCH_TRACK)
                .thumb_class(SWITCH_THUMB),
        )
    };
    let to_lock = layer.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .min_h(px(32.0))
        .child(shown_control)
        .child(
            text(layer.name.clone())
                .flex_1()
                .text_size(px(13.0))
                .text_color(move || {
                    if visible {
                        theme().sidebar_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
        .child(
            text(held.to_string())
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            toggle()
                .pressed(move || locked)
                .accessibility_label(t!("layers.lock", name = layer.name.clone()))
                .on_change(move |on: &bool| {
                    state.edit_layer(t!("layers.lock_edit"), layers::lock(&to_lock, *on));
                })
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .items_center()
                .justify_center()
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .pressed_style(|s| s.bg(move || theme().sidebar_accent))
                .focus_visible(|s| s.border_1().border_color(move || theme().ring))
                .child(
                    text(t!(if locked {
                        "layers.locked"
                    } else {
                        "layers.unlocked"
                    }))
                    .text_size(px(12.0))
                    .text_color(move || {
                        if locked {
                            theme().primary
                        } else {
                            theme().muted_foreground
                        }
                    }),
                ),
        )
}

/// The inspector's Layer row for `annotation`: the layer it is on, typed (a new name makes a layer), and the
/// document's layers to put it on with one press (pressing its own takes it off).
pub(super) fn layer_rows(state: AnnotateState, annotation: &Annotation) -> View {
    let typed = signal(annotation.layer.clone().unwrap_or_default());
    let keep = move || {
        let text = untracked(|| typed.get());
        state.change_note(t!("layers.move_edit"), |a| {
            layers::move_to(slice::from_ref(a), layers::name(&text).as_deref()).pop()
        });
    };
    let mut rows = view().gap_2().child(caption("layers.layer")).child(
        field(input().value(move || typed.get()))
            .on_change(move |text: &str| typed.set(text.to_owned()))
            .on_submit(move |_: &str| keep())
            .on_blur(keep)
            .placeholder(t!("layers.none"))
            .label(t!("layers.layer")),
    );
    let names: Vec<String> = state.layers().into_iter().map(|layer| layer.name).collect();
    if !names.is_empty() {
        let on = annotation.layer.clone();
        let chips = names.into_iter().fold(segmented(), |group, name| {
            let chosen = on.as_deref() == Some(name.as_str());
            let label = signal(name.clone());
            group.child(segment_of(
                move || label.get(),
                move || chosen,
                move || {
                    let to = (!chosen).then(|| name.clone());
                    state.change_note(t!("layers.move_edit"), |a| {
                        layers::move_to(slice::from_ref(a), to.as_deref()).pop()
                    });
                },
            ))
        });
        rows = rows.child(chips);
    }
    rows
}
