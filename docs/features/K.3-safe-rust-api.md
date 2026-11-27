# Ph0: 3/13 | K.3: Safe Rust API

**Tag:** K.3; **Priority:** P0; **Phase:** 0; **Needs:** K.2; **Code:** `crates/ecclesia-mupdf/`
**Plan:** §27 entry `Ph0: 3/13 | K.3`.

## Problem

Raw C MuPDF pointers and thread-affine `fz_context` handles are unsafe to pass directly across Rust application and feature crates.

## User story

As a feature developer, I want RAII Rust wrappers (`Document`, `Page`, `DisplayList`, `Pixmap`, `StextPage`, `Annot`, `Writer`, `Outline`) with typed geometry and `Result<T, MuError>`, so that memory leaks and cross-thread UB are prevented by the type system.

## UI sketch

None.

## Mode

All modes.

## Speed and size impact

Thread-local cloned `fz_context` avoids passing contexts across threads while keeping `Context` `!Send`.

## Design

- RAII `Drop` guards calling `fz_drop_*` on every owned MuPDF resource.
- Typed `PagePoint`, `DevicePoint`, `PageRect`, `DeviceRect`, `Quad`, and `Matrix`.
- `thread_rules_are_in_the_types` enforces `Document: Send`, `DisplayList: Send + Sync`, `Pixmap: Send + Sync`, and `!Send` on page/text/annot handles.

## Acceptance tests

- [x] `every_corpus_file_renders_page_one_to_png_and_gives_text` (`crates/ecclesia-mupdf/tests/corpus.rs`).
- [x] `thread_rules_are_in_the_types`, `an_aborted_cookie_cancels_the_work`, `opens_from_bytes`, `writes_pages_to_a_new_pdf`.
