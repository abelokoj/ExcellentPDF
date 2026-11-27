# Ph7: 29/32 | O.18: Headers, footers and watermarks

**Tag:** O.18; **Priority:** P2; **Phase:** 7; **Needs:** O.1; **Code:** `ecclesia-core::headers`
**Plan:** §27 entry `#### Ph7: 29/32 | O.18`.

## Problem

Preparing client deliverables or controlled drafts requires stamping headers, footers (with page number/total/date/filename tokens), and angled semi-transparent text, image, or PDF watermarks on a removable layer.

## User story

As a document author, I want customizable left/center/right headers and footers plus angled semi-transparent watermarks that can be removed cleanly later.

## UI sketch

- Header/Footer & Watermark modal with `{page}`, `{total}`, `{date}`, `{filename}` token chips, margin boxes, opacity slider, rotation angle dial, behind/over layer toggle, and "Remove overlays" action.

## Mode

Organize -> Headers, Footers & Watermarks.

## Speed and size impact

- Stamps are placed in dedicated `/OCProperties` OCG layers (`/ExtGState` `/ca`/`/CA` alpha), adding negligible size and instant removal without re-authoring underlying page streams.

## Design

- **Token expansion (`expand_template`)**: Substitutes `{page}`, `{total}`, `{date}`, `{filename}`, `{title}`, and Bates tokens.
- **Layer injection (`apply_headers_footers`, `apply_watermark`, `remove_ocg_overlays`, `inject_ocg_overlays`)**: Appends positioned Base-14 text headers/footers and `WatermarkKind::Text | PdfPage | ImageRgb` overlays in front of or behind page contents (`OverlayLayer`) with `/ExtGState` alpha and OCG tracking so `remove_ocg_overlays` strips them cleanly.

## Acceptance tests

- [x] `headers_footers_and_removable_watermark_round_trip`: applies headers, footers, and rotated semi-transparent watermarks, verifies extracted overlay text via MuPDF, and confirms `remove_ocg_overlays` removes them cleanly.
