# Ph1: 1/60 | K.5: Document and View model, document registry

**Tag:** K.5; **Priority:** P0; **Phase:** 1; **Needs:** K.4; **Code:** `ecclesia-core::{document, registry, view}`
**Plan:** §27 entry `Ph1: 1/60 | K.5`, §11.15 and §11.16.

## Problem

Each page canvas parsed its own copy of a file, and its view state lived in the Windows canvas crate, where no other
view or platform could reach it.

## User story

As a reader with a large PDF open in two places, I want it parsed once, so that the second view opens at once and the
app stays small in memory.

## UI sketch

None of its own. It is the model under every page view.

## Mode

Every mode.

## Speed and size impact

- Startup: nothing; the registry is created on the first open.
- A second view of an open file skips the parse and reuses its display lists.

## Design

- `Document`: the shared MuPDF document behind a mutex, each page's display list and revision, the unsaved flag.
  `hibernate` closes the MuPDF document unless there are unsaved changes; `mupdf()` reopens it.
- `FileId`: canonical path, size, modified time, and an FNV-1a hash of the first and last 64 KB.
- `Registry`: `FileId` to `Weak<Document>`; the last view to close frees the document.
- `View`: layout spec, rotation, page box and history (R.8); `ViewCommand` names the commands, `Effect` says what the
  canvas must redo.

## Acceptance tests

- [x] `a_file_opened_twice_is_parsed_once_and_shared`: a file opened twice is parsed once; both views share it.
- [x] `hibernates_only_without_unsaved_changes`: hibernating with and without unsaved changes.
- [x] `a_changed_page_drops_its_list_and_refuses_stale_ones`: a changed page never shows an old list.
- [x] `one_file_has_one_identity_and_new_content_a_new_one`: file identity.
- [x] View serialization: R.9 (`ecclesia-canvas-win::saved`).
- [ ] Open: annotation model and operation log (K.13, K.14); tiles shared between views.
