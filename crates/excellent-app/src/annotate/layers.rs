//! Annotation & ink layers (A.22, H.14) in the app: the Layers section at the top of the annotation list, showing
//! the four fixed layers (`Ink`, `Highlights`, `Text`, `Images`) plus user layers backed by PDF OCGs, where each layer
//! is shown/hidden, locked/unlocked, or chosen as the active drawing layer, and locked layers reject selection and
//! erasing. Each change is one step of undo. Models: `excellent-features`' `annotate::layers` & `ink::layers`.

use std::slice;

use excellent_core::annot::{Annotation, load_all, page_annotations};
use excellent_core::edit::{Change, Operation};
use excellent_core::shared::pool;
use excellent_features::annotate::layers;
use excellent_features::annotate::notes::Target;
use excellent_features::ink::layers::{LayerStack, LockState, Visibility};
use excellent_mupdf::Layer;
use nsh_base::{input, switch, theme, toggle};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::editing::show_message;
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

    /// The ink layer stack synced with the document's OCGs and active drawing layer (H.14).
    pub(super) fn layer_stack(self) -> LayerStack {
        let mut stack = LayerStack::from_ocgs(&untracked(|| self.layers()));
        let active = untracked(|| self.active_layer.get());
        let _ = stack.set_active(&active);
        stack
    }

    /// Sets the active drawing layer when it exists, is visible, and is unlocked (H.14).
    pub(crate) fn set_active_layer(self, name: &str) {
        let mut stack = self.layer_stack();
        match stack.set_active(name) {
            Ok(()) => self.active_layer.set(stack.active().to_owned()),
            Err(err) => show_message(t!("layers.title"), err.to_string()),
        }
    }

    /// Creates a new user layer as one undo step and activates it for drawing (H.14).
    pub(crate) fn add_user_layer(self, typed: &str) {
        let mut stack = self.layer_stack();
        match stack.add_user_layer(typed) {
            Ok(change) => {
                if let Change::SetLayer { ref name, .. } = change {
                    self.active_layer.set(name.clone());
                }
                self.editing
                    .apply(&Operation::new(t!("layers.add_edit"), vec![change]));
            }
            Err(err) => show_message(t!("layers.title"), err.to_string()),
        }
    }

    /// Tags newly created annotations with the active layer, refusing if that layer is locked or hidden (H.14).
    pub(super) fn place_changes(self, changes: Vec<Change>) -> Vec<Change> {
        if changes.is_empty() {
            return changes;
        }
        let stack = self.layer_stack();
        match stack.place_new(&changes) {
            Ok(tagged) => tagged,
            Err(err) => {
                show_message(t!("layers.title"), err.to_string());
                Vec::new()
            }
        }
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
    let stack = LayerStack::from_ocgs(&all);
    annotations
        .retain(|annotation| layers::shown(annotation, &all) && stack.is_visible(annotation));
    annotations
}

/// Filters `targets` on `page` so annotations on locked or hidden layers are excluded from selection (H.14).
pub(super) fn filter_selectable_targets(
    state: AnnotateState,
    page: usize,
    targets: &[Target],
) -> Vec<Target> {
    let stack = state.layer_stack();
    let annots = state.page(page);
    stack.filter_selection(targets, &annots)
}

/// The Layers section of the annotation list: the 4 fixed kind layers plus user layers (active, shown/hidden,
/// count, lock), a new-layer input, and "One layer per author".
pub(super) fn layers_section(
    state: AnnotateState,
    all: Memo<Vec<Layer>>,
    count: impl Fn(&str) -> usize + Copy + 'static,
) -> impl IntoElement {
    let new_name = signal(String::new());
    let create_layer = move || {
        let name = untracked(|| new_name.get());
        if !name.trim().is_empty() {
            state.add_user_layer(&name);
            new_name.set(String::new());
        }
    };
    let submit_layer = create_layer;
    let header = view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().child(caption("layers.title")))
        .child(action_button("layers.by_author", Tone::Plain, move || {
            state.layers_by_author();
        }));
    let add_row = view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view().flex_1().child(
                field(input().value(move || new_name.get()))
                    .on_change(move |text: &str| new_name.set(text.to_owned()))
                    .on_submit(move |_: &str| submit_layer())
                    .placeholder(t!("layers.new_placeholder"))
                    .label(t!("layers.add")),
            ),
        )
        .child(action_button("layers.add", Tone::Plain, create_layer));
    let lines = dynamic(move || {
        let stack = LayerStack::from_ocgs(&all.get());
        let layers = stack.to_mupdf_layers();
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
        .child(add_row)
        .child(lines)
        .child(view().h(px(1.0)).bg(move || theme().border))
}

/// One layer: active-for-drawing indicator, whether it is shown (fixed while locked), its name, count, and its lock.
fn layer_line(
    state: AnnotateState,
    layer: Layer,
    count: impl Fn(&str) -> usize + Copy + 'static,
) -> View {
    let (visible, locked) = (layer.visible, layer.locked);
    let held = count(&layer.name);
    let layer_name_for_active = layer.name.clone();
    let layer_name_for_color = layer.name.clone();
    let layer_name_for_label = layer.name.clone();
    let is_active = move || {
        state
            .active_layer
            .with(|active| active == &layer_name_for_active)
    };
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
        let layer_name = layer.name.clone();
        view().child(
            switch()
                .checked(move || visible)
                .accessibility_label(t!("layers.show", name = layer.name.clone()))
                .on_change(move |on: &bool| {
                    let mut stack = state.layer_stack();
                    let vis = if *on {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                    if let Ok(change) = stack.set_visibility(&layer_name, vis) {
                        state.edit_layer(t!("layers.show_edit"), change);
                    }
                })
                .track_class(SWITCH_TRACK)
                .thumb_class(SWITCH_THUMB),
        )
    };
    let lock_name = layer.name.clone();
    let lock_display = layer.name.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .min_h(px(32.0))
        .child(shown_control)
        .child(
            toggle()
                .pressed(is_active)
                .accessibility_label(t!("layers.activate", name = layer_name_for_label.clone()))
                .on_change(move |_: &bool| state.set_active_layer(&layer_name_for_label))
                .flex_1()
                .h(px(26.0))
                .px_2()
                .rounded(px(5.0))
                .items_center()
                .justify_start()
                .hover(|s| s.bg(move || theme().sidebar_accent))
                .pressed_style(|s| s.bg(move || theme().sidebar_accent))
                .child(
                    text(layer.name.clone())
                        .text_size(px(13.0))
                        .text_color(move || {
                            if !visible {
                                theme().muted_foreground
                            } else if state
                                .active_layer
                                .with(|active| active == &layer_name_for_color)
                            {
                                theme().primary
                            } else {
                                theme().sidebar_foreground
                            }
                        }),
                ),
        )
        .child(
            text(held.to_string())
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            toggle()
                .pressed(move || locked)
                .accessibility_label(t!("layers.lock", name = lock_display.clone()))
                .on_change(move |on: &bool| {
                    let mut stack = state.layer_stack();
                    let next = if *on {
                        LockState::Locked
                    } else {
                        LockState::Unlocked
                    };
                    if let Ok(change) = stack.set_lock(&lock_name, next) {
                        state.edit_layer(t!("layers.lock_edit"), change);
                    }
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
/// stack's layers to put it on with one press (pressing its own takes it off).
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
    let names: Vec<String> = state
        .layer_stack()
        .to_mupdf_layers()
        .into_iter()
        .map(|layer| layer.name)
        .collect();
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
