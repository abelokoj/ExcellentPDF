# Ph5: 17/45 | H.21: Symmetry

**Tag:** H.21; **Priority:** P2; **Phase:** 5; **Needs:** H.1; **Code:** `excellent-ink::symmetry`
**Plan:** §27 entry `#### Ph5: 17/45 | H.21: Symmetry`.

## Problem

Drawing symmetric geometry figures, mandalas, or diagrams by hand is tedious without live mirror and radial guides.

## User story

As a student sketching symmetric diagrams, I want vertical, horizontal, cross, angled, or radial symmetry guides that mirror my strokes as independent ink strokes.

## UI sketch

- Write toolbar / Command Palette (`ink.symmetry`) toggles symmetry guides (`Vertical`, `Horizontal`, `Cross`, `Line`, `Radial`) and spoke count (`2`–`32`).
- Canvas draws dashed non-printing guide lines from `guide_segments`.

## Mode

Write.

## Speed and size impact

- Startup: zero cost; pure 2D Euclidean reflections and rotations in `excellent-ink`.
- Storage: `MirroredInk` emits separate independent strokes so each copy can be saved, erased, or edited individually.

## Design

- **Model (`excellent-ink::symmetry`):**
  - `SymmetryAxis`, `RadialMode` (`RotateOnly` / `Kaleidoscope`), `guide_segments`, `mirror_stroke`, and `mirror_samples`.

## Acceptance tests

- [x] `symmetry::tests::vertical_and_horizontal_reflections_are_exact_involutions`: verifies reflection accuracy and involution.
- [x] `symmetry::tests::cross_and_radial_generate_separate_strokes`: verifies 4-way cross, N-fold rotational, and kaleidoscope copy counts.
