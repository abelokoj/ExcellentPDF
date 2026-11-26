# Ph5: 41/45 | E.1: Command-line batch tools

**Tag:** E.1; **Priority:** P1; **Phase:** 5; **Needs:** K.3; **Code:** `crates/excellent-cli` (`args.rs`, `commands.rs`, `error.rs`, `ocr_tools.rs`, `main.rs`)
**Plan:** §27 entry `#### Ph5: 41/45 | E.1`.

## Problem

Power users, scripts, and CI pipelines need headless batch PDF operations (inspect, extract text, search, merge, split, compress, OCR, redact, export, render) without launching the GUI window.

## User story

As an automation user, I want a fast CLI binary over the same MuPDF/core engine with `--json` output and deterministic exit codes so I can script document workflows reliably.

## UI sketch

- Zero-dependency typed CLI parser supporting `--json` before or after subcommands, `--help`, `--version`, and structured exit codes (`0` success, `2` usage, `3` I/O or root-denied, `4` processing failure, `5` verification/auth failure).
- Subcommands: `info`, `text` (`extract-text`), `search`, `merge`, `split`, `compress`, `ocr`, `redact`, `export`, `render`, `run`, `mcp`, `control`.

## Mode

Headless CLI binary (`excellent-cli`).

## Speed and size impact

- Zero external CLI parsing crate (`clap` omitted by design); subcommand dispatch links only `excellent-core`, `excellent-mupdf`, `excellent-features`, `excellent-store`, and `serde_json`.

## Design

- **`args`**: Typed `ParsedCli` / `Command` parser with zero panics or external dependencies.
- **`commands`**: Dispatches batch operations through `build_full_tool_registry()`, MuPDF `Writer`, `excellent-features::ocr`, and `excellent-features::annotate::{xfdf, w3c}`.
- **`redact`**: Rasterizes affected pages with black boxes over matched text/rects, writes a fresh clean PDF, reopens with MuPDF, and verifies zero remaining matches before reporting success.

## Acceptance tests

- [x] `parses_json_flag_before_or_after_subcommand`, `parses_run_tool_with_inline_json_without_swallowing_payload`: typed parser tests.
- [x] `cli_subcommands_run_end_to_end_on_corpus`: integration tests covering `info`, `text`, `search`, `merge`, `split`, `compress`, `ocr`, `redact`, `export`, `render`, and `run` on `base14-outline-annot.pdf`, `zlib-manual.pdf`, and `heavy-vector.pdf`.
- [x] `binary_process_mcp_and_exit_codes_work`: spawns the compiled `excellent-cli` binary and verifies exit codes and JSON output.
