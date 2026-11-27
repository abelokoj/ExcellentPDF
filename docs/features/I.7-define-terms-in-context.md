# Ph6: 7/13 | I.7: Define terms in context

**Tag:** I.7; **Priority:** P1; **Phase:** 6; **Needs:** I.3; **Code:** `crates/ecclesia-features/src/ai/actions.rs`
**Plan:** §27 entry `#### Ph6: 7/13 | I.7` and §12.1.

## Problem

Domain-specific terms carry meanings particular to a paper or textbook that generic dictionaries miss.

## User story

As a reader, I want to select an unfamiliar term and get its definition grounded in the surrounding paragraph and earlier occurrences in the document.

## UI sketch

- Selection toolbar & context menu: **Define in Context**; renders the contextual definition with page `[1]` link in the AI panel.

## Mode

All modes when AI is enabled.

## Speed and size impact

- Startup: none. Queries only the top matching chunks for the selected term plus the immediate paragraph context.

## Design

- `define_term_in_context` combines the immediate `surrounding_passage` on `page` with hybrid document search results for `term`, then runs `RagEngine::answer_from_chunks` so the definition is verified against real quotes.

## Acceptance tests

- [x] `summary_rewrite_definition_and_equation_actions_cite_source_pages`: contextual term definition grounds explanations in verified document quotes.
