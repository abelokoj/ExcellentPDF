# Ph3: 37/49 | A.25: Flatten on export

**Tag:** A.25; **Priority:** P1; **Phase:** 3; **Needs:** K.15; **Code:** `excellent-app::annotate::flatten`,
`excellent-mupdf::Document::save_flattened` (`Forms`), the shim's `ex_save_flattened`
**Plan:** §27 entry `#### Ph3: 37/49 | A.25`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A reviewed document often goes to people who should see the comments but not change or remove them, or whose
readers don't draw annotations at all. Printing to PDF loses text and links; the annotations need to become part of
the pages.

## User story

As a reviewer, I want to save a copy of the document with its annotations drawn into the pages, so every reader shows
them and nobody can move or delete them, while my own copy keeps them editable.

## UI sketch

- **Flatten** (Annotate toolbar, **Ctrl+Shift+E**, palette "Export flattened copy…"): a save dialog offering
  `<name> (flattened).pdf`. Choosing the open file itself is refused with a message. A dialog says when the copy is
  saved and that this document is unchanged.

## Mode

Annotate and Review.

## Speed and size impact

Nothing at startup. The copy is written on a blocking thread, holding the document's lock while MuPDF writes and
bakes it (about as long as a full save).

## Design

- **Engine:** `ex_save_flattened` writes the document as it is now (edits included) to memory, opens that as a new
  document, bakes its annotations into the page content with `pdf_bake_document` (form fields too when asked), and
  saves it with unused objects dropped. The open document, its edits and their undo history are untouched.
- **App:** `annotate::flatten` asks for the path, refuses the open file, writes to a temporary file beside the target
  and renames it over, then says so. Form fields stay fillable (DD22).

## Acceptance tests

- [x] A flattened copy shows the same pixels and holds no annotations; the open document keeps its own:
  `a_flattened_copy_looks_the_same_and_holds_no_annotations`.
- [ ] Tried by hand: not yet.

## Open

- Form fields always stay fillable; there is no switch to flatten them too (DD22).
- Links are kept, as they are not annotations a reader made.
