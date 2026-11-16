# Ph3: 15/49 | A.2: Replace and insert text

**Tag:** A.2; **Priority:** P0; **Phase:** 3; **Needs:** A.1; **Code:** `excellent-features::annotate::replace`,
`excellent-canvas::page::canvas::markbar`, `excellent-app::annotate`, the shim's `ex_annot_group`
**Plan:** §27 entry `#### Ph3: 15/49 | A.2`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reviewing a draft means proposing changes to its words: this phrase should read that, a word is missing here. The
reader needs Acrobat's two proposal markups, saved so that the author's reader shows them as edits to accept.

## User story

As a reviewer, I want to select text and propose its replacement, or put the cursor somewhere and propose an
insertion, so that the author sees exactly what I suggest, where.

## UI sketch

- The markup bar under selected text gains, after a rule, Replace (struck text over a caret) and Insert (a caret
  between words).
- Replace opens a small dialog: the struck text shown struck, then "Replace with", starting from the selected text,
  chosen whole. Enter or the Replace button proposes; Escape, Cancel or a press outside cancels.
- Insert opens the same dialog with an empty "Text to insert"; the caret goes after the selection, or where the
  last click in the text left the cursor.
- The commands are in the palette and on Annotate mode's toolbar; the page keys are `E` and `Insert`.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

Nothing at startup. A proposal is one operation of one or two annotations; the changed page is drawn again.

## Design

- **Model** (`annotate::replace`, pure): `insert` gives one caret holding the text; `replace` gives the caret, then
  a strikeout whose `group` names it. Blank text proposes nothing. `Request::parse` reads the canvas's event; the
  caret's rectangle is MuPDF's 20 × 14 pt centred on the foot of the edge the text goes in at.
- **Core and shim:** `Annotation::group` is the `/NM` of the annotation it is grouped with. The shim writes it as
  `/IRT` (the target found on the same page) with `/RT /Group`, removes both when ungrouped, and leaves a reply's
  `/IRT` alone. The operation log carries it; logs from before have none.
- **Canvas** (`markbar`): the bar's tools are the four markups and the two proposals. A plain click in the text
  keeps its caret for an insert. The event `textEdit` sends the mode, colour, page, quads and quote of the struck
  text, the foot of the caret's edge and the selected text.
- **App** (`annotate`): `AnnotateState` keeps the request while its dialog asks for the text, then applies the
  operation through `Editing`.

## Acceptance tests

- [x] `inserting_text_makes_a_caret_holding_it`, `replacing_text_groups_a_strikeout_with_its_caret`.
- [x] `the_caret_sits_on_the_line_s_foot`, `the_canvas_request_is_read`.
- [x] `a_grouped_pair_survives_save_and_reopen` (the group through MuPDF, the shim and back).
- [x] `the_bar_s_cells_sit_side_by_side_inside_it`.
- [ ] Acrobat shows the pair as one "replace text" comment.

## Open

- Not tried by hand; not checked in Acrobat.
- A selection across pages proposes replacing only its last page's part.
- The caret is drawn upright on rotated text: MuPDF's caret appearance has no angle.
- The bar is drawn in the canvas, so screen readers don't see it; its commands are in the palette and have keys.
