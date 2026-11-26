# Ph0: 9/13 | K.23: Security limits and fuzzing

**Tag:** K.23; **Priority:** P0; **Phase:** 0 onward; **Needs:** K.3; **Code:** `crates/excellent-core/src/sandbox/limits.rs`, `tests/fuzz/`
**Plan:** §27 entry `Ph0: 9/13 | K.23`, §14, §16.

## Problem

Untrusted PDFs, XFDF files, `.expdf` ZIP archives, and SyncTeX files can embed zip bombs, deeply nested object trees, huge XRef tables, giant images, directory-traversal attachment names, or executable link actions.

## User story

As a reader opening documents from the web or email, I want oversized, deeply nested, or path-escaping inputs rejected safely without panicking or exhausting memory, and external actions to ask before running.

## UI sketch

Blocked or unsafe files report a clear `LimitViolation` error; external links and attachments are classified (`UriSafety`, `sanitize_attachment_name`) so active scripts are blocked and web/file links always require confirmation.

## Mode

All modes.

## Speed and size impact

- Startup: none. Preflight structural checks run in-memory in linear time over input bytes before invoking MuPDF or archive extractors.

## Design

- **`SecurityLimits` (`sandbox::limits`):** central caps for `max_pages`, `max_recursion_depth`, `max_decompressed_bytes`, `max_xref_entries`, `max_compression_ratio`, `max_image_pixels`, `max_object_count`, `max_input_bytes`, `max_ipc_frame_bytes`, `worker_memory_cap_bytes`, and `worker_timeout`.
- **Preflight Validators:** `validate_pdf_bytes`, `validate_zip_archive_bytes`, `validate_xfdf_bytes`, `validate_synctex_bytes`, and `open_pdf_with_limits`.
- **Path & Link Guards:** `sanitize_archive_entry_path`, `sanitize_attachment_name`, and `classify_external_uri`.
- **Fuzz Targets (`tests/fuzz/`):** `open_render`, `parse_limits`, and `annot_save`, seeded from `tests/fuzz/corpus/` and executed in CI (`.github/workflows/fuzz.yml`) plus every `cargo test` run.

## Acceptance tests

- [x] `security_limits_reject_bombs_traversal_and_oversized_inputs`: rejects page/image/XRef/recursion bombs, zlib/ZIP/gzip bombs, XML entity bombs, path traversal, and dangerous URI schemes.
- [x] `malformed_fuzz_corpus_inputs_never_panic`: sweeps all malformed PDF, XFDF, `.expdf`, and SyncTeX regression seeds in `tests/fuzz/corpus/` without panicking.
