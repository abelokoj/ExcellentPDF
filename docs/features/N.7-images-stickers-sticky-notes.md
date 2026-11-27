# Ph4: 20/44 | N.7: Images, stickers and sticky notes on pages

**Tag:** N.7; **Priority:** P1; **Phase:** 4; **Needs:** A.20; **Code:** `ecclesia-features::notebook::media`
**Plan:** §27 entry `#### Ph4: 20/44 | N.7`.

## Problem

Notebook users embed screenshots, diagrams, stickers from packs, and quick sticky notes onto pages and need them to move, resize, rotate, crop, and appear identically in other PDF readers.

## User story

As a student annotating lab notebooks, I want to paste or insert images, stickers, and sticky notes, adjust their placement with standard handles, and have them display at the exact same size and place in other PDF readers.

## UI sketch

- Write/Annotate toolbar: Insert Image, Sticker Picker (K.21), Sticky Note.
- Selected media shows A.13 corner resize, rotate, and crop handles.

## Mode

Write and Annotate modes.

## Speed and size impact

- Startup: zero cost; PNG/JPEG header dimensions are read without decoding pixels.

## Design

- `PageMedia`: supports `MediaKind::{Image, Sticker, StickyNote}` with `CropRect`, `rotation_deg`, `move_by`, `resize_corner`, `rotate_by`, `set_crop`.
- `to_pdf_changes` emits undoable `Change::CreateAnnotation` + `Change::StampArt(StampArt::Image)` (or `Boxed::Note` for sticky notes), reusing A.20's PDF stamp appearance pipeline.

## Acceptance tests

- [x] `images_and_stickers_save_as_pdf_stamps_at_exact_size_and_place`: inserts a PNG image, moves/resizes/rotates/crops it, writes PDF stamp changes into a PDF, and verifies MuPDF reads back the stamp annotation at the exact bounding box.
