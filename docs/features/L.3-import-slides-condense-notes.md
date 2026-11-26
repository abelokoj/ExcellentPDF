# Ph4: 26/44 | L.3: Import slides and condense to notes

**Tag:** L.3; **Priority:** P1; **Phase:** 4; **Needs:** N.4; **Code:** `excellent-features::notebook::slides`, `excellent-core::notebook::pdf_ops`
**Plan:** §27 entry `#### Ph4: 26/44 | L.3`.

## Problem

Lecture slide decks leave little room for handwriting, while naive "print slides as images" handouts destroy text selection and searchability.

## User story

As a student importing lecture slides, I want to condense 1, 2, or 3 slides per page onto lined or Cornell note pages as vectors so all slide text stays selectable and searchable.

## UI sketch

- File > Import Slides to Notes (`notebook.import_slides`): choose 1-up, 2-up, or 3-up; choose Lined, Cornell, Grid, or Blank note space; choose paper size and slide numbers.

## Mode

Write and Study modes.

## Speed and size impact

- Pure vector Form XObjects avoid rasterizing slides into large bitmaps and stay crisp at any zoom.

## Design

- `notebook::slides`: `CondenseOptions` (`SlidesPerPage::{One, Two, Three}`, `CondenseNoteStyle::{Lined, Cornell, Grid, Blank}`, `SlideNumbers::{Show, Hide}`), `plan_condensed_pages`, `condense_slides_pdf`, and `condense_slides_to_notebook`.
- `pdf_ops::condense_pages_vector`: wraps each source slide page's `/Resources` and `/Contents` as a vector PDF Form XObject (`/Subtype /Form`) scaled into each target slot alongside vector note ruling lines.

## Acceptance tests

- [x] `condensed_slides_keep_text_selectable_and_searchable_in_1_2_and_3_up`: condenses a 4-slide PDF deck into 3-up Cornell, 2-up Lined, and 1-up Grid layouts, opens every output in MuPDF, and asserts `StextPage` extracts all slide titles as selectable text.
