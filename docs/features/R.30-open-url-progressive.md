# Ph5: 6/45 | R.30: Open from a URL with progressive loading

**Tag:** R.30; **Priority:** P1; **Phase:** 5; **Needs:** K.3, K.22; **Code:** `ecclesia-core::remote`
**Plan:** §27 entry `#### Ph5: 6/45 | R.30: Open from a URL with progressive loading`.

## Problem

Opening a large PDF from a URL (such as an arXiv paper or textbook) currently requires waiting for the entire file to download before page 1 appears.

## User story

As a researcher opening a PDF URL, I want linearized page 1 bytes fetched first via HTTP byte-range requests so I can start reading immediately and save a local copy once ready.

## UI sketch

- File > Open URL (`file.open_url`, `Ctrl+U`) prompts for `http://` or `https://` URL.
- Progress indicator shows `DownloadProgress::fraction`; File > Save a Copy (`RemoteStreamCache::save_copy`) writes the completed file atomically.

## Mode

All modes.

## Speed and size impact

- Startup: zero cost; `UreqTransport` runs only when opening a remote URL.
- Network: fetches 64 KiB blocks on demand; falls back to full `200 OK` streaming with progress callbacks if the server lacks `Accept-Ranges: bytes`.

## Design

- **Model (`ecclesia-core::remote`):**
  - `RangeTransport` trait (`UreqTransport` using `ureq`, mockable for offline tests).
  - `RemoteStreamCache`: sparse 64 KiB block cache returning `RemoteError::TryLater { start, end }` on uncached reads (mirroring `FZ_ERROR_TRYLATER`), `has_linearized_pdf_marker` for page-1 prioritization, `download_all`, and atomic `save_copy`.

## Acceptance tests

- [x] `remote::tests::progressive_range_reads_detect_linearized_and_trigger_trylater`: verifies `TryLater` signaling, partial block fetching, and linearized PDF detection.
- [x] `remote::tests::non_range_server_downloads_fully_with_progress_and_saves_copy`: verifies `200 OK` fallback with progress reporting and atomic `save_copy`.
