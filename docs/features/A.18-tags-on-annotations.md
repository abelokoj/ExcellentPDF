# Ph3: 31/49 | A.18: Tags on annotations

**Tag:** A.18; **Priority:** P1; **Phase:** 3; **Needs:** A.12, K.20; **Code:** `excellent-features::annotate::tags`,
`excellent-store::annotation_tags`, `excellent-app::annotate::tag_index`, `excellent-app::annotate::inspector`
**Plan:** §27 entry `#### Ph3: 31/49 | A.18`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Colours and categories are few and fixed; a reader's own threads ("chapter 3", "reviewer 2", "to cite") need words of
their own, found again later in this file and in others.

## User story

As a researcher, I want to tag annotations with my own words and later find every annotation with a tag.

## UI sketch

- The inspector of a selected annotation has a Tags field under its category: "method, to read". Return or leaving
  the field keeps them; each change is one step of undo.
- The Annotations tab's search ("Search notes, marked text and tags (tag:name)") finds tags as words, and
  `tag:method` finds the annotations tagged "method", whatever its case.

## Mode

All modes.

## Speed and size impact

None at startup. After each save, the file's annotations are read on the render workers at background priority and
their tags written to `library.db` by its writer thread.

## Design

- **Model** (`annotate::tags`, pure): `text`, `set_tags` (B.6's comma rules: trimmed, each once), `asked` (the tag a
  `tag:` word asks for). `annotate::list` matches `tag:` words against a row's tags, other words against its note,
  marked text and tags.
- **Store** (`annotation_tags`, migration 6): `index` replaces a file's rows; `find` returns every file's annotations
  with a tag, any case, by file and page.
- **App:** the inspector's field; `tag_index::index` after every successful save (`editing.rs`).

## Acceptance tests

- [x] `a_tag_finds_tagged_annotations_across_documents` (the index and query F.4's `tag:` will use; DD16).
- [x] `a_tag_word_finds_that_tag_and_plain_words_find_tags_too`.
- [x] `tags_are_typed_with_commas_and_set_in_one_change`, `a_search_word_asks_for_a_tag`.
- [x] `the_anchor_and_tags_travel_in_the_private_key` (saved in the file).
- [ ] `tag:` in the library's search: waits for F.3 and F.4 (Phase 5).
- [ ] Tried by hand: not yet.

## Open

- No library-wide search box before F.4 (DD16).
- The index reads every page's annotations after every save; F.3's indexer reads changed pages only.
