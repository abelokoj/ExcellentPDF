# Ph3: 17/49 | A.4: Notes on highlights

**Tag:** A.4; **Priority:** P0; **Phase:** 3; **Needs:** A.1; **Code:** `excellent-features::annotate::notes`,
`excellent-mupdf-sys/csrc/shim.c` (`ex_popup_rect`), `excellent-canvas::page::canvas::markbar`,
`excellent-app::annotate::note`
**Plan:** §27 entry `#### Ph3: 17/49 | A.4`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A highlight says "this matters" but not why. Readers want to write the why beside it, and see it again in any PDF
reader.

## User story

As a reader, I want to click a markup and write a note on it, beside the page, so that my thoughts stay with the text
and other readers show them as comments.

## UI sketch

- A click on a markup opens its note in the inspector (right dock), which is shown and brought to the front: a dot in
  the markup's colour and "Highlight on page 3"; the marked text set off by a rule in that colour; the note, several
  lines; Delete note (when it had one) and Done; a line saying it is kept in the PDF.
- The markup bar: a Note tool (a speech bubble) after Insert. It highlights the selection and opens the note.
- The command `annotate.note` is in the palette and the Annotate toolbar; its page key is `Shift+N`.

## Mode

All modes; the toolbar button is in Annotate mode's toolbar.

## Speed and size impact

None at startup. A click reads the clicked page's annotations once.

## Design

- **Model** (`annotate::notes`, pure): `Target`, `parse_press`, `markup_at`, `marked`, `normalize`, `write` (one
  `UpdateAnnotation`, none when unchanged), `first_line`.
- **Shim**: writing a markup with contents and no `/Popup` adds one (`pdf_set_annot_popup`).
- **Canvas**: `pagePress {page, x, y}` on a plain click on the pages; the markup event's `note` flag.
- **App**: `AnnotateState` keeps the open note and its draft; the window reveals the inspector for each note opened.

## Acceptance tests

- [x] `a_note_on_a_highlight_gets_a_popup_and_survives_save_and_reopen` (the popup other readers open).
- [x] `a_click_opens_the_markup_under_it_the_top_one_first`, `a_press_is_read`.
- [x] `a_note_is_written_as_one_change_only_when_it_changes`, `the_markup_just_made_is_the_one_noted`.
- [x] `the_first_line_is_cut_at_a_whole_character`, `the_panel_shows_the_markup_s_colour_and_text`.
- [ ] Notes open in other readers' comment panes: not yet checked in Acrobat by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- The editor is the inspector; the plan's separate popover is not built.
- An undo while a note is open is not shown in the editor until the note is opened again.
