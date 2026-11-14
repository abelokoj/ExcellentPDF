# R.14 Smart dark mode

**Tag:** R.14 · **Priority:** P0 · **Phase:** 1 · **Needs:** K.7 · **Code:** `excellent-core::darkmode`,
`excellent-core::render` (`Recolor`), `excellent-mupdf` (`DisplayList::images`, `PageImage`), the shim's
`ex_display_list_images`, `excellent-canvas-win` (`pageColors`), `excellent-app::reading` (`document_colors`)
**Plan:** §27 entry `#### R.14` and §7.5.

## Problem

A dark theme stops at the page's edge: the pages stay black on white, so reading at night means a bright white
rectangle in a dark window. Plain colour inversion fixes the text but ruins photos, charts and artwork.

## User story

As a reader in a dark theme, I want the pages drawn in the theme's text and paper colours while photos and artwork
keep theirs, so that I can read for hours without glare and still see the pictures as they are.

## UI sketch

- Settings window (Ctrl+,), Reading, first row, "Page colours": a segmented control with Original, Smart and
  Everything. Each segment is a toggle with its name as its accessible name, reachable by Tab and pressed with Space
  or Enter. The hint says what Smart does.
- Command palette: "Page colours: follow the theme" (`view.document_colors`) switches between Original and Smart.
- Keyboard: Shift+I while the pages have the focus does the same. Unlike Enhanced's Shift+I, which inverts for the
  session, this one keeps the choice in `settings.toml`.
- States: off by default. A page whose tiles haven't arrived yet shows the theme's paper colour, not white. A
  picture that can't be decoded is recoloured with the page (Enhanced does the same). Selection and the reading
  ruler's band lie over dark paper translucent, since darkest-wins would not show on it.

## Mode

All modes; off by default.

## Speed and size impact

- Startup: nothing. With the setting off, no tile does any extra work.
- With it on, each tile gets one pass over its pixels after rendering, on the render workers. A page's pictures are
  listed once, when its first tile needs them, and each picture is decoded once, subsampled toward 64 pixels where
  its format allows (as Enhanced does), only when a rule needs it; the statistics read about 32 × 32 of its pixels. This is not measured yet (K.24).
- Installer: code only, no new dependency and nothing for a pack.

## Design

- Model (`excellent-core::darkmode`, pure Rust): `PageColors` (ink, paper, link, `Pictures::Keep` or `Recolor`);
  `recolor` maps a tile's pixels from black-on-white to ink-on-paper, outside a kept region (Enhanced's
  `RecolorPixels`); `SampleStats` judges a small sample of a picture (Enhanced's `PdfDarkModeImageStats.cpp`, every
  threshold kept); `kept_region` picks the one picture region of a page that keeps its colours (Enhanced's
  `BuildPageDarkLegacySkipRects`: tiles of a sliced illustration grouped, decorative strips and small pictures left
  out, light panels and backdrops recoloured, the largest artwork kept). `PageImages` ties it to MuPDF.
- MuPDF: the shim runs a page's display list through a derived device that records each image's box and pixel size;
  `PageImage::sample` decodes one at 64 pixels.
- Rendering: `RenderPool::render_tile` takes an optional `Recolor` and recolours the tile on the worker.
- Canvas: `pageColors` prop; changing the colours renders the tiles again (the colours are not in the tile key).
- App: `document_colors` in `settings.toml`, `"off"` (default), `"smart"` or `"all"`. The colours come from the
  theme: ink is `foreground`, paper is `background`, and links take `primary` when the paper is dark.

This is the path Enhanced ships by default (its legacy bitmap renderer with `PreserveImages`). Its object-level
device (text and vector colours mapped through OKLab while the page is drawn) is opt-in there and not ported; its
unit tests belong to that device, so they wait for it.

## Acceptance tests

From the entry's **Done when** ("the ported tests pass; golden images in dark mode match Enhanced's output on the
corpus"), as far as this step reaches:

- [x] `black_becomes_ink_and_white_becomes_paper`: the pixel mapping matches Enhanced's.
- [x] `link_blue_becomes_the_link_colour_and_red_does_not`: link pixels take the link colour.
- [x] `kept_pixels_and_alpha_are_left_alone`: the kept region and alpha are untouched.
- [x] `the_largest_photo_is_kept_and_small_ones_follow_the_page`: one region per page, the largest.
- [x] `a_sliced_illustration_is_kept_whole`: tiles of one picture are judged together.
- [x] `a_page_sized_scan_and_an_undecodable_picture_follow_the_page`: scans recolour.
- [x] `artwork_on_a_white_backdrop_follows_the_page`, `a_dark_painting_keeps_its_colours_even_filling_the_page`:
  the image rules.
- [x] `reads_page_colours_and_falls_back_to_the_document_own`: the canvas prop.
- [x] `document_colours_round_trip_and_unknown_values_are_the_default`: the setting.
- [ ] Object-level OKLab device and Enhanced's `PdfDarkMode*_ut.cpp` tests ported.
- [ ] Golden images in dark mode against Enhanced's output on the corpus.
