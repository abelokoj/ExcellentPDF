# Ph6: 7/13 | I.5: Plain-language rewrite and explanation

**Tag:** I.5; **Priority:** P1; **Phase:** 6; **Needs:** I.3; **Code:** `crates/ecclesia-features/src/ai/actions.rs`
**Plan:** §27 entry `#### Ph6: 7/13 | I.5` and §12.1.

## Problem

Dense academic or legal prose is hard to parse without losing fidelity to the original text.

## User story

As a reader, I want to select a dense passage and view a plain-language rewrite anchored to the exact sentence being simplified.

## UI sketch

- Selection toolbar action: **Explain in Plain Language**; opens the result in the AI chat panel with citation `[1]`.

## Mode

Read, Annotate, and Study modes when AI is enabled.

## Speed and size impact

- Startup: none. Scoped strictly to the selected passage (`RagScope::Selection`).

## Design

- `rewrite_plain_language` wraps only the selected passage and its quads into a temporary selection `Chunk`, instructs the provider to simplify without adding external claims, and verifies the cited quote against the selection.

## Acceptance tests

- [x] `summary_rewrite_definition_and_equation_actions_cite_source_pages`: plain-language rewrite cites the verbatim phrase from the selected passage.
