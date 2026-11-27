# Ph4: 19/44 | N.6: Typed text blocks

**Tag:** N.6 (plus U.8 canvas IME bridge); **Priority:** P1; **Phase:** 4; **Needs:** N.4, U.8; **Code:** `ecclesia-features::notebook::textblocks`, `ecclesia-features::notebook::ime`
**Plan:** §27 entry `#### Ph4: 19/44 | N.6` and `#### Ph3: 49/49 | U.8` (DD26).

## Problem

Notebook users mix handwriting with structured typed notes (headings, bulleted/numbered lists, checklists, code snippets, and tables), and need full CJK input method composition directly inside the canvas.

## User story

As a bilingual note-taker, I want structured typed text blocks with Chinese/Japanese/Korean IME support that display cleanly as readable text in other PDF readers while keeping full block structure in EcclesiaPDF.

## UI sketch

- Write toolbar: Text Block tool (`T`), block-kind selector (Paragraph, Heading, List, Checkbox, Code, Table).
- Inline caret and preedit underline clauses during IME composition; OS candidate window positioned at `ImeBridge::candidate_window_rect()`.

## Mode

Write and Annotate modes.

## Speed and size impact

- Startup: zero cost; layout runs only on active edited blocks.

## Design

- `ecclesia-features::notebook::textblocks::TextBlock`: models all 6 `BlockKind` variants (`Paragraph`, `Heading`, `List`, `Checkbox`, `Code`, `Table`), UTF-8-safe cursor/selection editing, wrapped `layout_lines()`, caret geometry, XHTML rich contents (`/RC`), standard PDF FreeText annotation conversion (`to_freetext_annotation`), and lossless sidecar JSON serialization (`to_json`/`from_json`).
- `ecclesia-features::notebook::ime::ImeBridge` (U.8): platform-agnostic state machine receiving `ImeEvent::{Start, Preedit, Commit, End}` from Windows IMM32/TSF, macOS `NSTextInputClient`, and GTK `GtkIMContext`, tracking styled `CompositionClause` spans and updating the candidate window caret rectangle.

## Acceptance tests

- [x] `all_six_block_kinds_round_trip_sidecar_and_save_readable_freetext_in_pdf`: verifies sidecar JSON round-trip and XHTML rich contents for all 6 block kinds, writes PDF FreeText annotations into a PDF, and verifies MuPDF reads back the readable text.
- [x] `text_block_composes_cjk_via_ime_bridge_and_updates_caret_rect`: verifies Japanese preedit, candidate window tracking, and commit inside a `TextBlock`.
- [x] `cjk_chinese_japanese_and_korean_composition_round_trip`: verifies Chinese Pinyin, Japanese Bunsetsu clauses, and Korean Hangul composition and UTF-8 boundary clamping in `ImeBridge`.
