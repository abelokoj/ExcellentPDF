# Ph3: 14/49 | A.1: Text markup: highlight, underline, squiggly, strikeout

**Tag:** A.1; **Priority:** P0; **Phase:** 3; **Needs:** K.13, K.14; **Code:** `excellent-features::annotate::markup`,
`excellent-canvas::page::canvas::markbar`, `excellent-app::annotate`
**Plan:** §27 entry `#### Ph3: 14/49 | A.1`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reading to study means marking: a passage to come back to, a definition, a sentence that is wrong. The reader needs
the four standard text markups, saved in the PDF where any other reader shows them.

## User story

As a reader, I want to select text and highlight, underline or strike it out in a colour, so that what matters
stands out the next time I, or anyone I send the file to, read it.

## UI sketch

- Select text in a PDF: a small bar floats under the selection, or over it when there's no room below. Four
  buttons (highlight, underline, squiggly underline, strikeout), a thin rule, then five colour dots; the chosen one
  is ringed.
- A markup button marks the selection and the selection goes. A dot chooses the colour for the next markup.
- The commands are in the palette and on Annotate mode's toolbar; the page keys are `a`, `u`, `Shift+U` and `x`.
- Undo takes a markup back like any other edit.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

Nothing at startup. The bar is a few shapes drawn with the selection. Marking reads the page's annotations once and
applies one operation; the changed page is drawn again.

## Design

- **Model** (`annotate::markup`, pure): `mark(marking, page, quads, quote, existing)` gives the changes. A new
  markup, or an update of the one it touches (same kind, same colour, same line, a gap under a quarter of the line's
  height) with the runs that touch joined, and deletes for further markups it bridges. `Request::parse` reads the
  canvas's event.
- **Canvas** (`markbar`): the bar is drawn like the hover preview (R.25), a shadowed rounded frame in the chrome's
  tone, with icons made from the host's shapes. `mark(kind)` waits for unread pages' text like copy does, then sends
  `markup` with the quads in page points and the quote.
- **App** (`annotate`): reads the page's annotations, builds the operation and applies it through `Editing`, so it
  is logged (K.14) and saved (K.15). The canvas gets an `edits` prop that moves with every edit, undo or redo; it
  then drops the display lists and tiles of the pages whose revision moved.

## Acceptance tests

- [x] `marking_text_makes_one_markup_over_its_lines`, `nothing_selected_marks_nothing`.
- [x] `extending_a_highlight_merges_into_it`, `bridging_two_highlights_folds_them_into_one`.
- [x] `another_colour_or_line_stacks_instead`.
- [x] `the_canvas_request_is_read`, `the_bar_s_cells_sit_side_by_side_inside_it`.
- [ ] Markups show correctly in Acrobat, PDF-XChange and pdf.js (K.26).
- [ ] They stay on the text under every rotation (tried by hand).

## Open

- Not tried by hand; the cross-reader check (K.26) is not done.
- The bar is drawn in the canvas, so screen readers don't see it; its four commands are in the palette and have
  keys.
- A changed page shows its paper for a moment until its new tiles arrive.
- The merged markup keeps the first markup's anchor rather than quoting the joined text.
- The author is the system's user name until a setting exists.
