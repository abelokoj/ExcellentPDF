# ADR 0004: Our own MuPDF bindings, with C shims

- **Status:** accepted
- **Date:** 2026-11-09
- **Decision:** D4 (plan §3)

## Context

Existing MuPDF crates build stock MuPDF and call it straight from Rust. That loses SumatraPDF's patches and lets `longjmp` jump over Rust frames.

## Decision

Write our own `excellent-mupdf-sys`, built from SumatraPDF's tree, with a C shim around every MuPDF call (`fz_try` / `fz_catch` inside the shim).

## Why

It keeps SumatraPDF's patches, and MuPDF's `fz_try` uses `setjmp` / `longjmp`, which must not cross Rust frames.

## Consequences

Every MuPDF function we use needs a shim. Owned MuPDF objects are Rust types whose `Drop` calls the matching `fz_drop_*`; contexts are `!Send`.
