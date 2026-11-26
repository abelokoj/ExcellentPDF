# Ph7: 24/32 | O.13: Flatten forms

**Tag:** O.13; **Priority:** P1; **Phase:** 7; **Needs:** O.11; **Code:** `excellent-core::forms::flatten`, `excellent-features::forms::session`
**Plan:** §27 entry `#### Ph7: 24/32 | O.13`.

## Problem

Recipients or archival workflows often require filled forms to become static, uneditable page graphics so values cannot be altered and render identically on every viewer.

## User story

As a user submitting a completed form, I want to save a flattened copy that looks identical on every pixel while removing all interactive form widgets and `/AcroForm` metadata.

## UI sketch

- **Export flattened PDF…** in the Form / File menu and command palette, writing a separate copy while leaving the open editable document untouched.

## Mode

Read and Annotate modes on PDFs with forms.

## Speed and size impact

- Startup: zero startup cost.
- Runtime: runs on a worker via MuPDF's `pdf_bake_document(..., bake_widgets = 1)`.

## Design

- `excellent_core::forms::save_flattened_forms(doc, dest)` delegates to `excellent_mupdf::Document::save_flattened(dest, Forms::Flatten)`.
- `excellent_core::forms::flatten_pdf_bytes(pdf_bytes, target)` bakes interactive widgets into static page content streams and verifies that zero `/AcroForm` fields remain in the output PDF.
- `FormSession::save_flattened_copy` exposes flattening directly on active form sessions.

## Acceptance tests

- [x] `flattened_forms_keep_visual_pixels_and_remove_acroform_widgets`: renders the filled PDF and flattened PDF via MuPDF, asserting exact pixel equality and zero remaining interactive fields.
