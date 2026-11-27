# Ph3: 23/49 | A.10: Polygons, polylines and clouds

**Tag:** A.10; **Priority:** P0; **Phase:** 3; **Needs:** A.8; **Code:** `ecclesia-features::annotate::{shapes,
polygons}`, `ecclesia-canvas::page::canvas::draw`, `ecclesia-app::annotate`
**Plan:** §27 entry `#### Ph3: 23/49 | A.10`. The label is the entry's place in its phase (plan §27.0, Tag rules).

## Problem

Reviewers ring an irregular region, trace a path, or put a cloud round a change, and expect other apps to show the
same shape. Regular polygons and stars mark things at a glance.

## User story

As a reader, I want to click out a polygon, a polyline or a cloud corner by corner, and drag out a regular polygon or
a star, so I can mark any region and others see the same shape in Acrobat.

## UI sketch

- `annotate.polygon` (`P`), `annotate.polyline` (`Shift+P`), `annotate.cloud` (`Shift+K`): the inspector says "Click
  to add corners". Each click puts a corner down; the side to the mouse follows it, and a polygon's closing side shows
  thinner. A double-click finishes; so does a click on a polygon's first corner. Shift keeps the side at 15° steps.
- `annotate.regular_polygon` (`Shift+R`) and `annotate.star` (`Shift+X`): dragged like a rectangle, tip up.
- Settings, Annotation: "Regular polygon sides" and "Star points", 3 to 32, 5 by default.
- The inspector: a closed shape gets Border (Plain, Cloudy) and Fill; a polyline gets the line's ends; all get width
  and line style.

## Mode

All modes; the toolbar buttons are in Annotate mode's toolbar.

## Speed and size impact

None at startup. While clicking, the canvas draws one or two polylines a frame.

## Design

- **Model** (pure): `annotate::polygons` (regular polygon and star corners in a box, dedupe, bounds, distance, inside);
  `shapes::Kind` gains `Regular(n)`, `Star(n)`, `Polygon`, `PolyLine`, `Cloud`, with `enough`, `trace`, `set_cloud`
  and hit-testing of paths.
- **Core**: `Annotation::cloud`, through `AnnotData` and the shim, and in the operation log's JSON (older logs read as
  a plain border).
- **Shim**: `/BE` read into `cloudy` and `cloud_intensity`, written only when it changes.
- **Canvas**: clicked kinds keep a trace of corners; the click counter finds the double-click; one `pageDraw` carries
  every corner, so the shape is one step of undo.

## Acceptance tests

- [x] `a_cloud_keeps_its_border_effect_through_save_and_reopen` (`/S/C` in the bytes; read back).
- [x] `random_annotation_edits_survive_save_and_reopen` (clouds on closed shapes).
- [x] `a_kind_round_trips_through_its_name_with_its_count`, `a_dragged_star_or_polygon_is_a_polygon_annotation`,
  `clicked_points_make_a_polygon_a_polyline_or_a_cloud`, `a_click_hits_a_polygon_s_sides_and_a_filled_one_s_inside`.
- [x] `a_regular_polygon_and_a_star_fit_their_box_tip_up`, `a_double_click_s_second_point_is_dropped`,
  `distance_and_inside_follow_the_path`.
- [ ] Clouds render the same in Acrobat: not yet checked by hand.

## Open

- Not tried by hand; not opened in Acrobat.
- Shift snapping works on Windows only until the GTK and macOS hosts read the key.
- A shape can't be moved, and its corners can't be dragged, until A.13.
