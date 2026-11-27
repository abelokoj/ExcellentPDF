# ADR 0003: MuPDF 1.28.5 as vendored by SumatraPDF

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D3 (plan §3)

## Context

EcclesiaPDF replaces SumatraPDF Enhanced and must render at least as well, across PDF, EPUB, XPS, CBZ and images.

## Decision

Use MuPDF 1.28.5 as vendored by upstream SumatraPDF, with its 70 patches, plus SumatraPDF Enhanced's six annotation patches renumbered `0088`–`0093` (plan §6.1). No PDFium, no pdf.js, no other engine.

## Why

The same rendering, format coverage and fixes as SumatraPDF. MuPDF also gives annotations, redaction, forms, signatures, text, OCR and writers.

## Consequences

`third_party/mupdf` changes only through `third_party/mupdf-patches/`, following SumatraPDF's process. The move to MuPDF 1.29 follows SumatraPDF's update procedure (research note R1.5).
