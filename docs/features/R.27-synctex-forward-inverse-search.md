# Ph5: 4/45 | R.27: SyncTeX forward and inverse search

**Tag:** R.27; **Priority:** P1 (parity); **Phase:** 5; **Needs:** R.11; **Code:** `ecclesia-features::synctex`
**Plan:** §27 entry `#### Ph5: 4/45 | R.27: SyncTeX forward and inverse search` and SumatraPDF `src/PdfSync.cpp`.

## Problem

LaTeX authors need instant two-way synchronization between `.tex` source lines in their editor and rendered locations in the PDF, plus safe editor command invocation and automatic reload after recompilation.

## User story

As a LaTeX author, I want forward search from VS Code / Vim / Emacs / TeXstudio to highlight the matching PDF box and double-clicking the PDF to jump back to the exact `.tex` line.

## UI sketch

- CLI / single-instance flag `--forward-search <pdf> <source.tex> <line>` highlights `ForwardMatch::rects`.
- Double-click on page canvas runs inverse search (`EditorPreset::VsCode`, `Vim`, `Emacs`, `TexStudio`, or `Custom`) with `%f` and `%l`.
- Settings: `synctex_editor_command`.

## Mode

All reading/study modes whenever a companion `.synctex` or `.synctex.gz` file exists.

## Speed and size impact

- Startup: zero cost; `.synctex(.gz)` is parsed only on first forward/inverse search or mtime reload.
- Installer size: uses workspace `flate2`; zero C library or external crates.

## Design

- **Model (`ecclesia-features::synctex`):**
  - `SyncTexIndex` parses plain `.synctex` and gzipped `.synctex.gz` records, scales SyncTeX coordinates (`Unit`, `Magnification`, offsets) to PDF points, and tracks file `mtime` via `reload_if_changed`.
  - Rejects shell/CLI argument-injection characters (`"`, `<`, `>`, `|`, `\r`, `\n`, `\t`) in SyncTeX input paths (GHSA-jf4v-rw66-j4w2 defense ported from SumatraPDF `PdfSync.cpp`).

## Acceptance tests

- [x] `synctex::tests::parses_gz_synctex_and_performs_forward_and_inverse_search`: verifies `.synctex.gz` parsing and bidirectional search accuracy.
- [x] `synctex::tests::formats_editor_presets_and_rejects_shell_injection`: verifies VS Code/Vim/Emacs/TeXstudio templates and blocks unsafe paths.
