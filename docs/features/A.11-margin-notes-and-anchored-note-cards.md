# Ph3: 24/49 | A.11: Margin notes and anchored note cards

**Tag:** A.11; **Priority:** P0; **Phase:** 3; **Needs:** A.4, K.28; **Code:** `excellent-features::annotate::margin`,
`excellent-canvas::page::canvas::margin`, `excellent-app::annotate::margin`
**Plan:** §27 entry `#### Ph3: 24/49 | A.11`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Notes hidden behind markups are read one at a time. Readers who review want every note in view at once, beside what
it is about, as a word processor shows comments.

## User story

As a reviewer, I want the notes on the pages I'm reading shown as cards in the margin, each level with its markup, so
I can read and answer them without opening each one.

## UI sketch

- `annotate.margin_notes` (`M` on the pages, the palette, the Annotate toolbar) and Settings, Annotation, "Margin
  notes": a 280 px column to the right of the pages.
- Each card: a dot in the annotation's colour, its kind and author ("Highlight; Ada"), and up to four lines of its
  note. Cards follow the pages as they scroll and zoom; crowded ones are pushed apart round their notes.
- Click a card: it opens to a text field with the note, focused, and a Done button; the inspector opens on it too.

## Mode

All modes.

## Speed and size impact

None at startup, and nothing while the column is off (the canvas reports only while it shows). While it shows, the
canvas sends one small event per frame the pages move; the notes are read again only when other pages come into view
or the document is edited. Laying out 200 cards takes well under a millisecond.

## Design

- **Model** (`annotate::margin`, pure): `ToScreen` (a page's points to the app's pixels), `has_card`, `outline`,
  `lines`, and `layout`: cards sorted by anchor, merged into clusters while they overlap, each cluster centred on
  its cards' anchors and kept below the column's top.
- **Canvas**: the `margin` prop; each frame, when the pages in view or their places changed, a `margin` event with
  each page's map. `Host::ui_pixels_per_point` turns device pixels into the app's (1/96 inch on Windows and GTK,
  points on macOS).
- **App**: `annotate::margin`: the column, the cards (read from the pages in view), their laid-out tops, and the
  open card's editor on the A.4 draft.

## Acceptance tests

- [x] `two_hundred_notes_on_one_page_lay_out_without_overlap_within_a_frame`.
- [x] `overlapping_cards_spread_both_ways_round_their_notes`, `a_card_alone_sits_level_with_its_note`.
- [x] `a_card_lines_up_with_the_top_of_its_annotation_under_any_rotation`, `a_note_s_lines_are_counted_by_paragraph_and_capped`.
- [ ] Tried by hand: not yet.

## Open

- The column is beside the pages, not drawn over them (DD11).
- "Expand margins" (N.3, Phase 4) isn't built yet.
- A card's height is estimated from its text's length, so a note with long words can be cut at four lines.
