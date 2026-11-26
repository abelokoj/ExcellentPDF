# Ph7: 4/32 | A.23: Measurement

**Tag:** A.23; **Priority:** P1; **Phase:** 7; **Needs:** A.8, A.10; **Code:** `excellent-features::measure`
**Plan:** §27 entry `#### Ph7: 4/32 | A.23`.

## Problem

Engineering and architectural reviewers need calibrated distance, perimeter, area, and angle tools on scaled drawings with vector snapping and PDF `/VP`/`/Measure` dictionary round-tripping.

## User story

As a reviewer inspecting scaled plans, I want to calibrate a known length, snap accurately to vector endpoints/midpoints/intersections, and record distance, perimeter, area, and angle annotations.

## UI sketch

- Measurement HUD displays live distance/perimeter/area/angle readout in calibrated units (`pt`, `mm`, `cm`, `m`, `in`, `ft`) plus `/VP` viewport scale controls.
- Vector snap cursor indicator locks onto nearby segment endpoints, midpoints, and line intersections.

## Mode

Annotate mode with Measurement tools active.

## Speed and size impact

- Snap search evaluates page vector segments with bounded tolerance and zero allocations in tight hit loops.

## Design

- **`ViewportScale`**: Calibrates drawing scale (`from_ratio`, `calibrate`) and serializes/parses ISO PDF `/Measure` dictionaries (`to_pdf_measure_dict`, `from_pdf_measure_dict`).
- **`MeasureTool`**: Computes scaled Euclidean distance, polyline perimeter, shoelace polygon area, and vertex angle (`measure_distance`, `measure_perimeter`, `measure_area`, `measure_angle_deg`) and exports standard PDF annotations (`to_annotation`).
- **`snap_to_vector`**: Finds closest vector endpoint, midpoint, or segment-segment intersection (`VectorSegment`, `SnapKind`) within threshold.

## Acceptance tests

- [x] `calibrated_distance_perimeter_area_angle_within_half_percent`: verifies 1:100 calibration and all four measurement formulas within 0.5% accuracy plus `/Measure` dictionary round-trip.
- [x] `vector_snap_finds_endpoints_midpoints_and_intersections`: confirms endpoint, midpoint, and intersection snapping precedence.
