# Ph3: 32/49 | A.19: Filters by color, tag, author and date

**Tag:** A.19; **Priority:** P1; **Phase:** 3; **Needs:** A.14, A.18; **Code:** `ecclesia-features::annotate::list`,
`ecclesia-app::annotate::list`, `ecclesia-store::annotation_filters`, `ecclesia-core::document`
(`hide_annotations`), `ecclesia-mupdf::Page::display_list_without`
**Plan:** §27 entry `#### Ph3: 32/49 | A.19`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A file with hundreds of annotations from several readers needs narrowing: one reviewer's notes, this week's, the
ones tagged "method". And the page itself is cluttered with the rest.

## User story

As a reviewer, I want to show only one author's recent annotations, in the list and on the page, and find the same
filter when I open the file again.

## UI sketch

- The Annotations tab's filters, top to bottom: search; kinds; colours; categories; **Tags** (when any); **Authors**
  (when more than one); **Changed**: Any time, Today, Past week, Past month; **Show only matching on the pages** (a
  switch); **Clear filters** (when any filter is on).
- Chosen filters combine with AND; the choices within one (two tags, say) with OR.

## Mode

All modes.

## Speed and size impact

None at startup. Filtering stays one pass over the rows. Turning "show only matching" on, or changing the filter
while it is on, draws the pages again once.

## Design

- **Model** (`annotate::list`, pure): `Filter` gains `tags`, `authors`, `since: Option<Since>` and `show_only`;
  `apply_at` (testable time), `hidden` (the ids the pages leave out), `to_json`/`from_json`, `tags`/`authors` offered.
- **Store** (`annotation_filters`, migration 7): one row per file; the empty filter is forgotten.
- **Core:** `Document::hide_annotations` keeps the hidden ids and draws every page again; display lists are recorded
  without them (`RenderPool::build_display_list`, thumbnails) by the shim's `ex_new_display_list_without`, which runs
  the page's contents, each annotation not hidden (`pdf_run_annot`), then its widgets.
- **App:** the tab loads the file's filter when it shows another file, keeps each change, and hides the rows the
  filter doesn't show when the switch is on (`Editing::redraw`).

## Acceptance tests

- [x] `filters_combine_with_and_and_choices_with_or`.
- [x] `a_filter_is_kept_per_file`, `a_filter_is_kept_as_json_and_read_back` (saved per document).
- [x] `show_only_matching_hides_the_rest_on_the_page`, `a_hidden_annotation_is_left_out_of_the_page`,
  `hiding_annotations_redraws_once_and_leaves_the_file`.
- [ ] Tried by hand: not yet.

## Open

- Printing and export keep every annotation (DD17).
- The hidden set is worked out by the Annotations tab: a reopened file shows every annotation until the tab is shown.
- Typing a search with the switch on draws the pages again at each keystroke that changes what matches.
