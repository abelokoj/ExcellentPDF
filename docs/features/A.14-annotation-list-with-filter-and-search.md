# Ph3: 27/49 | A.14: Annotation list with filter and search

**Tag:** A.14; **Priority:** P0; **Phase:** 3; **Needs:** K.14; **Code:** `excellent-features::annotate::list`,
`excellent-core::annot::load_all`, `excellent-app::annotate::list`
**Plan:** §27 entry `#### Ph3: 27/49 | A.14`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A long review leaves hundreds of marks across a document, and the only way to find one was to scroll the pages.

## User story

As a reviewer, I want every annotation in one list, to search it for a word or show only my red highlights, and to
jump to the one I want.

## UI sketch

- An Annotations tab in the right dock, beside the inspector. A search field on top, then a chip per kind the
  document has, then a round chip per colour.
- Under them, a heading per page ("Page 4 (3)") and a row per annotation: a bar in its colour, its note or the text
  it marks (two lines at most), and its kind.
- Pressing a row shows its page, selects the annotation and opens its note in the inspector.
- While the list is read it says so; with no annotations, it says how to make one; with none matching, how to see
  them all.

## Mode

All modes; the toolbar button is Annotate's.

## Speed and size impact

None at startup: the list is read only while its tab is shown, at background priority. Each row keeps its text
folded to lower case, so filtering 10,000 rows is one pass of substring tests; the list draws only the rows on
screen (`virtual_list`).

## Design

- **Model** (`annotate::list`, pure): `Kind`, `Row`, `Filter`, `offered`, `entries`.
- **Core:** `annot::load_all` reads every page's annotations, one lock per page, cancellable.
- **App** (`annotate/list.rs`): the rows and the filter, read again on every edit; the panel; `choose_row`.
- **Command:** `annotate.list` (Shift+O on the page; Annotate toolbar; palette) shows the tab.

## Acceptance tests

- [x] `ten_thousand_annotations_filter_as_you_type_within_a_frame` (the Done when; 16 ms in release builds).
- [x] `rows_read_the_note_else_the_marked_text_and_group_by_page`.
- [x] `the_filter_takes_kinds_a_colour_and_every_word_of_the_search_in_any_case`.
- [ ] Tried by hand: not yet.

## Open

- The tab is in the right dock and rows show a colour bar, not a type icon (DD13).
- After every edit the whole document is read again; a long document with many annotations re-reads in the
  background.
