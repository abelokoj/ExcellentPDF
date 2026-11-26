# Ph5: 36/45 | F.10: Citations

**Tag:** F.10; **Priority:** P1; **Phase:** 5; **Needs:** F.7; **Code:** `excellent-features::citations`
**Plan:** §27 entry `#### Ph5: 36/45 | F.10`.

## Problem

Researchers reading papers need to extract cited DOIs/arXiv IDs and export clean BibTeX, RIS, CSL-JSON, or formatted citations without leaving the reader.

## User story

As an academic writer, I want to detect references in a PDF, generate citation keys, and export BibTeX, RIS, CSL-JSON, or APA/IEEE/Chicago citations, so that my bibliography stays accurate.

## UI sketch

- Inspector "Cite" panel with key format selector, BibTeX / RIS / CSL-JSON export, and "Copy formatted citation".

## Mode

All modes.

## Speed and size impact

- Startup: nothing loads at startup; runs only when viewing or exporting citations.
- Budgets touched: zero-allocation-heavy parsing in pure Rust; no UI-thread stalls.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::citations::{CitationEntry, EntryKind, AuthorName, KeyFormat, CitationStyle, parse_bibtex, export_bibtex, parse_ris, export_ris, parse_csl_json, export_csl_json, format_citation, generate_citation_key, assign_unique_keys, detect_references_in_text}`.

## Acceptance tests

- [x] `bibtex_ris_and_csl_json_round_trip_cleanly`: Verifies BibTeX, RIS, CSL-JSON round-trips, key generation, and formatted APA output.
- [x] `detects_multiple_references_in_bibliography_text`: Extracts distinct DOIs and arXiv IDs from reference lists.
