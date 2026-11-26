# Ph7: 21/32 | O.10: PDF to Markdown and DOCX

**Tag:** O.10; **Priority:** P1; **Phase:** 7; **Needs:** R.16; **Code:** `excellent-core::export_doc` (`extract_structured_blocks`, `pdf_to_markdown`, `pdf_to_docx`)
**Plan:** §27 entry `#### Ph7: 21/32 | O.10`.

## Problem

Copying prose, headings, lists, and tables out of PDFs destroys paragraph wrapping and document hierarchy; users need structured export to clean Markdown and valid OOXML `.docx` files.

## User story

As an author or researcher reusing PDF content, I want to export a PDF into structured Markdown (`#` headings, lists, Markdown tables) or a valid `.docx` document editable in Word or LibreOffice.

## UI sketch

- File -> Export submenu offering `Markdown (.md)` (with optional `<!-- page N -->` markers) and `Word Document (.docx)`.

## Mode

File -> Export.

## Speed and size impact

- Pure Rust block reconstruction from MuPDF `TextLayout` spans plus `zip`-compressed OOXML packaging with zero external converters.

## Design

- **Structured block recovery (`extract_structured_blocks`, `DocBlock`)**: Estimates document body font size (`estimate_global_body_size`) and classifies headings (`H1`–`H3`), paragraphs, ordered/unordered lists, and multi-column tables (`extract_page_blocks`).
- **Markdown & DOCX emitters (`pdf_to_markdown`, `pdf_to_docx`)**: Formats headings, bullets, and tables (`write_md_row`) to Markdown, or writes `[Content_Types].xml`, `_rels/.rels`, and XML-escaped `word/document.xml` (paragraphs, heading styles, tables) into an OOXML ZIP archive via `zip::ZipWriter`.

## Acceptance tests

- [x] `pdf_exports_structured_markdown_and_valid_docx`: exports a structured multi-block PDF to Markdown and `.docx`, verifying headings, lists, XML structure inside `word/document.xml`, and ZIP validity.
