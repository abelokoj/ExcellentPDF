---
name: ExcellentPDF
description: A fast, beautiful PDF reader, annotator and notebook; colour in the chrome, the document neutral.
colors:
  background: "oklch(0.985 0 0)"
  foreground: "oklch(0.21 0 0)"
  popover: "oklch(0.998 0 0)"
  primary: "oklch(0.24 0 0)"
  primary-foreground: "oklch(0.985 0 0)"
  muted-foreground: "oklch(0.48 0 0)"
  accent: "oklch(0.93 0 0)"
  accent-foreground: "oklch(0.24 0 0)"
  border: "oklch(0.90 0 0)"
  sidebar: "oklch(0.962 0 0)"
  page-area: "oklch(0.885 0 0)"
  destructive: "oklch(0.53 0.20 27)"
  success: "oklch(0.50 0.13 150)"
  warning: "oklch(0.50 0.14 70)"
  mode-read: "oklch(0.53 0.16 255)"
  mode-annotate: "oklch(0.53 0.16 70)"
  mode-write: "oklch(0.53 0.16 295)"
  mode-study: "oklch(0.53 0.16 150)"
  mode-present: "oklch(0.53 0.16 30)"
  ink-blue: "#1a3d8f"
typography:
  body:
    fontFamily: "system-ui"
    fontSize: "14px"
  list-row:
    fontFamily: "system-ui"
    fontSize: "14px"
  search:
    fontFamily: "system-ui"
    fontSize: "16px"
  group-label:
    fontFamily: "system-ui"
    fontSize: "12px"
rounded:
  row: "6px"
  card: "12px"
spacing:
  row-x: "12px"
  row-y: "8px"
  card-top: "72px"
components:
  list-card:
    backgroundColor: "{colors.popover}"
    rounded: "{rounded.card}"
    width: "560px"
  list-row-focus:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-foreground}"
    rounded: "{rounded.row}"
    padding: "8px 12px"
  page-area:
    backgroundColor: "{colors.page-area}"
---

## Overview

An Operate surface: people read, mark up and write for hours. The chrome is calm and tinted by the chosen theme
family; the document page stays white paper. One look on every OS. The tokens above are the default family, Neutral,
in light; every family and variant comes from one OKLCH palette (`crates/excellent-app/src/theme/palette.rs`), which
is the normative source. The values here are a snapshot of it.

## Colors

- Each variant (light, dark, high-contrast light and dark) is one lightness ladder. A family sets `hue`, `chroma` and
  `tint` (`app/themes/*.toml`): surfaces take a faint tint, the primary and focus ring the full chroma.
- Fixed hues in every family: danger 27, success 150, warning 70, and the mode accents (read 255, annotate 70, write
  295, study 150, present 30), so a mode reads the same in any theme.
- Contrast is tested, not hoped for: 4.5:1 text and 3:1 controls; 7:1 and 4.5:1 in high contrast.
- Annotation and ink colours are content and never follow the theme.

## Typography

The system UI font until the plan names the bundled one. Sizes: 16 search fields, 14 rows and body, 12 group labels
and shortcut hints.

## Layout

Docked sidebar and inspector around the page canvas. Searchable lists (command palette, pickers) sit 72 px from the
window top, centred, 560 px wide, at most 384 px of rows.

## Elevation & Depth

One level: popover cards over the window, with a hairline border and a large soft shadow. No scrim behind list
dialogs, so a live theme preview shows around them.

## Shapes

Cards 12 px, rows 6 px, the theme glyph 5 px, dots round.

## Components

- **List dialog** (`list_dialog`): the command palette and the pickers. Search bar 48 px high with a bottom border;
  group labels muted; the focused row in the accent tint; the checked row medium weight with a primary dot.
- **Theme glyph**: 32 by 22, the family's sidebar strip, background and accent dot.

## Do's and Don'ts

- Do draw every colour from a token; add a token to the palette rather than a literal.
- Do preview a choice live where it can be undone with Escape.
- Don't tint the document page or the annotation colours.
- Don't add a colour pair without adding it to `every_theme_passes_contrast`.
