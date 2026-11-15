# Ph3: 1/49 | K.28: Annotation anchors

**Tag:** K.28; **Priority:** P0; **Phase:** 3; **Needs:** K.13, K.14; **Code:** `excellent-core::anchor`
**Plan:** §27 entry `#### Ph3: 1/49 | K.28` and Appendix D.1.5. The label is the entry's place in its phase
(plan §27.0, Tag rules).

## Problem

An annotation tied only to page coordinates lands on the wrong words, or on nothing, once the document changes: a
re-typeset preprint, a corrected edition, a re-OCR'd scan. Notes, personal bookmarks (B.6), canvas links (X.1),
transfer (A.16) and sync (Y.1) all need a place that outlives the layout.

## User story

As a reader who annotated a preprint, I want my highlights and notes to land on the same words in the published
version, and to be told plainly about the few that can't be placed, so that I never lose my reading work silently.

## UI sketch

No UI in this step. The model hands back one of three placements; the views that use it (A.16, B.6, the margin in
A.17) show an orphaned annotation in their list for the reader to place by hand.

## Mode

All modes; nothing visible.

## Speed and size impact

- Startup: nothing. Anchors are computed when an annotation is made and matched only when a document whose text
  changed is opened with annotations from an earlier version.
- An exact match is a linear scan; a fuzzy match is bit-parallel (one machine word per pattern character, quotes up to
  64 characters), longer quotes are found by their ends. A 1,200-word page set places 109 quotes in well under a
  second in a debug build.
- No new dependency.

## Design

Model, `excellent-core::anchor`, pure Rust:

- `Quote::of(text, range)`: the W3C text-quote selector of a range of the flat text (K.13), 32 characters of prefix
  and suffix and the start as a hint.
- `locate(quote, text) -> Placement`: `Exact(range)`, `Fuzzy { range, errors }` or `Orphaned`.
  - Both sides are normalized first, keeping a map back to the original characters: whitespace runs as one space,
    soft hyphens dropped, a hyphen ending a line joins the word, ligatures spelled out, curly quotes and dashes plain.
    So a re-typeset version usually matches exactly.
  - Several exact matches are ranked by how much of the prefix and suffix agree, then by nearness to the old start.
  - Otherwise a fuzzy match (Myers / Hyyrö) with at most a quarter of the quote's characters wrong; quotes under 8
    characters are never placed fuzzily. The match is widened to whole words.
  - Nothing within that: orphaned. The annotation is kept, never dropped.
- `new_id()`: a version 4 UUID for `/NM`, from std's OS-seeded hasher.

Open (the rest of step 1): writing the anchor into the annotation's private key and the sidecar, filling `/NM` on
every new annotation, and re-anchoring on open. These wait for the annotation model (K.14 step 1).

## Acceptance tests

Done when: annotations survive a re-typeset version of the same paper with at least 95 % placed correctly on a test
set.

- [x] `quotes_survive_a_re_typeset_version_95_percent`: 109 five-word quotes from a 1,200-word text set at 72
  characters a line, placed in the same text re-set at 58 with ligatures and hyphenation and every 50th word
  corrected; at least 95 % land on the right words and occurrence.
- [x] `re_typesetting_keeps_the_quote_exact`: new line breaks, a hyphenated word and a ligature still match exactly.
- [x] `context_chooses_between_repeats_of_the_exact_text`: of three repeats, the one with the old context wins.
- [x] `a_corrected_typo_is_placed_approximately`, `a_long_quote_with_edits_is_placed_by_its_ends`: fuzzy placement.
- [x] `text_that_is_gone_is_orphaned`: removed text and short changed quotes are orphaned, not misplaced.
- [x] `ids_are_version_4_uuids_and_differ`.
- [ ] Anchors stored in `/NM` and the private key, and annotations re-anchored on open (needs K.14 step 1).
