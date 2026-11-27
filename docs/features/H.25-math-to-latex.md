# Ph6: 2/13 | H.25: Math to LaTeX

**Tag:** H.25; **Priority:** P1; **Phase:** 6; **Needs:** H.6, K.21; **Code:** `crates/ecclesia-features/src/ai/math.rs`
**Plan:** §27 entry `#### Ph6: 2/13 | H.25`.

## Problem

Handwritten equations in notebooks and PDFs cannot be copied into LaTeX documents or fed into equation explainers (I.6) directly.

## User story

As a student or researcher, I want to lasso handwritten math strokes, preview the recognized LaTeX formula, confirm it, and copy or insert it as text.

## UI sketch

- Write-mode lasso toolbar: **Math to LaTeX**; opens an inline preview popover showing rendered stroke preview + editable LaTeX + KaTeX rendering with **Copy LaTeX**, **Insert as Text**, and **Dismiss** actions.

## Mode

Write and Notebook modes.

## Speed and size impact

- Startup: none. Built-in geometric recognizer runs purely in-memory on lassoed strokes; optional ONNX `pix2tex` pack loads only on demand.

## Design

- `convert_strokes_to_latex` clusters `HandwritingStroke`s, detects multi-stroke operators (`=`, `+`, `x`), matches 16-point resampled prototypes for digits/letters, parses 2D spatial relations (`\frac{num}{den}`, `\sqrt{...}`, superscripts `^{...}`, subscripts `_{...}`), and returns `MathConversionResult` in `MathConfirmationState::PendingConfirmation`.
- `render_math_strokes_svg` and `render_math_strokes_rgba` provide crisp vector/bitmap previews for confirmation or ONNX inference.

## Acceptance tests

- [x] `math_test_set_achieves_above_eighty_percent_exact_conversion`: >= 80% of the handwritten formula test set converts to exact valid LaTeX with SVG/RGBA previews and confirmation gating.
