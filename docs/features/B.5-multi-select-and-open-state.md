# Ph1: 35/60 | B.5: Multi-select, undo and open state

**Tag:** B.5; **Priority:** P0; **Phase:** 1; **Needs:** B.2, B.3; **Code:** as B.1
**Plan:** §27 entry `Ph1: 35/60 | B.5`.

## Problem

Tidying a long outline one entry at a time was slow, and the file forgot which entries the reader had opened.

## User story

As a reader, I want to mark several entries and delete or move them at once, undo that in one step, and have the
file remember what I opened, so that tidying an outline is quick.

## UI sketch

- Ctrl+press (⌘ on a Mac) marks or unmarks an entry; Shift+press marks the shown entries from the last one pressed.
  Marked rows have the accent's tint and a bar at their start, so they read as a group without relying on colour.
- Delete and the moves act on the marked entries (else the selected one), each as one undo step.
- A plain press clears the marks; Escape clears them and gives the keyboard to the pages.

## Mode

Every reading mode, PDF files.

## Speed and size impact

None at startup.

## Design

- `top_most` drops marked entries already under another marked one; `plan_delete` and `plan_shift` take the rest.
- Open state: written only as part of an edit (`open_changes`: entries with children whose shown state differs from
  the file's), so expanding and collapsing alone never marks the file changed.

## Acceptance tests

- [x] `entries_move_one_place_as_a_group`: a group's move is planned as steps the app makes in one operation.
- [ ] A multi-entry move undoes in one step, end to end (checked by hand; no test applies a planned move to a PDF).
- [x] `only_open_state_the_reader_changed_is_written`.
- [ ] Restyling several entries at once (colour and style come with outline styling).
- [ ] Open state saved on its own when the reader changes it, without another edit (step 2 as written).
