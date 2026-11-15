# Ph1: 4/60 | K.14: Annotation model and operation log

**Tag:** K.14; **Priority:** P0; **Phase:** 1 (design), 3 (build); **Needs:** K.5; **Code:** `excellent-core::{edit,
oplog}`, `excellent-mupdf::edit`, `excellent-app::editing`
**Plan:** §27 entry `Ph1: 4/60 | K.14`, Appendix D.1.2.

This is the operation log and the undo it carries, built now because outline editing (B.1 to B.5) is the first thing
that changes a document. The annotation model (step 1) comes with the annotation tags in Phase 3.

## Problem

Nothing could change a document safely: an edit had no undo, and a crash before a save lost it.

## User story

As a reader editing a PDF, I want every change to be one step I can undo and redo, and kept even if the app crashes,
so that I never lose work and never fear trying something.

## UI sketch

- **Undo** (Ctrl+Z) and **Redo** (Ctrl+Shift+Z; not Ctrl+Y, which Enhanced gives to zoom), in the palette's Edit
  group. A text field with the focus keeps its own undo.
- An edit that fails changes nothing and says so: "Add outline entry failed", with MuPDF's reason.

## Mode

Every mode, for PDF files. Other formats can't be edited; their outline commands say so.

## Speed and size impact

Nothing at startup: the log file is created by the first edit. Each edit appends one JSON line and syncs it (one
small write); not measured.

## Design

- `excellent-core::edit`: `Operation { name, changes }`, one step of undo however many `Change`s it holds (a move of
  several entries is one). Changes are what the log writes and what MuPDF applies.
- `excellent-mupdf::edit`: every change runs inside `pdf_begin_operation` / `pdf_end_operation` through the shim; a
  failed step is taken back, so the journal and the file never disagree. Undo and redo are `pdf_undo` / `pdf_redo`.
- `excellent-core::oplog`: one JSON line per operation, undo, redo and save, written in one call and synced before
  the edit counts. The log starts with the file's fingerprint; a half-written last line is cut off on reading.
  Logs live in the app folder's `oplog/`, named by the file's path key.
- Deviation from step 2: records carry no author yet (one local user); it comes with sync (Y.1).
- `excellent-app::editing`: applies operations to the shown document, undo and redo, and reports failures.

## Acceptance tests

- [x] `changes_read_back_as_they_were_written`, `an_unknown_change_is_not_read`.
- [x] `nothing_is_written_until_the_first_record`, `a_log_from_an_earlier_run_is_kept_for_recovery`,
  `saving_marks_the_history_saved_until_it_moves`.
- [x] `a_log_cut_anywhere_keeps_every_whole_record` (proptest).
- [x] `outline_edits_are_undone_and_redone_one_operation_at_a_time`, `a_failed_operation_changes_nothing`
  (`excellent-mupdf`); `edits_are_logged_undone_and_saved_over_the_file`,
  `a_failed_edit_changes_nothing_and_is_not_logged`.
- [ ] Model-based tests pass for 10,000 random sequences (needs the annotation model, step 1).
- [ ] Undo after reopening, where `pdf_save_journal` / `pdf_load_journal` allow it.
