# PDF Interoperability & Oracle Suite (K.26)

Tracks compatibility of saved and exported PDFs across external readers and command-line oracles:

- **Automated CLI oracles** (`tests/interop/verify_interop.ps1`):
  - `qpdf --check`: verifies cross-reference tables, trailer structure, and stream lengths on corpus & saved PDFs.
  - `pdftotext`: verifies extracted text matches `excellent-text` word expectations.
  - `pdfsig`: verifies PAdES / CMS digital signatures when present.
  - Structural fallback check (always runs even when Poppler/qpdf are not installed): `%PDF-` header, `%%EOF` trailer, `/Catalog`, `/Pages`, and security preflight compatibility.
- **External reader checklist** (`tests/interop/checklist.json`):
  - Adobe Acrobat Reader, PDF-XChange Editor, Apple Preview, Mozilla pdf.js, and SumatraPDF.
