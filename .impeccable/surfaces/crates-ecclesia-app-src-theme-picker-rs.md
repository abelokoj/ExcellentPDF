---
version: 1
slug: "crates-ecclesia-app-src-theme-picker-rs"
primary_target: "crates/ecclesia-app/src/theme/picker.rs"
related_targets: ["crates/ecclesia-app/src/list_dialog.rs"]
---

## Scope

The theme and appearance pickers (K.17) and the command palette that shares their list (`list_dialog`). Mode: Operate.

## Audience and job

Readers, students and annotators who want the app to look like theirs. Job: try themes and keep one, in seconds,
without a settings page. Action: move through the list (keys, pointer, typing), Enter keeps, Escape goes back.
Constraint: brief-pinned by the plan's §10.3 (colour in the chrome, the document stays neutral, WCAG AA) and the
maintainer's "one look everywhere".

## Direction contract

THESIS: the window itself is the preview. The picker is a quiet card; the theme it shows is the whole app around it.
It refuses the category default of a settings page of swatches with an Apply button.

OWN-WORLD: tinted neutral surfaces from one OKLCH ladder per variant, one accent from the family's hue, popover card
with a hairline border and soft shadow, rows rounded 6 px with the accent tint on focus, a 32 by 22 window glyph per
theme (sidebar strip, page, accent dot), a 6 px dot for the choice in effect.

STORY: the reader opens the picker, arrows down, sees the app change at once, keeps going, presses Enter on the one
they like, or Escape and nothing changed.

FIRST VIEWPORT: the 560 px card 72 px below the window top, centred; search field on top; groups Themes then High
contrast; no scrim, so the chrome shows the preview.

FORM: a command-palette list, first on the ordered list (it is the palette the app already has). seed: none
(unavailable)

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and
every shipping raster carrying its provenance

## Unresolved

- The page canvas hides under dialogs (it is a window of its own), so the page area previews only after the picker
  closes.
- Which bundled UI font (plan §10.3); the system UI font for now.
- Density and UI scaling (K.17 step 3).
