# Ph6: 1/13 | R.31: Visual search (Google Lens)

**Tag:** R.31; **Priority:** P2 (parity); **Phase:** 6; **Needs:** O.23; **Code:** `crates/excellent-features/src/lens.rs`
**Plan:** §27 entry `#### Ph6: 1/13 | R.31`.

## Problem

Readers encountering an unlabelled diagram, photo, or foreign-script figure inside a PDF want to look it up visually on the web without manually saving and uploading screenshots.

## User story

As a reader, I want to drag a region (O.23), confirm a privacy prompt, and open that cropped PNG in Google Lens when online features are enabled.

## UI sketch

- Snapshot / selection menu: **Search with Google Lens...** (disabled/hidden when online features are off).
- Confirmation dialog: shows region preview, size, and destination URL (`lens.google.com`) with **Cancel** and **Upload & Open** buttons.

## Mode

Read and Annotate modes when online features are enabled.

## Speed and size impact

- Startup: none. Region rendering happens only after explicit confirmation (`LensConsent::Confirmed`).

## Design

- `lens::build_upload_from_region` and `lens::build_upload_from_png` enforce `OnlineAccessMode::Enabled` and `LensConsent::Confirmed` **before** rendering or encoding.
- Validates PNG magic bytes and `MAX_LENS_PNG_BYTES` (32 MiB), then builds both a multipart `encoded_image` HTTP payload and SumatraPDF Enhanced's self-submitting HTML trampoline (`LensUploadPackage::write_trampoline_file`).

## Acceptance tests

- [x] `lens_blocks_when_online_off_or_unconfirmed`: disabled online access, pending consent, and declined consent all fail before any upload is built.
- [x] `lens_builds_valid_multipart_and_trampoline_after_confirmation`: builds valid multipart body and trampoline HTML targeting Google Lens after confirmation.
- [x] `lens_rejects_oversized_or_non_png_payloads`: rejects invalid non-PNG payloads cleanly.
