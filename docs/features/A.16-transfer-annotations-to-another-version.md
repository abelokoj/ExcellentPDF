# Ph3: 29/49 | A.16: Transfer annotations to another version

**Tag:** A.16; **Priority:** P1 (parity); **Phase:** 3; **Needs:** K.28; **Code:**
`ecclesia-features::annotate::transfer`, `ecclesia-core::annot::load_text`, `ecclesia-app::annotate::transfer`
**Plan:** §27 entry `#### Ph3: 29/49 | A.16`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A reader highlights a preprint, then the published version comes out, re-typeset on other pages with corrections.
Their highlights are stuck in the old file.

## User story

As a researcher, I want to bring my notes and highlights from the preprint into the published paper, each on the
text it marked, and to know which didn't make it.

## UI sketch

- Transfer (Annotate toolbar), `annotate.transfer` in the palette, or Shift+V on the page: a file dialog asks for
  the version to take annotations from.
- Once both are read, a dialog: "Add 41 annotations from the other version?", with how many were found where they
  were, found elsewhere or with changed text, kept in place (they mark no text) and not found, and the first five
  not found by their text. Add 41, or Cancel.
- Picking the open file itself says so.

## Mode

All modes; the toolbar button is Annotate's.

## Speed and size impact

None at startup. Both documents' text is read on the render workers at background priority, and the matching runs
on a background thread.

## Design

- **Model** (`annotate::transfer`, pure): `transfer` quotes each text markup (its K.28 anchor, else the text under
  its quads), finds it with `anchor::locate` in the target's flat text, and marks the found text on its page; other
  annotations keep page and place. Each placed one gets a new id and an anchor in the target. `changes`, `count`.
- **Core:** `annot::load_text`, every page's text layout, cancellable.
- **App** (`annotate/transfer.rs`): picks the source, reads both, reviews (DD14), adds the placed ones as one
  operation, to the file reviewed only.

## Acceptance tests

- [x] `a_preprint_s_highlights_land_on_the_published_version_95_percent` (the Done when, on a typeset preprint and
  published version: a foreword added, lines and pages broken differently, typos corrected).
- [x] `highlights_follow_their_text_and_other_annotations_keep_their_place`.
- [x] `text_that_is_gone_orphans_its_highlight_and_text_in_place_is_placed`.
- [ ] Tried by hand, on a real preprint and its published version: not yet.

## Open

- The review is a dialog of counts, not a list to pick from (DD14).
- A notes, shape or drawing keeps its page and place even when the text beside it moved.
- Each quote is searched in the whole text, normalized again each time: thousands of highlights in a long book
  take seconds.
