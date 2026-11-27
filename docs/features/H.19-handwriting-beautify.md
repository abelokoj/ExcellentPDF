# Ph5: 15/45 | H.19: Handwriting beautify

**Tag:** H.19; **Priority:** P2; **Phase:** 5; **Needs:** H.13; **Code:** `ecclesia-ink::beautify`
**Plan:** §27 entry `#### Ph5: 15/45 | H.19: Handwriting beautify`.

## Problem

Fast stylus note-taking often leaves jittery curves, uneven letter heights, and cramped spacing across a line.

## User story

As a stylus note-taker, I want to beautify selected handwriting with an adjustable strength slider—and undo or revert it cleanly—while keeping my writing legible and recognizable.

## UI sketch

- Lasso selection toolbar / Write menu / Command Palette (`ink.beautify`, "Beautify handwriting") opens a live strength slider (`0%`–`100%`).
- Commits updated strokes as one undoable K.14 operation.

## Mode

Write (with active ink lasso selection).

## Speed and size impact

- Startup: zero cost; runs purely in memory on selected strokes (< 1 ms per paragraph).
- Installer size: zero new crates.

## Design

- **Model (`ecclesia-ink::beautify`):**
  - `beautify_ink` smooths high-frequency pen jitter while keeping stroke endpoints pinned, groups strokes into lines via `straighten`, and gently normalizes letter x-heights and intra-word horizontal gaps according to `BeautifyOptions::strength` and `LetterNormalization`.
  - Returns `BeautifyResult` holding `original` strokes for instant `revert()`.

## Acceptance tests

- [x] `beautify::tests::beautify_smooths_jitter_and_preserves_endpoints`: verifies jitter energy drops substantially while endpoints remain pinned.
- [x] `beautify::tests::beautify_evens_letter_heights_and_is_reversible`: verifies letter height variance decreases and `revert()` restores bit-exact original strokes.
