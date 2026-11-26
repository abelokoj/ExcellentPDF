# Ph4: 28/44 | S.2: Image occlusion

**Tag:** S.2; **Priority:** P0; **Phase:** 4; **Needs:** S.1; **Code:** `excellent-features::study::cards`, `excellent-features::study::anki`
**Plan:** §27 entry `#### Ph4: 28/44 | S.2` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Diagrams, anatomical plates, circuit schematics and equation tables cannot be turned into text-only flashcards without losing spatial visual cues.

## User story

As a learner studying visual diagrams, I want to draw mask boxes over a figure and study each box in either single-box ("hide one, guess one") or "hide all, guess one" mode so that I can test visual recall and export the deck to Anki.

## UI sketch

- Invoked from a figure snapshot or Study mode toolbar ("Image occlusion", palette `study.image_occlusion`).
- User draws normalized rectangles over the figure, optionally labels each box, and picks `HideOneGuessOne` or `HideAllGuessOne`.
- Review screen renders the figure with masked boxes and reveals the active box on flip. Every control is keyboard accessible and labelled for screen readers.

## Mode

Study mode (visible by default in Study toolbar).

## Speed and size impact

- Startup: zero startup cost; loads lazily when Study mode or review opens.
- Budgets touched (plan §13.1): none at startup; figure bytes are stored once per note on Anki export.
- Installer size: pure Rust geometry and JSON serialization; zero new crates.

## Design

- **Model** (`excellent-features::study::cards`): `NormalizedRect`, `OcclusionMode` (`HideOneGuessOne`, `HideAllGuessOne`), `OcclusionBox`, `ImageOcclusionSpec`, `CardKind::ImageOcclusion`, `Flashcard::from_image_occlusion`. `masked_indices(active_index)` returns either `[active_index]` or `0..boxes.len()`.
- **Storage** (`excellent-store::study`): serialized in `study_card.occlusion_json` with `kind = 'image_occlusion'`.
- **Anki interoperability** (`excellent-features::study::anki`): exports to Anki 23.10+ native image-occlusion note type (`{{cN::image-occlusion:rect:left=...:top=...:width=...:height=...}}` + `<img src="...">` + `SourceLink`) and imports back losslessly.

## Acceptance tests

- [x] `study::cards::tests::image_occlusion_supports_single_box_and_hide_all_guess_one`: single-box and hide-all-guess-one modes generate per-box cards with exact mask sets and round-trip through SQLite `CardRecord`.
- [x] `study::anki::tests::apkg_round_trips_basic_cloze_and_image_occlusion_with_media_and_source_links`: occlusion cards export to Anki (S.11) as image-occlusion notes with media and re-import cleanly.
