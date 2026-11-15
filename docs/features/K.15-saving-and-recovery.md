# Ph1: 5/60 | K.15: Saving, autosave, recovery and external changes

**Tag:** K.15; **Priority:** P0; **Phase:** 1 (read-only), 3 (edits); **Needs:** K.14; **Code:**
`excellent-core::{save, document}`, `excellent-mupdf::edit`, `excellent-app::editing`
**Plan:** §27 entry `Ph1: 5/60 | K.15`.

## Problem

An edited document had to be written back over the reader's own file without ever risking it, and edits had to
survive a crash.

## User story

As a reader, I want my edits saved on their own, without a save button to remember, and offered back after a crash,
so that I never lose work.

## UI sketch

- No save button needed: the file is saved 2 s after the last edit, when another tab is shown, and as the app quits.
  **Save** (Ctrl+S) in the palette's File group saves now.
- A save that fails says so ("Couldn't save report.pdf", with the reason); the edits stay in the log.
- A file opened with edits a crash left unsaved asks once: "report.pdf has unsaved changes", **Restore changes** or
  **Discard changes**. Closing the question asks again the next time the file opens.

## Mode

Every mode, for PDF files.

## Speed and size impact

Saves run on a background thread, except at quit. An incremental save writes only the edits; not measured.

## Design

- Step 1: `ex_save_pdf` saves incrementally when MuPDF allows it, else writes the whole file (`Save::Rewritten`, for
  a file MuPDF repaired); then the document is reopened from the new file and its undo history starts there.
- Step 2 (`excellent-core::save::replace`): written to a hidden temp file beside the original, read back, synced,
  then renamed over it. A failure at any point leaves the original as it was. The document is opened with shared
  delete, so the rename works while it is open.
- Step 3: the log is synced on every edit (K.14); the PDF follows as above (`excellent-app::editing`).
- Step 4: saves are incremental, so the file as the log began is still the start of the saved file. Recovery replays
  the whole log over those first bytes (the base prefix) and offers the result; Discard deletes the log.

## Acceptance tests

- [x] `the_temp_file_is_hidden_beside_the_original`; `saves_replace_the_open_file_and_keep_undo`
  (`excellent-mupdf`).
- [x] `unsaved_edits_come_back_after_a_crash`, `discarded_edits_are_gone`, `hibernates_only_without_unsaved_changes`.
- [ ] Killing the process at every write point never loses a committed operation (fault injection).
- [ ] Saved files pass `qpdf --check`.
- [ ] Step 5: watching the file for outside changes, safe reload, "keep both" on a conflict.
- [ ] Step 6: saving when the user signs out or Windows shuts down (`WM_QUERYENDSESSION` in the canvas crate).
