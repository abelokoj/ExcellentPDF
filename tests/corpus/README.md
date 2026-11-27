# Test corpus

Documents the tests open. Keep files small and redistributable; record each one here.

| File | What | Source and licence |
| --- | --- | --- |
| `zlib-manual.pdf` | zlib's manual page: PDF 1.4, 2 US Letter pages, fonts embedded, made with groff and Ghostscript (28 KB) | `zlib.3.pdf` from zlib, as vendored in `third_party/amalgamations/a-zlib/`; zlib licence |
| `base14-outline-annot.pdf` | Hand-written for the tests: 2 A4 pages in Helvetica *not embedded* (exercises the built-in fonts), a two-entry outline, and one Text annotation on page 1 (1.4 KB) | EcclesiaPDF's own, made by a 40-line script; no third-party content |
| `formats.epub`, `.fb2`, `.mobi`, `.html`, `.md` | Hand-written for R.28: one short book of three chapters in each reflowable format, every sentence starting with a "Mark c.nn" so tests can tell where a page starts (3 to 13 KB each) | EcclesiaPDF's own, made by `tests/corpus-src/make_formats.py`; no third-party content |
| `formats.svg`, `.cbz`, `.png`, `.xps` | Hand-written for R.28: an SVG with a shape and a line of text, a two-picture comic, a picture, and an XPS page with a shape (no font, so no text) (under 1.5 KB each) | EcclesiaPDF's own, made by `tests/corpus-src/make_formats.py`; no third-party content |
| `heavy-vector.pdf` | Hand-written for the tests: 1 US Letter page of 1,000 large overlapping grey squares under a line of Helvetica, slow to draw at high zoom (for cancelling a running render) (6.5 KB) | EcclesiaPDF's own, made by a 40-line script with a fixed random seed; no third-party content |
| `rtl-arabic-biditest.pdf` | Hand-written for K.26: 1 A4 page with `/ViewerPreferences << /Direction /R2L >>` and Helvetica text | EcclesiaPDF's own, made by `tests/corpus-src/make_pdf_corpus.py`; no third-party content |
| `acroform-fields.pdf` | Hand-written for K.26: 1 US Letter page with `/AcroForm` `/Tx` widget field and visible Helvetica label | EcclesiaPDF's own, made by `tests/corpus-src/make_pdf_corpus.py`; no third-party content |
| `ten-page-paper.pdf` | Hand-written for K.24/K.26: 10 US Letter pages with `/Outlines`, `/PageLabels`, and Helvetica paragraphs | EcclesiaPDF's own, made by `tests/corpus-src/make_pdf_corpus.py`; no third-party content |
