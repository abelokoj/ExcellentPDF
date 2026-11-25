# Ph3: 40/49 | H.3: Highlighter pen

**Tag:** H.3; **Priority:** P0; **Phase:** 3; **Needs:** H.2; **Code:** `excellent-ink::pens` (the preset and
`Blend::over`), `excellent-canvas::page` (the live preview), `excellent-mupdf-sys` (the shim's `multiply`),
`excellent-core::annot` (`Annotation::multiply`), MuPDF patch `0094`, `excellent-app` (`ink.pen_highlighter`)
**Plan:** §8.4 and §27 entry `#### Ph3: 40/49 | H.3`.

## Problem

A marker drawn over text covers it. A reader marking a passage by hand needs ink that tints the paper and leaves the
words under it as dark as they were.

## User story

As a student, I want to run a highlighter over printed lines by hand, the way I would on paper, and still read every
word under it, in the light and the dark reading themes.

## UI sketch

The Write toolbar's seventh pen, after the marker: Highlighter (key 7 on the pages, "Highlighter" in the palette under
Ink). While drawing, the stroke shows as a broad yellow chisel the text shows through.

## Mode

Write (the toolbar); the command works in any mode.

## Speed and size impact

Nothing at startup. The live preview draws one quad per segment through the canvas's highlight call instead of the
system trail; the saved appearance is MuPDF's own ink appearance with one more graphics-state entry.

## Design

- **Preset:** `PenKind::Highlighter` is an even chisel four times the pen size (12 pt at the default 3), lightly
  steadied, `Blend::Multiply`, in `HIGHLIGHTER` yellow (`0xffd84d`). Every pen now carries its colour (`INK` blue for
  the others).
- **Blend:** `Blend::over` composites ink over paper as PDF does: multiply keeps black text black and tints white
  paper to the ink's colour.
- **Saved:** the stroke's event carries `blend: "multiply"`; the annotation model's `multiply` reaches the shim, which
  writes `/BM /Multiply` on the Ink annotation (PDF 2.0). Patch `0094` makes MuPDF's synthesized ink appearance honour
  it (`pdf_write_opacity_blend_mode`), so MuPDF and other PDF 2.0 readers draw it multiplied. Reading it back sets
  `multiply` again. XFDF has no blend mode: imported ink comes in Normal.
- **Live preview:** the system's trail draws opaque, so a highlighter stroke skips it and the canvas draws it in its
  own frames as segment quads, darkest wins, so overlapping segments don't darken twice.

## Acceptance tests

- [x] Text under the highlighter stays fully readable in light and dark mode:
  `text_under_the_highlighter_stays_readable_in_every_theme` (black text multiplied stays black, and keeps WCAG AA
  contrast with the highlighted paper in every reading theme after the theme's recolouring).
- [x] The blend: `multiply_keeps_dark_text_dark_where_normal_covers_it`.
- [x] The golden image: `each_pen_matches_its_golden_image` (now with `highlighter.pgm`).
- [x] The preview geometry: `a_highlighter_preview_covers_each_segment`.

## Open

- Snapping a stroke to the text line under it (K.13), optional in the plan, is not built.
- One colour until H.7's favourites let the reader pick one (DD25).
