# Ph5: 19/45 | H.24: Convert handwriting to text

**Tag:** H.24; **Priority:** P1; **Phase:** 5; **Needs:** H.23, H.6; **Code:** `ecclesia-features::handwriting::convert`
**Plan:** §27 entry `#### Ph5: 19/45 | H.24: Convert handwriting to text`.

## Problem

After jotting rough handwritten notes with a stylus, users often want to convert a lassoed paragraph into clean typed `FreeText` or copy it to the clipboard.

## User story

As a note-taker, I want to lasso handwritten lines, inspect recognition candidates, and convert them into a positioned `FreeText` annotation while preserving line breaks.

## UI sketch

- Lasso selection toolbar > **Convert to text** (`ink.convert_to_text`).
- Candidate popover displays `RecognizedWord::candidates` and offers **Replace with Text Box** or **Copy Text**.

## Mode

Write (with active ink lasso selection).

## Speed and size impact

- Startup: zero cost.
- Undoability: generates a single K.14 `Operation` deleting source ink annotations and creating the `FreeText` annotation.

## Design

- **Model (`ecclesia-features::handwriting::convert`):**
  - `convert_lasso_to_text` runs `Recognizer::recognize`, preserves line breaks via `RecognizedWord::line_index`, computes the tight padded `PageRect`, and emits either `ConversionTarget::CopyTextOnly` or `ConversionTarget::ReplaceWithFreeText` with an atomic `Operation`.

## Acceptance tests

- [x] `handwriting::convert::tests::conversion_keeps_position_and_line_breaks_in_freetext_operation`: verifies multi-line line-break preservation, exact `FreeText` position, and single undoable `Operation` creation.
