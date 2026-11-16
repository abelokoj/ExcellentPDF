# Ph3: 26/49 | A.13: Select, move, resize, copy, paste and delete

**Tag:** A.13; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::edit`,
`excellent-canvas::page::canvas::select`, `excellent-app::annotate::select`
**Plan:** §27 entry `#### Ph3: 26/49 | A.13`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Annotations could be made and their notes written, but not picked up again: not moved, resized, copied to another
document or deleted from the page.

## User story

As a reviewer, I want to click a mark I made, drag it where it belongs, resize it, copy it into the next document, or
delete it, as I would a shape in any drawing app.

## UI sketch

- Click an annotation: it is outlined in blue and its note opens in the inspector. Ctrl-click adds one or takes it
  out. A drag on an empty part of the page draws a marquee; what lies wholly inside is selected.
- Selected boxes, ink drawings and callouts show four corner grips; lines and polygons show a grip on each point.
  The pointer turns to a grab hand over the selection. The outline follows a drag until the button comes up.
- Arrow keys nudge by one point. Delete, Ctrl+X, Ctrl+C and Ctrl+V delete, cut, copy and paste; Cut, Paste and
  Delete are also on the Annotate toolbar and in the palette. Escape selects nothing.

## Mode

All modes; the toolbar buttons are Annotate's.

## Speed and size impact

None at startup. A drag redraws only the outline; the page is drawn again once, after the edit.

## Design

- **Model** (`annotate::edit`, pure): grips, drags (`dragged`, `moved`, `Drag::applied`), `change` (A.12's lock),
  `delete`, `in_marquee`, `toggled`, `hit`, and `Clip` with its JSON format.
- **Canvas** (`select.rs`): the `selected` prop, grips and outline drawing, drag and marquee tracking; the
  `annotDrag` and `annotMarquee` events, and `add` on `pagePress`. `Host::ctrl_down` (Windows reads it).
- **App** (`annotate/select.rs`): the selection signal, its prop (read again after every edit), the event handlers,
  `selection_command` (copy and the arrow keys while something is selected), cut, paste and delete.

## Acceptance tests

- [x] `pasting_into_another_document_keeps_type_style_and_contents` (the Done when).
- [x] `a_move_a_corner_and_a_vertex_drag_reshape_the_annotation`, `ink_and_callouts_scale_with_their_box`.
- [x] `text_markup_and_locked_annotations_keep_still`.
- [x] `a_marquee_takes_what_lies_wholly_inside_it_and_ctrl_click_toggles`.
- [x] `the_canvas_s_drags_and_marquees_parse_and_a_move_takes_the_selection_along`.
- [x] `the_selected_prop_reads_its_items_and_grips`, `a_drag_s_outline_follows_the_grip`.
- [ ] Tried by hand: not yet.

## Open

- The clipboard is the app's own (DD12): no image fallback, and two running copies don't share it.
- Ctrl is read on Windows only; GTK and macOS hosts report it as up until they implement `ctrl_down`.
- Editing several selected annotations at once in the inspector (A.12 item 2) is not built.
- Dragging past the window's edge doesn't scroll.
