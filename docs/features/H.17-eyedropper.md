# Ph4: 10/44 | H.17: Eyedropper

**Tag:** H.17; **Priority:** P1; **Phase:** 4; **Needs:** H.2; **Code:** `excellent-features::ink::eyedropper`
**Plan:** §8.4 and §27 entry `#### Ph4: 10/44 | H.17`.

## Problem

Matching a pen or annotation color to a figure, chart, or highlight already on the page required guessing hex values, and picking from the screen while dark mode is enabled returned inverted colors instead of the document's true colors.

## User story

As a reader annotating diagrams in light or dark mode, I want to click any pixel on the rendered page or an annotation with an eyedropper and pick its exact true color before dark mode so my ink matches the original document.

## UI sketch

Reached from the Write toolbar color picker, the annotation inspector, the command palette (`ink.eyedropper`, "Pick color from page"), and keyboard shortcut `I` (in Write/Annotate modes). Displays a magnified pixel loupe (`loupe_grid`) with hex readout and WCAG-contrasting crosshair; clicking commits the color to the active pen or tool, `Esc` cancels.

## Mode

Write and Annotate.

## Speed and size impact

- Startup: nothing runs at startup.
- Budgets touched: `sample_page_point` renders a 1×1 (or 3×3) pixel patch from the cached `DisplayList` in < 1 ms, never stalling the UI thread.
- Installer size: negligible (~4 KB compiled Rust, zero new dependencies).

## Design

- **Model (`excellent-features::ink::eyedropper`):**
  - `SampledColor` (`r, g, b`, `#rrggbb`, `0xRRGGBB`, normalized `[f32; 3]`, WCAG `luminance`).
  - `PixelBuffer` handles 3-channel RGB, premultiplied RGBA (MuPDF), and straight RGBA with arbitrary row strides.
  - `sample_pixel` and `sample_pixmap` sample exact 1×1 (`SampleArea::SinglePixel`) or averaged 3×3 (`SampleArea::Average3x3`) pixels.
  - `sample_page_point` renders a tiny device patch directly from the page's `DisplayList` on `Background::White` **before** `darkmode::recolor` is applied, guaranteeing exact original colors even when smart dark mode (R.14) is active.
  - `loupe_grid` extracts a square pixel neighborhood for the on-screen magnifier.
  - `EyedropperState` manages tool activation, hover preview, commit, and direct application to `Favorites` (H.7) and `Colors` (A.3).
- **Command id (for app wiring):** `ink.eyedropper` (shortcut `I` in Write/Annotate).
- **Storage:** picked colors update the active favorite pen or tool color (persisted in `settings.toml`).

## Acceptance tests

- [x] `picked_color_is_exact_from_pixmap_before_dark_mode`: renders a real PDF containing exact `#2a7b4f`, confirms smart dark mode alters the displayed tile pixels, and verifies `sample_pixmap` (pre-dark-mode) and `sample_page_point` return exact `#2a7b4f`.
- [x] `buffer_sampling_loupe_and_tool_integration_work`: verifies padded-stride RGB sampling, 3×3 averaging, out-of-bounds safety, loupe grid extraction, and applying picked colors to `Favorites` and `Colors`.
