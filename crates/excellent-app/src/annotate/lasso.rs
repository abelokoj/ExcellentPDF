//! The lasso (H.6), the app's side: the lasso tool (the canvas's `lasso` prop), the loop the canvas reports
//! (`annotLasso`), and what is done to a selection whole: resized from its own box, turned, made thinner or thicker,
//! recoloured, duplicated. A selection of several shows in the inspector with those controls. Each edit is one step
//! of undo. The model is `excellent-features`' `annotate::lasso`.

use excellent_core::edit::Change;
use excellent_features::annotate::colors::{Tool, ToolStyle};
use excellent_features::annotate::edit::{self, Clip, Drag, Grip};
use excellent_features::annotate::lasso::{self as model, GROUP, Looped};
use excellent_features::annotate::notes::Target;
use nsh_base::{button, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::{AnnotateState, author};
use crate::bookmarks::caption;
use crate::tabs::{Tone, action_button};

/// How far the turn commands turn a selection, in degrees.
const QUARTER: f32 = 90.0;

impl AnnotateState {
    /// Turns the lasso on, putting the pen's eraser away, or off again.
    pub(crate) fn toggle_lasso(self) {
        let on = !untracked(|| self.lasso.get());
        self.lasso.set(on);
        if on {
            self.eraser.set(None);
        }
    }

    /// The canvas's `lasso` prop: whether the lasso is on.
    pub(crate) fn lasso_prop(self) -> JsonValue {
        JsonValue::from(self.lasso.get())
    }

    /// The canvas reported a loop (`annotLasso`): what it encloses is selected, or added to the selection with Ctrl.
    pub(super) fn looped(self, data: &JsonValue) {
        let Some(looped) = Looped::parse(data) else {
            log::warn!("annotLasso: a loop that doesn't parse: {data}");
            return;
        };
        let taken = super::layers::filter_selectable_targets(
            self,
            looped.page,
            &looped.takes(&self.page(looped.page)),
        );
        let selection = if looped.add {
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
        self.selected.set(selection);
    }

    /// A drag on the selection's own box ([`GROUP`]): a corner scales everything selected. Returns whether it was
    /// one; a move inside the box goes the way of any move.
    pub(super) fn group_dragged(self, drag: &Drag) -> bool {
        let Grip::Corner(index) = drag.grip else {
            return false;
        };
        if drag.id != GROUP {
            return false;
        }
        self.apply(
            t!("select.resize"),
            model::group_resized(&self.selection(), index, drag.to),
        );
        true
    }

    /// The selection's own box as a `selected` prop item, for a selection of several: its corners are grips that scale
    /// them all. `None` for fewer than two.
    pub(super) fn group_item(self) -> Option<JsonValue> {
        let selection = self.selection();
        if selection.len() < 2 {
            return None;
        }
        let bounds = model::group_bounds(&selection)?;
        let movable = selection.iter().any(edit::can_move);
        let grips: Vec<JsonValue> = edit::corners(bounds)
            .into_iter()
            .enumerate()
            .map(|(index, at)| serde_json::json!(["corner", index, at.x, at.y]))
            .collect();
        Some(serde_json::json!({
            "id": GROUP,
            "bounds": [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
            "grips": if movable { grips } else { Vec::new() },
            "locked": !movable,
        }))
    }

    /// Turns the selection `degrees` clockwise about its centre; nothing unless each can turn.
    pub(crate) fn rotate_selection(self, degrees: f32) {
        self.apply(
            t!("select.rotate"),
            model::rotated(&self.selection(), degrees),
        );
    }

    /// Multiplies the selection's widths by `factor` ([`model::THINNER`], [`model::THICKER`]).
    pub(crate) fn scale_selection_width(self, factor: f32) {
        self.apply(
            t!("select.width"),
            model::width_scaled(&self.selection(), factor),
        );
    }

    /// Gives the selection the colour `rgb`.
    fn recolor_selection(self, rgb: [f32; 3]) {
        self.apply(
            t!("select.recolor"),
            model::recolored(&self.selection(), rgb),
        );
    }

    /// Duplicates the selection a little down and right, and selects the copies; the clipboard keeps what it had.
    pub(crate) fn duplicate_selection(self) {
        let selection = self.selection();
        let Some(page) = selection.first().map(|a| a.page) else {
            return;
        };
        let copies = Clip::of(selection).paste(page, edit::PASTE_OFFSET, &author());
        let targets: Vec<Target> = copies
            .iter()
            .map(|a| Target {
                page: a.page,
                id: a.id.clone(),
            })
            .collect();
        let changes = copies.into_iter().map(Change::CreateAnnotation).collect();
        if self.apply(t!("select.duplicate"), changes) {
            self.selected.set(targets);
        }
    }
}

/// The inspector for a selection of several: how many, a colour for all, and turning, width and duplicating.
pub(super) fn selection_panel(state: AnnotateState) -> View {
    let count = move || state.selected.with(Vec::len);
    let swatches = super::colors::palette(state.live)
        .swatches(Tool::Shape)
        .into_iter()
        .fold(view().flex_row().flex_wrap().gap_2(), |row, color| {
            row.child(
                button()
                    .accessibility_label(t!(
                        "inspector.swatch",
                        color = excellent_features::annotate::colors::hex(color)
                    ))
                    .on_press(move || state.recolor_selection(ToolStyle::new(color, 1.0).rgb()))
                    .w(px(28.0))
                    .h(px(28.0))
                    .rounded_full()
                    .bg(Hsla::from(rgb(color)))
                    .border_2()
                    .border_color(move || theme().border)
                    .focus_visible(|s| s.border_color(move || theme().ring)),
            )
        });
    let actions = view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(action_button(
            "command.rotate_left",
            Tone::Plain,
            move || {
                state.rotate_selection(-QUARTER);
            },
        ))
        .child(action_button(
            "command.rotate_right",
            Tone::Plain,
            move || {
                state.rotate_selection(QUARTER);
            },
        ))
        .child(action_button("command.thinner", Tone::Plain, move || {
            state.scale_selection_width(model::THINNER);
        }))
        .child(action_button("command.thicker", Tone::Plain, move || {
            state.scale_selection_width(model::THICKER);
        }))
        .child(action_button(
            "command.duplicate_selection",
            Tone::Plain,
            move || state.duplicate_selection(),
        ));
    view()
        .gap_3()
        .p_4()
        .child(
            text(move || t!("lasso.selected", count = count()))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(caption("inspector.color"))
        .child(swatches)
        .child(caption("lasso.arrange"))
        .child(actions)
}
