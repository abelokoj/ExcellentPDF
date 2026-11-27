# Ph4: 29/44 | S.3: Cloze deletions

**Tag:** S.3; **Priority:** P0; **Phase:** 4; **Needs:** S.1; **Code:** `ecclesia-features::study::cards`, `ecclesia-features::study::anki`
**Plan:** §27 entry `#### Ph4: 29/44 | S.3` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Turning dense factual sentences into question-and-answer pairs is slow when the learner simply wants to blank out key terms inside a highlighted passage.

## User story

As a student, I want to select words inside a highlight and turn them into Anki-compatible `{{c1::...}}` cloze deletions so that each blank becomes a card and round-trips cleanly with Anki.

## UI sketch

- From a highlighted passage or card editor, select words and click "Cloze deletion" (or press shortcut / palette `study.make_cloze`).
- Displays `{{c1::term}}` (or `{{c1::term::hint}}`); review prompt shows `[...]` or `[hint]` while non-active ordinals stay visible.
- Accessible labels announce the blank and hint text for screen readers.

## Mode

Study mode (visible by default in Study toolbar).

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched (plan §13.1): none; pure string parsing in microseconds.
- Installer size: zero extra crates.

## Design

- **Model** (`ecclesia-features::study::cards`): `ClozeDeletion`, `parse_clozes`, `cloze_ordinals`, `render_cloze_question`, `render_cloze_answer`, `wrap_cloze`, `CardKind::Cloze { ordinal }`, `Flashcard::from_cloze_text`.
- **Storage** (`ecclesia-store::study`): stored in `study_card` with `kind = 'cloze'` and `cloze_ordinal`.
- **Anki interoperability** (`ecclesia-features::study::anki`): sibling cards sharing a cloze template are grouped into one Anki Cloze note (`type: 1`) with one `cards` row per `ord`, preserving `{{cN::...}}` verbatim across export and import.

## Acceptance tests

- [x] `study::cards::tests::cloze_syntax_parses_wraps_and_renders_anki_compatible`: `{{c1::...}}` and `{{c2::...::hint}}` wrap, parse, and render question/answer sides accurately.
- [x] `study::anki::tests::apkg_round_trips_basic_cloze_and_image_occlusion_with_media_and_source_links`: cloze cards round-trip through Anki export and import.
