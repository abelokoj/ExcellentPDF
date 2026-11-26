# Ph6: 8/13 | I.6: Explain equations step by step

**Tag:** I.6; **Priority:** P1; **Phase:** 6; **Needs:** I.3; **Code:** `crates/excellent-features/src/ai/actions.rs`
**Plan:** §27 entry `#### Ph6: 8/13 | I.6` and §12.1.

## Problem

Mathematical equations in papers often skip algebraic steps or omit variable definitions from earlier paragraphs.

## User story

As a student or researcher, I want to select an equation (from text or H.25 LaTeX) and see a step-by-step KaTeX derivation citing the variables defined on that page.

## UI sketch

- Selection / math preview action: **Explain Equation Step by Step**; displays numbered KaTeX `$$...$$` derivation steps and page citations in the chat panel.

## Mode

Read, Write, Study, and Notebook modes when AI is enabled.

## Speed and size impact

- Startup: none. Uses page-scoped chunks for variable grounding.

## Design

- `explain_equation_steps` takes a LaTeX equation string (either from text or `ai::math::convert_strokes_to_latex`), retrieves page context chunks, requests JSON-schema structured `EquationStep`s, verifies every cited quote via `verify_quote_in_chunk`, and formats KaTeX display blocks.

## Acceptance tests

- [x] `summary_rewrite_definition_and_equation_actions_cite_source_pages`: equations parse into `EquationStep`s with KaTeX `$$` blocks and verified page citations.
