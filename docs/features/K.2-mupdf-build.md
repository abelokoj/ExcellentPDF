# Ph0: 2/13 | K.2: MuPDF build

**Tag:** K.2; **Priority:** P0; **Phase:** 0; **Needs:** K.1; **Code:** `crates/excellent-mupdf-sys/`, `third_party/`
**Plan:** §27 entry `Ph0: 2/13 | K.2`, §6.1, §6.3.

## Problem

MuPDF is written in C with `fz_try`/`fz_catch` `longjmp` error handling that must never unwind across Rust stack frames.

## User story

As a core engine developer, I want MuPDF 1.28.5 and its SumatraPDF patches compiled cleanly via `build.rs` with a safe C shim boundary, so that Rust code never encounters C `longjmp` UB.

## UI sketch

None.

## Mode

All modes.

## Speed and size impact

Static C library built in parallel via `cc`; built-in Base14 and Charis fonts embedded.

## Design

- `third_party/mupdf/` + `third_party/mupdf-patches/` (upstream SumatraPDF 70 patches plus Enhanced `0088`–`0093`).
- `crates/excellent-mupdf-sys/shim.c` & `shim.h` wrapping every MuPDF entry point inside `fz_try`/`fz_catch`.
- Pre-generated `src/bindings.rs` from `bindgen` so builds do not require `libclang`.

## Acceptance tests

- [x] `crates/excellent-mupdf/tests/corpus.rs`: builds on Windows MSVC and opens corpus PDFs cleanly.
