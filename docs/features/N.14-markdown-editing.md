# Ph4: 24/44 | N.14: Markdown editing with math and Mermaid

**Tag:** N.14; **Priority:** P1 (parity); **Phase:** 4; **Needs:** K.5; **Code:** `ecclesia-features::markdown`
**Plan:** §27 entry `#### Ph4: 24/44 | N.14`.

## Problem

Technical users and developers read and edit `.md` READMEs and study notes containing GitHub-Flavored Markdown tables, task lists, LaTeX math, and Mermaid diagrams, and need a reliable fallback when a platform WebView runtime is missing.

## User story

As a developer writing technical notes, I want split Markdown editing and preview with KaTeX math, Mermaid diagrams, and a GitHub-slug table of contents that still renders cleanly via MuPDF if WebView is unavailable.

## UI sketch

- Source editor + live WebView preview served over `ecclesiapdf-md://` (never `file://`), synced heading TOC sidebar, and automatic switch to MuPDF fallback when no WebView runtime exists.

## Mode

Read and Write modes for `.md` / `.markdown` files.

## Speed and size impact

- Startup: zero cost; KaTeX and Mermaid scripts live in an optional pack (K.21) loaded only when a Markdown document contains math or Mermaid blocks.

## Design

- `markdown::toc`: extracts ATX and Setext headings (ignoring fenced code blocks) with GitHub slug rules faithfully ported from SumatraPDF Enhanced's `src/MarkdownToc.cpp`.
- `markdown::render`: pure-Rust GFM HTML renderer with matching heading `id`s, `$...$` / `$$...$$` math blocks, ` ```mermaid ` containers, tables, task lists, strikethrough, autolinks, and tag escaping.
- `markdown::preview`: `PreviewController` serves HTML + KaTeX/Mermaid pack references over `ecclesiapdf-md://`, emits `web_view::evaluate` scroll-sync scripts, and switches to `PreviewBackend::MuPdfFallback` (`render_mupdf_fallback_pdf`) when WebView is unavailable.

## Acceptance tests

- [x] `github_slug_matches_sumatrapdf_enhanced_rules`: verifies exact GitHub slug behavior including `adc_intr_ctl . TRANS_EN` -> `adc_intr_ctl--trans_en`.
- [x] `extract_toc_handles_atx_setext_duplicates_and_fenced_code`: verifies ATX, Setext, duplicate `-1` suffixes, and fenced code exclusion.
- [x] `renders_readme_with_katex_math_mermaid_tables_tasklist_and_github_slugs`: verifies complete README rendering with inline/display math, Mermaid, GFM tables, and task lists.
- [x] `preview_serves_over_app_scheme_and_falls_back_cleanly_to_mupdf`: verifies `ecclesiapdf-md://` asset serving (zero `file://`) and MuPDF PDF fallback when WebView is unavailable.
