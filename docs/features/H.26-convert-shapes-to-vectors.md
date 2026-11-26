# Ph4: 13/44 | H.26: Convert shapes to vectors

**Tag:** H.26; **Priority:** P1; **Phase:** 4; **Needs:** H.10; **Code:** `excellent-features::ink::vectorize`
**Plan:** §8.4 and §27 entry `#### Ph4: 13/44 | H.26`.

## Problem

Freehand shapes sketched with the pen remain ink paths even after H.10 hold-to-snap straightens them, so they cannot be reshaped with standard PDF annotation handles (dragging endpoints, resizing boxes, moving polygon vertices) or styled with arrow endings, dashed borders, and fills.

## User story

As a diagrammer sketching flowcharts with a stylus, I want to lasso my hand-drawn shapes and convert them into standard PDF Line, Square, Circle, and Polygon annotations so I can resize, move vertices, and edit them with A.13 handles.

## UI sketch

Available in the lasso selection toolbar ("Convert to vector shapes"), Write menu, command palette (`ink.convert_shapes`, "Convert shapes to vectors"), and keyboard shortcut `Ctrl+Shift+V` (when a lasso selection is active). Converts recognized strokes in one undoable operation while leaving unrecognized handwriting untouched; reports how many shapes were converted.

## Mode

Write.

## Speed and size impact

- Startup: nothing runs at startup; invokes H.10's recognizer only on the user's selected ink strokes.
- Budgets touched: < 1 ms per shape; zero UI-thread stall.
- Installer size: negligible (~3 KB compiled Rust, reuses `excellent-ink::shapes` and A.8–A.10 annotation models).

## Design

- **Model (`excellent-features::ink::vectorize`):**
  - Runs `excellent_ink::shapes::recognize` on each stroke of each selected ink annotation (skipping locked annotations and locked H.14 layers).
  - Maps recognized shapes to standard PDF annotations:
    - `Line` → `Shape::Line` (2 `Grip::Vertex` handles in A.13)
    - `Arrow` → `Shape::Line` with `[LineEnd::None, LineEnd::OpenArrow]` (2 `Grip::Vertex` handles in A.13)
    - Axis-aligned `Rectangle` → `Shape::Boxed(Boxed::Square, rect)` (4 `Grip::Corner` handles in A.13)
    - `Circle` / `Ellipse` → `Shape::Boxed(Boxed::Circle, rect)` (4 `Grip::Corner` handles in A.13)
    - `Triangle`, `Polygon`, and rotated rectangles → `Shape::Path(Path::Polygon, vertices)` (`n` `Grip::Vertex` handles in A.13)
  - Preserves page, color, opacity, width, author, layer, category, and tags.
  - For multi-stroke ink annotations containing both shapes and handwriting, splits recognized strokes out into new vector annotations while keeping unrecognized strokes in the updated ink annotation; deletes the source ink annotation when all its strokes convert.
  - Packs all deletions/updates/creations into a single undoable K.14 `Operation`.
- **Command id (for app wiring):** `ink.convert_shapes` (`Ctrl+Shift+V` with selection).

## Acceptance tests

- [x] `converted_shapes_are_editable_with_a13_handles`: converts hand-drawn line, arrow, rectangle, circle, triangle, and pentagon ink strokes plus a scribble; verifies each converted shape produces the expected PDF annotation kind, offers A.13 `grips()`, responds to A.13 `dragged()` (vertex drags on lines/polygons, corner resizes on squares/circles), preserves metadata, and leaves the scribble untouched.
- [x] `mixed_annotation_keeps_unrecognized_strokes_and_respects_layer_locks`: verifies partial conversion of mixed ink annotations and locked-layer protection.
