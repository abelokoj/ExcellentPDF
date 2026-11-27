# Ph6: 7/13 | I.4: Summaries

**Tag:** I.4; **Priority:** P1; **Phase:** 6; **Needs:** I.3; **Code:** `crates/ecclesia-features/src/ai/actions.rs`
**Plan:** §27 entry `#### Ph6: 7/13 | I.4` and §12.1.

## Problem

Readers skimming long technical papers or chapters need concise overviews anchored to exact pages.

## User story

As a reader, I want brief, standard, or detailed document and chapter summaries with verified page citations, and the option to insert one as a note only when I explicitly ask.

## UI sketch

- Command palette & document toolbar: `AI: Summarize Document`, `AI: Summarize Chapter`.
- Summary renders in the AI chat panel with `[1]` page citations and an explicit **Insert as Note** button.

## Mode

All modes when AI is enabled.

## Speed and size impact

- Startup: none. Runs asynchronously on demand over indexed chunks.

## Design

- `summarize_document` and `summarize_chapter` retrieve indexed chunks (optionally bounded to a chapter `RangeInclusive<usize>`) and verify every summary bullet through `RagEngine::answer_from_chunks`.
- `build_note_annotation` constructs an `Annotation` (`Boxed::Note`) only when the user explicitly requests insertion.

## Acceptance tests

- [x] `summary_rewrite_definition_and_equation_actions_cite_source_pages`: document summary produces cited Markdown and explicit note builder creates a `Note` annotation only when called.
