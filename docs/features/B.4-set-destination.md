# Ph1: 34/60 | B.4: Set destination to the current view

**Tag:** B.4; **Priority:** P0; **Phase:** 1; **Needs:** B.1; **Code:** as B.1
**Plan:** §27 entry `Ph1: 34/60 | B.4`.

## Problem

An entry that opened the wrong page could not be pointed somewhere else.

## User story

As a reader, I want to point an entry at the place I'm reading, so that it opens where the chapter really starts.

## UI sketch

- **Point outline entry here**: the row's context menu or the palette. The selected entry then opens at the top of
  the selection, or at the top-left of the view, as B.1 adds entries. Its title stays.

## Mode

Every reading mode, PDF files.

## Speed and size impact

None at startup. One operation.

## Design

- The pages report the place (`outlineTarget`); the entry's URI is rewritten with `fz_outline_iterator_update`.

## Acceptance tests

- [x] The repointed entry opens at the new place after save and reopen:
  `a_repointed_entry_opens_its_new_place_after_reopening` (`ecclesia-mupdf`), which also undoes it.
- [ ] The inspector shows the entry's page and zoom (needs the inspector's outline view).
