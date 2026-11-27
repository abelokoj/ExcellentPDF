# Ph4: 27/44 | S.1: Flashcards from highlights, notes and lasso selections

**Tag:** S.1; **Priority:** P0; **Phase:** 4; **Needs:** A.1, H.6, S.4; **Code:** `ecclesia-store::study`, `ecclesia-features::study::cards`
**Plan:** §27 entry `#### Ph4: 27/44 | S.1` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Highlights, margin notes and handwritten figure excerpts stay locked inside individual PDFs, forcing students to re-type material manually into separate flashcard apps and losing the link back to the original page context.

## User story

As a student, I want to turn any highlight, note or lasso selection into a flashcard that remembers its source page and K.28 text anchor so that I can review it with spaced repetition and jump straight back to the source passage.

## UI sketch

- Triggered from the selection/annotation context menu ("Make card"), Study mode toolbar, keyboard shortcut and command palette (`study.make_card`).
- Pre-fills front and back from the highlight text + note, sticky/free-text note, or lasso crop image.
- On the review screen, a "Go to source" button opens the document at the card's anchored page and region.
- Empty state shows a hint to select a highlight, note or lasso region; missing-file state prompts to locate the moved PDF. Every control carries an accessible name and keyboard focus.

## Mode

Study mode (toolbar button visible by default) and Annotate/Write context menus.

## Speed and size impact

- Startup: zero startup cost; `study_card` tables are initialized lazily on first Study use.
- Budgets touched (plan §13.1): none at cold start or scroll; card creation is a single SQLite transaction.
- Installer size: pure Rust model, no extra dependencies.

## Design

- **Model** (`ecclesia-features::study::cards`): `Flashcard`, `CardKind::Basic`, `SourceAnchor` (storing `path`, `page`, K.28 `Quote`, `PageRect`, `annot_id`), `JumpTarget`, `CardMedia`. Constructors: `Flashcard::from_highlight`, `Flashcard::from_note`, `Flashcard::from_lasso`, plus `jump_to_source(document_text)` which re-anchors via `ecclesia_core::anchor::locate` and portable `file://...#page=N` URI round-tripping.
- **Storage** (`ecclesia-store::study`): `study_card` and `study_srs` tables in `library.db` via `upsert_card` / `get_card` / `list_cards` / `delete_card`.
- **Integration hooks** (for `ecclesia-app` wiring): register `study.make_card` in K.16 command table, connect `Flashcard::from_highlight`/`from_note`/`from_lasso` to active selection/annotation, and route `JumpTarget` to the tab/page navigator.

## Acceptance tests

- [x] `study::cards::tests::highlight_card_opens_source_page_and_reanchors`: a card made from a highlight opens its source page from the review screen and re-anchors on re-typeset text.
- [x] `study::cards::tests::note_and_lasso_cards_link_back_to_source_page`: cards made from notes and lasso image crops keep their source page link and round-trip through `CardRecord`.
