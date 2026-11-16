# Ph3: 25/49 | A.12: Annotation inspector

**Tag:** A.12; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::inspector`,
`excellent-app::annotate::inspector`, `excellent-app::annotate::note`
**Plan:** §27 entry `#### Ph3: 25/49 | A.12`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

An annotation's properties were spread over its kind's own controls, and some (who made it, when, how opaque, whether
it may change) could not be seen or set at all.

## User story

As a reviewer, I want one panel that shows everything about the annotation I clicked, lets me change it, and lets me
lock what is final, so I can tidy a document's markup without hunting for each setting.

## UI sketch

- The inspector (the right panel, opened by a click on an annotation) shows, under its note and its kind's controls:
  Author (a text field, saved on Enter or when it loses focus), Colour (five swatches from the palette in use, the
  chosen one ringed), Opacity (25, 50, 75, 100 %), a rule, then Created and Changed (UTC) and a Locked switch.
- Locked, the kind's controls, author, colour, opacity and Delete give way to a short note saying why; the note and
  the switch stay.

## Mode

All modes.

## Speed and size impact

None at startup. While the inspector is open, the annotation is read again after each edit (one page's annotations).

## Design

- **Model** (`annotate::inspector`, pure): `Details::of`, the setters (each `Option<Change>`, none when nothing
  changes), `allows(before, change)` for locks, and `date` (UTC; Hinnant's civil-from-days).
- **Core**: `Annotation.locked`, read from and written to PDF's `/F` Locked bit by the shim, other flags kept.
- **App**: `AnnotateState::change_note` refuses what `allows` refuses; `delete_open` leaves a locked one. The rows
  are a `dynamic` on the editing revision.

## Acceptance tests

- [x] `each_property_set_here_is_one_change_and_none_when_it_is_the_same`.
- [x] `a_locked_annotation_takes_only_its_note_and_its_lock`, `a_text_box_s_colour_is_not_the_inspector_s`.
- [x] `dates_read_as_utc_and_a_missing_one_is_empty`.
- [x] `a_lock_and_an_author_keep_through_save_and_reopen_and_other_flags_stay`.
- [ ] Tried by hand: not yet.

## Open

- Category, tags and layer rows come with A.17, A.18 and A.22.
- Multi-selection (item 2) needs A.13's selection.
- Dates are shown in UTC until the app has a date and time library.
- The lock is enforced in the inspector; A.13's move and resize will check it too. Merging overlapping highlights
  (A.1) does not check it yet.
