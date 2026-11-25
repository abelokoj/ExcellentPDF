//! Selecting annotations on the pages and editing them there (A.13): a click selects the annotation under it (Ctrl
//! adds it or takes it out), a marquee selects what lies inside it, and the canvas reports drags on the selection's
//! grips (`annotDrag`) and marquees (`annotMarquee`). Each move, resize, nudge, cut, paste or deletion is one step of
//! undo. The annotation clipboard is the app's own, shared by its documents, with each annotation's note put on the
//! system clipboard as text for other apps. The model is `excellent-features`' `annotate::edit`.

use std::sync::Mutex;

use excellent_core::annot::Annotation;
use excellent_core::edit::{Change, Operation};
use excellent_features::annotate::edit::{self, Clip, Drag, Grip, Marquee};
use excellent_features::annotate::notes::Target;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::{AnnotateState, author};

/// The annotation clipboard, shared by every document and window.
///
/// shortcut: the app's own, not the system's; pasting between two running copies of the app, or the image fallback
/// the plan names, needs a registered clipboard format (upgrade with NSH's custom clipboard formats).
static CLIP: Mutex<Option<Clip>> = Mutex::new(None);

impl AnnotateState {
    /// The canvas's `selected` prop: the selected annotations as they are now, their bounds and grips; `null` for
    /// none. It is read again after every edit, undo or redo.
    pub(crate) fn selected_prop(self) -> JsonValue {
        self.editing.revision();
        let selected = self.selected.get();
        let Some(page) = selected.first().map(|target| target.page) else {
            return JsonValue::Null;
        };
        let items: Vec<JsonValue> = untracked(|| self.page(page))
            .iter()
            .filter(|annotation| selected.iter().any(|target| target.id == annotation.id))
            .filter_map(|annotation| {
                let bounds = edit::bounds(annotation)?;
                let grips: Vec<JsonValue> = edit::grips(annotation)
                    .into_iter()
                    .map(|(grip, at)| serde_json::json!([grip.name(), grip.index(), at.x, at.y]))
                    .collect();
                Some(serde_json::json!({
                    "id": annotation.id,
                    "bounds": [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
                    "grips": grips,
                    "locked": !edit::can_move(annotation),
                }))
            })
            .collect();
        serde_json::json!({ "page": page, "items": items })
    }

    /// Whether any annotation is selected.
    pub(crate) fn has_selection(self) -> bool {
        self.selected.with(|selected| !selected.is_empty())
    }

    /// Selects nothing.
    pub(crate) fn clear_selection(self) {
        if untracked(|| self.has_selection()) {
            self.selected.set(Vec::new());
        }
    }

    /// A click hit `hit` (or nothing): with Ctrl (`add`) it joins the selection or leaves it; otherwise it is the
    /// selection. Returns whether the click was Ctrl's, which opens no note.
    pub(super) fn select_press(self, hit: Option<Target>, add: bool) -> bool {
        match (hit, add) {
            (Some(target), true) => {
                let selection = untracked(|| self.selected.get());
                self.selected.set(edit::toggled(&selection, target));
                true
            }
            (Some(target), false) => {
                self.selected.set(vec![target]);
                false
            }
            (None, _) => {
                self.clear_selection();
                false
            }
        }
    }

    /// The canvas reported a drag on the selection (`annotDrag`: `{page, id, grip, index, from, to}`): a move takes
    /// every selected annotation along; a corner or a vertex reshapes the one dragged. One step of undo.
    pub(super) fn selection_dragged(self, data: &JsonValue) {
        let Some(drag) = Drag::parse(data) else {
            log::warn!("annotDrag: a drag that doesn't parse: {data}");
            return;
        };
        let selected = untracked(|| self.selected.get());
        let changes: Vec<Change> = self
            .page(drag.page)
            .iter()
            .filter_map(|a| {
                let chosen = selected.iter().any(|t| t.id == a.id);
                edit::change(a, drag.applied(a, chosen)?)
            })
            .collect();
        let name = match drag.grip {
            Grip::Move => t!("select.move"),
            Grip::Corner(_) => t!("select.resize"),
            Grip::Vertex(_) => t!("select.reshape"),
        };
        self.apply(name, changes);
    }

    /// The canvas drew a marquee (`annotMarquee`: `{page, rect, add}`): what lies wholly inside it is selected, or
    /// added to the selection with Ctrl. One selected opens its note.
    pub(super) fn marquee(self, data: &JsonValue) {
        let Some(marquee) = Marquee::parse(data) else {
            log::warn!("annotMarquee: a marquee that doesn't parse: {data}");
            return;
        };
        let taken = marquee.takes(&self.page(marquee.page));
        let selection = if marquee.add {
            taken
                .into_iter()
                .fold(untracked(|| self.selected.get()), |selection, target| {
                    if selection.contains(&target) {
                        selection
                    } else {
                        edit::toggled(&selection, target)
                    }
                })
        } else {
            taken
        };
        if let [only] = selection.as_slice() {
            self.open_note(only);
        }
        self.selected.set(selection);
    }

    /// A page command while annotations are selected: `copy` copies them, the arrow keys' commands nudge them.
    /// Returns whether it was taken; otherwise the canvas has it.
    pub(crate) fn selection_command(self, name: &str) -> bool {
        if !untracked(|| self.has_selection()) {
            return false;
        }
        let (dx, dy) = match name {
            "copy" => {
                self.copy();
                return true;
            }
            "lineDown" => (0.0, edit::NUDGE),
            "lineUp" => (0.0, -edit::NUDGE),
            "stepLeft" => (-edit::NUDGE, 0.0),
            "stepRight" => (edit::NUDGE, 0.0),
            _ => return false,
        };
        let changes = self
            .selection()
            .iter()
            .filter_map(|a| edit::change(a, edit::moved(a, dx, dy)?))
            .collect();
        self.apply(t!("select.nudge"), changes);
        true
    }

    /// Copies the selected annotations: whole onto the app's clipboard, their notes onto the system's as text.
    pub(crate) fn copy(self) {
        let clip = Clip::of(self.selection());
        if clip.is_empty() {
            return;
        }
        drop(set_clipboard(clip.text())); // the write happens whether or not the answer is awaited
        *CLIP.lock().expect("clipboard lock poisoned") = Some(clip);
    }

    /// Copies the selected annotations and deletes them, the locked ones kept: one step of undo.
    pub(crate) fn cut(self) {
        self.copy();
        self.delete_selected_named(t!("select.cut"));
    }

    /// Deletes the selected annotations, the locked ones kept: one step of undo.
    pub(crate) fn delete_selected(self) {
        self.delete_selected_named(t!("select.delete"));
    }

    fn delete_selected_named(self, name: String) {
        let changes = edit::delete(&self.selection());
        if !changes.is_empty() {
            self.close_note();
            self.apply(name, changes);
            self.selected.set(Vec::new());
        }
    }

    /// Pastes the annotation clipboard onto page `page` of the shown document, as one step of undo, and selects
    /// what it pasted.
    pub(crate) fn paste(self, page: usize) {
        let pasted = match &*CLIP.lock().expect("clipboard lock poisoned") {
            Some(clip) => clip.paste(page, edit::PASTE_OFFSET, &author()),
            None => return,
        };
        let targets: Vec<Target> = pasted
            .iter()
            .map(|a| Target {
                page: a.page,
                id: a.id.clone(),
            })
            .collect();
        let changes = pasted.into_iter().map(Change::CreateAnnotation).collect();
        if self.apply(t!("select.paste"), changes) {
            self.selected.set(targets);
        }
    }

    /// The selected annotations as the shown document has them now.
    fn selection(self) -> Vec<Annotation> {
        let selected = untracked(|| self.selected.get());
        let Some(page) = selected.first().map(|target| target.page) else {
            return Vec::new();
        };
        self.page(page)
            .into_iter()
            .filter(|a| selected.iter().any(|target| target.id == a.id))
            .collect()
    }

    /// Page `page`'s annotations in the shown document; none when it can't be read.
    fn page(self, page: usize) -> Vec<Annotation> {
        let Some(document) = self.editing.document() else {
            return Vec::new();
        };
        let Ok(shared) = document.mupdf() else {
            return Vec::new();
        };
        let mupdf = shared.lock().expect("document lock poisoned");
        // What a hidden layer holds can't be selected on the page (A.22).
        super::layers::shown_on_page(&mupdf, page)
    }

    /// Makes `changes` one step of undo named `name`; nothing for none. Returns whether it was made.
    fn apply(self, name: String, changes: Vec<Change>) -> bool {
        !changes.is_empty() && self.editing.apply(&Operation::new(name, changes))
    }
}
