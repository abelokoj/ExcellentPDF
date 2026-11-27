# Ph7: 16/32 | O.5: Edit text runs

**Tag:** O.5; **Priority:** P1; **Phase:** 7; **Needs:** K.13, O.7; **Code:** `ecclesia-core::edit_content` (`edit_text_run`, `reflow_text_in_box`, `SignatureGuard`)
**Plan:** §27 entry `#### Ph7: 16/32 | O.5`.

## Problem

Fixing typos or updating dates/amounts directly inside a PDF requires replacing the underlying text operators cleanly, reflowing lines to stay inside the original text box, preserving sibling graphics, and guarding digitally signed documents.

## User story

As a user making small fixes to a PDF without source files, I want to edit a text run, keep lines reflowed inside its bounding box, get warned on missing fonts, and guard against breaking digital signatures.

## UI sketch

- Click-to-edit text box overlay showing line-wrapped preview, font substitution warning banner when fallback metrics apply, and confirmation dialog on signed documents.

## Mode

Edit Text tool in Annotate / Organize mode.

## Speed and size impact

- Rewrites only the edited page's content stream and font resource dictionary (`ensure_font_resource`) while leaving other pages untouched.

## Design

- **`edit_text_run` & `reflow_text_in_box`**: Checks `has_digital_signatures` against `SignatureGuard`, removes/replaces text runs matching target rects, word-wraps replacement text to stay within the target box (`ReflowReport`), and injects fallback Base-14 font resources (`FontHint`) when the exact font subset lacks glyphs (`warnings`).
- **Stream preservation**: Preserves non-text images and vector XObjects (`sample_pdf_with_text_and_image`).

## Acceptance tests

- [x] `edit_text_run_reflows_and_preserves_images`: replaces text on a mixed text+image PDF, asserts updated text extracts cleanly inside the box via MuPDF, verifies embedded images remain intact, and checks signed-PDF guard behavior.
