# Ph6: 11/13 | I.10: Cross-document comparison and evidence tables

**Tag:** I.10; **Priority:** P1; **Phase:** 6; **Needs:** I.9; **Code:** `crates/excellent-features/src/ai/extract.rs`
**Plan:** §27 entry `#### Ph6: 11/13 | I.10`.

## Problem

Comparing methodology, sample sizes, results, and limitations across multiple papers manually is error-prone.

## User story

As a researcher, I want to build an evidence table across selected PDFs where every cell links to its source passage and exports cleanly to CSV or Markdown.

## UI sketch

- Workspace / library selection action: **Compare Documents (Evidence Table)**; displays a matrix of `Document x {Method, Sample, Result, Limitation}` with clickable `[p.N]` links and CSV/Markdown export buttons.

## Mode

Read and Annotate modes when AI is enabled.

## Speed and size impact

- Startup: none. Runs `I.9` per selected document on a background worker.

## Design

- `build_evidence_table` executes `extract_structured_items` across chosen `(doc_id, title)` pairs, merges verified items into `EvidenceRow` / `EvidenceCell` structs retaining `Vec<Citation>`, and exports RFC-4180 CSV (`EvidenceTable::to_csv`) and Markdown (`EvidenceTable::to_markdown`).

## Acceptance tests

- [x] `extraction_evidence_table_and_ghost_highlights_enforce_verification_and_consent`: every table cell retains verified source citations and exports valid CSV/Markdown with page links.
