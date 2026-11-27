# Ph7: 17/32 | O.6: Replace images

**Tag:** O.6; **Priority:** P1; **Phase:** 7; **Needs:** K.13; **Code:** `ecclesia-core::edit_content` (`list_page_images`, `extract_image`, `replace_image`, `apply_image_mask`)
**Plan:** §27 entry `#### Ph7: 17/32 | O.6`.

## Problem

Users need to inspect embedded raster XObjects on a PDF page, extract original images cleanly, swap in replacement imagery at matching placement, or crop/clip an image via soft masks/clipping without breaking the rest of the page.

## User story

As a PDF editor updating logos or photos, I want to list and extract embedded images, replace an image XObject in place, and apply rectangular/oval masks cleanly.

## UI sketch

- Image selection handle showing embedded dimensions, color space, `Extract Image...`, `Replace Image...`, and `Mask / Crop` controls.

## Mode

Edit Content tool in Organize / Annotate mode.

## Speed and size impact

- Operates directly on `/XObject` image streams without rasterizing surrounding text or vectors.

## Design

- **Inspection & extraction (`list_page_images`, `extract_image`)**: Enumerates `/Resources` `/XObject` `/Subtype /Image` entries on a page (`EmbeddedImageInfo`) and extracts RGB/alpha payloads (`ExtractedImage`).
- **Replacement & masking (`replace_image`, `apply_image_mask`)**: Writes new RGB/Flate `/Image` XObjects with optional `/SMask` transparency or clipping masks (`ImageMaskShape::Rect | Ellipse`) and preserves `SignatureGuard` protection.

## Acceptance tests

- [x] `replace_extract_and_mask_embedded_image`: inspects embedded image XObjects, extracts original bytes, replaces with new RGB image data, applies a mask, and verifies round-trip image count and pixels.
