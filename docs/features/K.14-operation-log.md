# Ph1: 4/60 | K.14: Annotation model and operation log

**Tag:** K.14; **Priority:** P0; **Phase:** 1 (design), 3 (build); **Needs:** K.5; **Code:** `ecclesia-core::{annot,
edit, oplog}`, `ecclesia-mupdf::{annot, edit}`, `ecclesia-app::editing`
**Plan:** §27 entry `Ph1: 4/60 | K.14`, Appendix D.1.2.

This is the operation log and the undo it carries, built now because outline editing (B.1 to B.5) is the first thing
that changes a document. The annotation model (step 1) was added at the start of Phase 3, for the annotation tags.

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

- `ecclesia-core::edit`: `Operation { name, changes }`, one step of undo however many `Change`s it holds (a move of
  several entries is one). Changes are what the log writes and what MuPDF applies.
- `ecclesia-mupdf::edit`: every change runs inside `pdf_begin_operation` / `pdf_end_operation` through the shim; a
  failed step is taken back, so the journal and the file never disagree. Undo and redo are `pdf_undo` / `pdf_redo`.
- `ecclesia-core::oplog`: one JSON line per operation, undo, redo and save, written in one call and synced before
  the edit counts. The log starts with the file's fingerprint; a half-written last line is cut off on reading.
  Logs live in the app folder's `oplog/`, named by the file's path key.
- Deviation from step 2: records carry no author yet (one local user); it comes with sync (Y.1).
- `ecclesia-app::editing`: applies operations to the shown document, undo and redo, and reports failures.
- `ecclesia-core::annot` (step 1): `Annotation { id, page, shape, style, author, created, modified, contents, tags,
  anchor }`. The shape is an enum (text markup with quads; a boxed kind with a rectangle; ink strokes; a line;
  a polygon or polyline), so a kind can't carry the wrong geometry. Geometry is in page points, y down, as MuPDF
  gives it. The id is the `/NM` (a UUID from K.28); tags and the K.28 anchor travel in a private `/EcclesiaPDF`
  text string (JSON), which other readers ignore. `page_annotations` reads a page's annotations; kinds the model
  doesn't know are skipped and left untouched in the file.
- Changes `annot.create`, `annot.update` (every property written again; the kind stays) and `annot.delete`, found by
  page and `/NM`. The shim (`ex_annot_create`, `ex_annot_update`, `ex_annot_delete`) writes them in one place; it
  sets the border width before the rectangle, because MuPDF's width setter moves `/Rect`.
- MuPDF draws some kinds at its own size, and the model test reads them back that way: a stamp's artwork is fitted
  and centred in its rectangle, a caret is 20 by 14 points about its centre, a note icon is a square of side 10 to
  20 at its top left.

## Acceptance tests

- [x] `changes_read_back_as_they_were_written`, `an_unknown_change_is_not_read`.
- [x] `nothing_is_written_until_the_first_record`, `a_log_from_an_earlier_run_is_kept_for_recovery`,
  `saving_marks_the_history_saved_until_it_moves`.
- [x] `a_log_cut_anywhere_keeps_every_whole_record` (proptest).
- [x] `outline_edits_are_undone_and_redone_one_operation_at_a_time`, `a_failed_operation_changes_nothing`
  (`ecclesia-mupdf`); `edits_are_logged_undone_and_saved_over_the_file`,
  `a_failed_edit_changes_nothing_and_is_not_logged`.
- [x] Model-based tests pass for 10,000 random sequences: `random_annotation_edits_survive_save_and_reopen`
  (create, update and delete annotations on two pages, save, reopen, compare with the model and with the other
  annotations; then undo everything and compare with the start). Run with `PROPTEST_CASES=10000` (199 s, debug);
  the default run is 48 cases.
- [x] `annotation_changes_read_back_from_the_log_as_written`, `the_anchor_and_tags_travel_in_the_private_key`.
- [ ] Undo after reopening, where `pdf_save_journal` / `pdf_load_journal` allow it.
