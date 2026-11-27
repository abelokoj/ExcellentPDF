# Ph5: 39/45 | F.13: Export to Markdown and Obsidian with deep links

**Tag:** F.13; **Priority:** P1; **Phase:** 5; **Needs:** F.12; **Code:** `ecclesia-features::notes::export`
**Plan:** §27 entry `#### Ph5: 39/45 | F.13`.

## Problem

Notes and highlights exported to external tools like Obsidian lose their connection back to the exact PDF page and annotation.

## User story

As an Obsidian user, I want to export my notes and PDF highlights as Markdown files with `ecclesiapdf://open?file=...&page=...&annot=...` deep links, so that clicking any reference jumps straight to the exact spot in EcclesiaPDF.

## UI sketch

- "Export to Obsidian vault…" command (File menu / Command Palette) prompting for a destination vault folder.

## Mode

All modes.

## Speed and size impact

- Startup: zero startup cost.
- Budgets touched: atomic file writes per note/document in a background job.
- Installer size: 0 bytes added.

## Design

- Model: `ecclesia-features::notes::export::{DeepLink, HighlightExport, ObsidianVaultReport, export_note_markdown, export_document_highlights_markdown, export_obsidian_vault}`.
- Deep link format: `ecclesiapdf://open?file=<percent-encoded>&page=<1-based>&annot=<id>`.

## Acceptance tests

- [x] `deep_links_round_trip_with_special_characters`: Round-trips `ecclesiapdf://open?file=...&page=...&annot=...` URIs with spaces and parentheses.
- [x] `exports_obsidian_vault_with_wiki_links_and_deep_links`: Writes `Notes/*.md`, `Highlights/*.md`, and `Index.md` preserving `[[wiki-links]]` and deep links.
