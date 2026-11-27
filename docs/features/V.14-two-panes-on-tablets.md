# Ph8: 2/3 | V.14: Two panes on tablets

**Tag:** V.14; **Priority:** P1; **Phase:** 8; **Needs:** V.1, K.11, K.12; **Code:** `ecclesia-canvas-ios::split`, `ecclesia-canvas-android::split`, `android/.../SplitTabletContainer.kt`
**Plan:** §27 entry `#### Ph8: 2/3 | V.14`, §11.15 (`V.1`, `V.9`).

## Problem

Tablets in landscape can display two documents or two parts of one document side by side, whereas phones must remain single-pane, and stylus/hover input in split mode must stay strictly isolated to the pane under the pen.

## User story

As a tablet reader on iPad or Android in landscape, I want two independent panes side by side with V.9 open-link-in-other-pane, where writing or hovering with my stylus in one pane never shifts or marks the sibling pane.

## UI sketch

- Landscape tablets (`shortest_side_dp >= 600` and `width_dp > height_dp && width_dp >= 840`) automatically enable two side-by-side panes with an adjustable divider.
- Portrait tablets and phones (`shortest_side_dp < 600`) stay single-pane (`SinglePanePortrait` / `SinglePanePhone`).
- Touch-down locks the pointer ID to its pane until touch-up; hover events route to the pane under the pointer.

## Mode

All reading, annotation, and study modes when tablet geometry satisfies the two-pane breakpoint.

## Speed and size impact

- Startup: zero extra cost in single-pane mode; secondary pane state activates only when `SplitEligibility::TwoPane` holds.
- Budgets: pointer routing is constant-time coordinate hit-testing with zero sibling invalidation.

## Design

- **`SplitEligibility`**: `TwoPane`, `SinglePanePortrait`, `SinglePanePhone` computed from `width_dp` / `height_dp` breakpoints (`MIN_TABLET_SHORTEST_SIDE_DP = 600.0`, `MIN_TWO_PANE_WIDTH_DP = 840.0`).
- **`TabletSplitController`**: tracks `PaneViewportState` (`Primary`, `Secondary`), divider ratio (`0.25..=0.75`), pointer capture across strokes, hover routing, and `open_link_in_other_pane` (V.9).
- **`SplitTabletContainer.kt`**: Android `LinearLayout` host reading `Configuration.smallestScreenWidthDp` and dispatching `MotionEvent` touch/hover locally to the target `EcclesiaCanvasView`.

## Acceptance tests

- [x] `breakpoint_distinguishes_landscape_tablet_portrait_tablet_and_phone`: evaluates `TwoPane`, `SinglePanePortrait`, and `SinglePanePhone` across iPad/Android/phone dimensions.
- [x] `stylus_and_hover_route_to_pointer_pane_without_touching_sibling`: iPad stylus/hover routing and V.9 link opening leave sibling pane scroll, page, and stroke state untouched.
- [x] `android_tablet_routes_stylus_to_pane_under_pointer_without_moving_sibling`: Android stylus routing leaves sibling pane untouched.
- [ ] By hand on iPad and Android tablet: verify dual-pane stylus writing and V.9 link opening.
