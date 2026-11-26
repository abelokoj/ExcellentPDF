# Ph4: 17/44 | N.4: Notebook format (.expdf) and export to PDF

**Tag:** N.4; **Priority:** P0; **Phase:** 4; **Needs:** K.15; **Code:** `excellent-core::notebook`
**Plan:** §27 entry `#### Ph4: 17/44 | N.4` and `docs/formats/expdf.md`.

## Problem

Rich notebooks need procedural templates, sections, inter-page links, audio recordings, and structured sidecar blocks while remaining based on standard PDF and easy to export for sharing.

## User story

As a user taking lecture notes with audio and templates, I want a reliable `.expdf` notebook file that saves atomically and exports to a plain PDF anyone can open.

## UI sketch

- File > Save / Export as PDF (`notebook.export_pdf`), with an option to omit audio (default) or attach audio as embedded PDF files (O.25).

## Mode

All modes; primary format in Write mode.

## Speed and size impact

- Uses workspace `zip` crate with Deflate for PDF/JSON and Store for pre-compressed media.
- Atomic save verifies ZIP and MuPDF page count before replacing the target file.

## Design

- `excellent-core::notebook::{Notebook, NotebookMetadata, ExportOptions, AttachAudio}` implements `.expdf` ZIP reading, atomic saving, MuPDF in-memory opening (`Document::from_bytes`), asset management under `assets/`, and plain PDF export baking vector templates, `/Outlines` bookmarks, `/Link` annotations, and optional `/EmbeddedFiles` audio attachments.
- Full specification documented in `docs/formats/expdf.md`.

## Acceptance tests

- [x] `expdf_notebook_round_trips_and_exports_plain_pdf`: creates a multi-page `.expdf` with sections, links, and audio asset, atomically saves and reopens it, and verifies plain PDF export both with `AttachAudio::Omit` and `AttachAudio::Embed` in MuPDF.
