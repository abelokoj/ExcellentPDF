# Ph3: 35/49 | A.22: Annotation layers by person or purpose

**Tag:** A.22; **Priority:** P1; **Phase:** 3; **Needs:** A.12; **Code:** `ecclesia-features::annotate::layers`,
`ecclesia-app::annotate::layers`, `ecclesia-core::annot` (`layer`), `ecclesia-core::edit` (`Change::SetLayer`,
`Pages`), `ecclesia-mupdf::Layer`, the shim's `ex_layers`, `ex_layer_set` and `ex_annot_layer`
**Plan:** §27 entry `#### Ph3: 35/49 | A.22`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Several reviewers mark up one document, or one reader marks it for several purposes. Everything lands on one sheet,
so a reader can't look at one reviewer's marks, or hide the working notes, without deleting them.

## User story

As a reviewer, I want each person's (or each purpose's) annotations on a layer of their own that I can show, hide and
lock, and have Acrobat's Layers panel show the same layers in the same state.

## UI sketch

- The top of the annotation list (Annotations panel, **Ctrl+Alt+L**, palette "Layers") shows **Layers**: a line per
  layer with a switch to show or hide it, its name (muted while hidden), how many annotations it holds and **Lock**.
  A locked layer shows "Shown" or "Hidden" in place of the switch. **One per author** puts every annotation on no
  layer on its author's layer (palette "Put annotations on one layer per author"). "No layers yet." when there are
  none.
- An annotation's inspector has **Layer**: a field (typing a new name makes that layer) and a chip per layer;
  pressing the chosen chip takes the annotation off.

## Mode

Annotate and Review.

## Speed and size impact

Nothing at startup. The layers are read from MuPDF's cache once per edit; showing or hiding one redraws every page,
as the change affects them all.

## Design

- **Model** (`annotate::layers`, pure): `name` (one trimmed line, at most 80 characters), `move_to` (skips locked
  annotations, replies and those already there), `by_author`, `show` and `lock` (none when nothing changes; a locked
  layer can't be shown or hidden), `shown`, `default_for` and `place_new` (a new annotation goes on its author's
  layer when the document has one), `count`.
- **Core:** `Annotation::layer`, in the file (`/OC`) and the operation log; `Change::SetLayer` writes the default
  configuration's `/OFF`, `/ON` and `/Locked`; `Operation::pages` returns `Pages::Every` for it so every page draws
  again on apply, undo and redo.
- **Engine:** the shim makes an optional content group per new name in `/OCProperties` (`/OCGs` and `/D /Order`) and
  sets the annotation's `/OC`; MuPDF's layer cache is reset after each change, undo and redo, so the page draws the
  new state. MuPDF leaves a hidden layer's annotations out of the drawing.
- **App:** `layers_section` in the annotation list, `layer_rows` in the inspector; presses, selection and the margin
  leave out what a hidden layer holds (`shown_on_page`); `Editing::apply` puts new annotations on their author's
  layer.

## Acceptance tests

- [x] On a layer, drawn; hidden, not drawn, here and after a save and reopen; undo shows it; lock is kept; off the
  layer it is drawn and the layer stays: `a_hidden_layer_hides_its_annotations_here_and_in_the_file`.
- [x] The shim's list: `the_shim_s_list_reads_as_layers`.
- [x] Model: `a_name_is_one_trimmed_line`, `moving_skips_what_is_there_locked_or_a_reply`,
  `sorting_by_author_gives_each_reviewer_a_layer`, `a_locked_layer_keeps_its_visibility`,
  `what_a_hidden_layer_holds_is_not_shown`, `a_new_annotation_goes_on_its_author_s_layer`.
- [ ] Hiding a layer here hides it in Acrobat's Layers panel: not checked (no Acrobat here).
- [ ] Tried by hand: not yet.

## Open

- Lock is the PDF's `/Locked` (readers keep the layer's visibility); it doesn't stop editing what is on it (DD20).
- A hidden layer's annotations stay in the annotation list, so they can be found and moved (DD20).
- "One per author" has no shortcut (DD20).
- Layers can't be renamed or deleted yet; a layer with nothing on it stays in the file.
