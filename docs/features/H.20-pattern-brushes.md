# Ph5: 16/45 | H.20: Pattern brushes

**Tag:** H.20; **Priority:** P2; **Phase:** 5; **Needs:** H.2; **Code:** `excellent-ink::brushes`
**Plan:** §27 entry `#### Ph5: 16/45 | H.20: Pattern brushes`.

## Problem

Diagrams, borders, and creative annotations often need repeating vector patterns (dots, dashes, crosshatches, zigzags, chains, or stars) rather than plain solid strokes.

## User story

As someone annotating diagrams or creative notebooks, I want pattern brushes whose repeating vector geometry exports into standard PDF appearance streams so every PDF reader shows the exact same pattern.

## UI sketch

- Pen picker offers **Pattern brushes** (`Dots`, `Dashes`, `Crosshatch`, `Zigzag`, `Chain`, `Stars`, or custom `PatternTile`), spacing, scale, and color.

## Mode

Write.

## Speed and size impact

- Startup: zero cost.
- Compatibility: emits ISO 32000-1 `q`/`Q`/`m`/`l`/`c`/`f`/`S` vector operators so Acrobat, Preview, pdf.js, and MuPDF render identically.

## Design

- **Model (`excellent-ink::brushes`):**
  - `PatternBrush` with `PatternKind`, `PatternTile`, `StampOrientation` (`FollowTangent` / `FixedAngle`).
  - `place_stamps` performs uniform arc-length sampling along `InkPoint` polylines.
  - `to_pdf_appearance_stream` converts top-left page points to bottom-left PDF coordinates and emits clean PDF vector operators.

## Acceptance tests

- [x] `brushes::tests::stamps_follow_arc_length_and_tangent`: verifies uniform arc-length spacing and tangent rotation across corners.
- [x] `brushes::tests::emits_valid_pdf_operators_for_all_builtin_patterns`: verifies ISO 32000-1 PDF operator emission for all six built-in patterns.
