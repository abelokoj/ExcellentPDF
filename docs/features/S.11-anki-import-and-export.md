# Ph4: 36/44 | S.11: Anki import and export

**Tag:** S.11; **Priority:** P1; **Phase:** 4; **Needs:** S.1; **Code:** `ecclesia-features::study::anki`
**Plan:** §27 entry `#### Ph4: 36/44 | S.11` and §11.7. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Learners who already have decks in Anki desktop or share study decks with classmates need bidirectional `.apkg` import and export supporting Basic, Cloze and Image Occlusion cards with embedded media and source links.

## User story

As a student using both EcclesiaPDF and Anki, I want to export my PDF-linked flashcards (basic, cloze and image occlusion) to `.apkg` and import existing Anki `.apkg` decks cleanly without losing media or source links.

## UI sketch

- Reachable from Study toolbar / deck menu ("Export to Anki (.apkg)", "Import Anki (.apkg)") and command palette (`study.export_apkg`, `study.import_apkg`).
- Runs on a background worker with progress and clear error reporting for corrupt or oversized archives.

## Mode

Study mode.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched (plan §13.1): none at startup; export/import run off the UI thread.
- Installer size: reuses existing workspace `zip` and `rusqlite` crates; zero new dependencies.

## Design

- **Format** (`ecclesia-features::study::anki`): writes and reads standard Anki `.apkg` ZIP archives containing SQLite `collection.anki2` (or `collection.anki21` on import) with `col`, `notes`, `cards`, `revlog` and `graves` tables, JSON `media` manifest and numbered media files (`"0"`, `"1"`, …).
- **Note types**:
  - Basic (`Front`, `Back`, `SourceLink`)
  - Cloze (`Text`, `Back Extra`, `SourceLink`, `type: 1`) grouping sibling ordinals into one note
  - Image Occlusion (`Occlusion` using Anki 23.10+ `{{cN::image-occlusion:rect:left=...:top=...:width=...:height=...}}` syntax, `Image` `<img src="...">`, `Header`, `Back Extra`, `SourceLink`)
- **Safety**: bounds decompressed entry reads to 64 MiB and cleans up temporary SQLite files via an RAII drop guard.

## Acceptance tests

- [x] `study::anki::tests::apkg_round_trips_basic_cloze_and_image_occlusion_with_media_and_source_links`: exports a mixed deck of Basic, Cloze and Image Occlusion cards with media and `SourceLink` URIs to `.apkg`, verifies standard `collection.anki2` + `media` entries, and imports every card back cleanly.
