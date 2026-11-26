# Ph4: 40/44 | G.5: Pen on slides and fading ink

**Tag:** G.5; **Priority:** P0; **Phase:** 4; **Needs:** G.1, H.1; **Code:** `excellent-features::present::ink`
**Plan:** §27 entry `#### Ph4: 40/44 | G.5`.

## Problem

Presenters frequently circle formulas or sketch arrows on slides without wanting to pollute the original slide deck file on disk.

## User story

As a presenter, I want temporary pen ink and auto-fading ink on slides that never modifies the slide file unless I explicitly choose Keep.

## UI sketch

- Present toolbar: Slide Pen, Fading Ink toggle, Clear Ink, Keep Annotations.

## Mode

Present mode.

## Speed and size impact

- Volatile in-memory overlay; zero file I/O unless "Keep" is clicked.

## Design

- `SlideInkLayer` stores `SlideStroke`s with `InkPersistence::{Temporary, Fading { lifetime_ms }}`.
- `tick(now_ms)` dims and prunes expired fading ink.
- `keep_to_changes(author, now_ms)` explicitly converts remaining strokes to standard PDF ink `Change::CreateAnnotation`s (H.9).

## Acceptance tests

- [x] `temporary_ink_never_modifies_file_unless_kept_and_fading_ink_expires`: verifies fading opacity decay, expiry removal, original PDF immutability, and explicit conversion to ink annotations only when kept.
