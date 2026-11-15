# Ph1: 32/60 | B.2: Rename and delete

**Tag:** B.2; **Priority:** P0; **Phase:** 1; **Needs:** B.1; **Code:** as B.1
**Plan:** §27 entry `Ph1: 32/60 | B.2`.

## Problem

A wrong or clumsy outline title, or a stray entry, could not be fixed.

## User story

As a reader, I want to rename an entry in place and delete entries I don't need, and undo either, so that the
outline says what I need.

## UI sketch

- **Rename outline entry**: F2 while the outline has the keyboard, the row's context menu, or the palette. The title
  becomes a field with the text selected; Return or leaving the field keeps it, Escape drops it. An empty or
  unchanged title changes nothing.
- **Delete outline entry**: Delete while the outline has the keyboard, the context menu, or the palette. The entries
  under it go with it; Undo (Ctrl+Z) brings them all back.

## Mode

Every reading mode, PDF files.

## Speed and size impact

None at startup. One operation per rename or delete.

## Design

- Rename: `fz_outline_iterator_update` with the new title, the URI and open state kept.
- Delete: `plan_delete` lists the targets from the last in document order, skipping entries already under a deleted
  one, so the paths stay right as each goes; all in one operation.

## Acceptance tests

- [x] `deletes_go_from_the_last_and_skip_entries_already_going`.
- [x] Undo restores the exact previous outline: `outline_edits_are_undone_and_redone_one_operation_at_a_time`
  (`excellent-mupdf`).
- [ ] Promoting the children one level before deleting (step 2's other choice).
