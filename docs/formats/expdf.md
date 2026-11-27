# EcclesiaPDF Notebook Format (`.expdf`)

**Version:** 1  
**Tag:** `Ph4: 17/44 | N.4: Notebook format (.expdf) and export to PDF`  
**Implementation:** `crates/ecclesia-core/src/notebook/`

## 1. Overview

An `.expdf` file is a standard ZIP archive (PKZIP, Deflate/Store) that packages a standard PDF document alongside structured notebook metadata and binary assets. Any ZIP tool can inspect an `.expdf` file, and its embedded `document.pdf` opens directly in any PDF viewer.

## 2. Archive Layout

```text
example.expdf (ZIP)
├── document.pdf          # Standard PDF 1.7 document (kept incremental where possible)
├── notebook.json         # UTF-8 JSON metadata (schema version 1)
└── assets/               # Optional binary assets
    ├── audio/
    │   └── <id>.opus     # Ogg/Opus or PCM WAV lecture recordings (L.1)
    └── images/
        └── <id>.png      # Cover images, stickers, and embedded media (N.7, N.8)
```

### Compression Rules

- `document.pdf` and `notebook.json`: Deflate compressed (`CompressionMethod::Deflated`).
- Pre-compressed media inside `assets/` (`.opus`, `.png`, `.jpg`, `.jpeg`): Stored without recompression (`CompressionMethod::Stored`); uncompressed assets such as `.wav` use Deflate.
- Archive paths use forward slashes (`/`) and never contain `..` path traversal segments.

## 3. `document.pdf`

- Standard PDF 1.7 document opened in memory via MuPDF's `fz_open_document_with_stream` (`ecclesia_mupdf::Document::from_bytes`).
- Procedural paper templates (N.1) are **not** baked into `document.pdf` during interactive editing; they are drawn procedurally under the page so blank notebooks stay tiny and paper themes adapt dynamically.
- Edits such as page insertion (N.2), margin expansion/restoration (N.3), section reordering (N.8), and inter-page links (N.9) append **incremental PDF revisions** where possible so the original bytes remain intact.
- Expanded page margins record the pre-expansion boxes in the PDF page dictionary private key `/EcclesiaPdfOrigBoxes [[mx0 my0 mx1 my1] [cx0 cy0 cx1 cy1]]` so removing margins restores the original MediaBox and CropBox exactly.

## 4. `notebook.json` Schema (Version 1)

```json
{
  "version": 1,
  "id": "uuid-v4",
  "title": "Physics 101",
  "default_page_size": { "kind": "a4" },
  "default_orientation": "portrait",
  "default_template": "cornell",
  "cover": {
    "style": "linen",
    "title": "Physics 101",
    "subtitle": "Fall 2026",
    "accent_rgb": [29, 78, 216]
  },
  "pages": [
    {
      "id": "page-uuid",
      "template": "cornell",
      "kind": "fixed",
      "section_id": "sec-uuid",
      "original_boxes": {
        "media_box": [0, 0, 595.28, 841.89],
        "crop_box": [0, 0, 595.28, 841.89]
      },
      "sidecar": {}
    }
  ],
  "sections": [
    {
      "id": "sec-uuid",
      "title": "Mechanics",
      "color": [37, 99, 235],
      "start_page": 0,
      "page_count": 4
    }
  ],
  "links": [
    {
      "id": "link-uuid",
      "source_page": 0,
      "rect": [40, 40, 180, 70],
      "label": "Jump to Optics",
      "destination": { "kind": "page", "page": 4, "point": null, "zoom": null }
    }
  ],
  "audio": [
    {
      "id": "rec-uuid",
      "asset_path": "assets/audio/rec-uuid.opus",
      "format": "ogg-opus",
      "started_at_ms": 1000.0,
      "duration_ms": 600000.0,
      "sample_rate": 48000,
      "channels": 1,
      "title": "Lecture 1"
    }
  ]
}
```

### Field Reference

- `version`: Must be `1`. Unknown versions return `NotebookError::InvalidFormat`.
- `default_page_size`: `{"kind": "a4" | "letter" | "a5" | "phone"}` or `{"kind": "custom", "width_pt": <f32>, "height_pt": <f32>}`.
- `default_template` / `pages[].template`: `"blank"`, `"lined"`, `"college-ruled"`, `"grid"`, `"dot"`, `"cornell"`, `"music"`, `"graph"`, `"isometric"`, or `"planner"`.
- `pages[].kind`: `"fixed"` or `"infinite"` (N.5).
- `links[].destination`: `{"kind": "page", "page": <usize>, "point": [x, y] | null, "zoom": <f32> | null}` or `{"kind": "uri", "uri": "ecclesiapdf://..."}`.
- `audio[].format`: `"wav"` or `"ogg-opus"`.

## 5. Atomic Save Semantics (K.15)

`Notebook::save_atomic` writes the complete ZIP archive to a hidden sibling file `.<filename>.ecclesiapdf-saving`, reopens and validates both the ZIP structure and `document.pdf` page count with MuPDF, calls `sync_all()`, and atomically renames over the destination file. Any failure deletes the temporary file and leaves the existing `.expdf` untouched.

## 6. Plain PDF Export

`Notebook::export_pdf` produces a standalone PDF readable by Acrobat, Preview, Chrome, and MuPDF:

1. Bakes every page's procedural paper template (N.1) as a pure-vector PDF content stream underneath the page's own content (`q ... Q` isolated).
2. Writes notebook `sections` (N.8) as standard PDF `/Outlines` bookmarks.
3. Writes `links` (N.9) as standard PDF `/Link` annotations (internal `/Dest` and external `/URI`).
4. Leaves audio assets out by default (`AttachAudio::Omit`), or attaches them as PDF `/EmbeddedFiles` (O.25) when `AttachAudio::Embed` is requested.
