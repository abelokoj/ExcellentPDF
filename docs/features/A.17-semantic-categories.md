# Ph3: 30/49 | A.17: Semantic categories

**Tag:** A.17; **Priority:** P1; **Phase:** 3; **Needs:** A.12; **Code:** `excellent-features::annotate::categories`,
`excellent-core::annot` (`Annotation::category`), `excellent-app::annotate::inspector`, `excellent-app::annotate::list`
**Plan:** §27 entry `#### Ph3: 30/49 | A.17`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

A highlight's colour says little: yellow means something different to each reader, and nothing at all to the tools
that list and export annotations. Researchers mark what a passage does in the argument.

## User story

As a researcher, I want to mark a passage as a claim, its evidence or a question, and later see only my questions.

## UI sketch

- The inspector of a selected annotation starts with "Category": seven chips (Claim, Evidence, Question,
  Definition, Method, Result, To do), each a dot in its colour and its name. The chosen one is raised; pressing it
  again takes the category away.
- The Annotations tab writes a row's category after its kind ("Highlight; Claim") and, when the document has any
  categories, offers them as chips under the colours; chosen chips show only those rows.

## Mode

All modes.

## Speed and size impact

None at startup. The category is a short string in the annotation's private key.

## Design

- **Core:** `Annotation::category: Option<String>` (the key), in the private `/ExcellentPDF` JSON and the log.
- **Model** (`annotate::categories`, pure): `Category` (seven), `key`, `from_key`, `color`, `icon`, `of`,
  `set_category`. A key this version doesn't know reads as none and is kept as it is.
- **List:** `Row::category`, `Filter::categories`, `toggled_category`, `categories(rows)`.
- **App:** `inspector::look` holds the picker (DD15); `category_chip` is shared with the list's filter.

## Acceptance tests

- [x] `a_category_survives_save_and_reopen` (the first half of Done when).
- [x] `the_anchor_and_tags_travel_in_the_private_key`, and the operation log's round trip.
- [x] `a_category_is_set_changed_and_taken_away_one_change_each`.
- [x] `the_filter_shows_the_chosen_categories_only`.
- [ ] Shows in A.26 reports: waits for A.26.
- [ ] Tried by hand: not yet.

## Open

- Picked in the inspector, not a selection toolbar; drawn as colour dots, not icons (DD15).
- Export by category comes with A.26's reports.
