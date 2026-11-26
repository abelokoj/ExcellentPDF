# Ph8: 1/3 | K.12: Page canvas on Android

**Tag:** K.12; **Priority:** P1; **Phase:** 8; **Needs:** K.8; **Code:** `crates/excellent-canvas-android` (`motion`, `surface`, `access`, `split`, `jni_bridge`), `android/src/main/kotlin/com/excellentpdf/canvas/`
**Plan:** §27 entry `#### Ph8: 1/3 | K.12`, §5.4, §7.3, §15 (`U.3` tablets).

## Problem

Android tablets had no `SurfaceView` page canvas, JNI `MotionEvent` batching/axis bridge, GPU/surface tile upload pipeline, or `AccessibilityNodeProvider` tree for TalkBack.

## User story

As an Android tablet reader with a stylus, I want unbuffered low-latency ink using `MotionEvent` history, pressure, tilt, and orientation, palm rejection by `toolType`, smooth pinch/double-tap zoom, and TalkBack document navigation.

## UI sketch

- `ExcellentCanvasView` (`SurfaceView`) calling `requestUnbufferedDispatch(event)` for stylus/eraser pointers.
- Front-buffered ink overlay drawing predicted points separately from committed strokes.
- `CanvasAccessibilityProvider` exposing document root, page nodes, links, and annotations to TalkBack.

## Mode

Every mode; the canvas is the document view.

## Speed and size impact

- Startup: tiles render on demand into `RGBA8888` or `RGB565` buffers bounded by a 64 MiB `TileCache`.
- Budgets: primitive-argument JNI symbols (`Java_com_excellentpdf_canvas_NativeCanvasBridge_*`) avoid per-event object allocation on the hot stylus path.
- Size: zero new Rust crate dependencies; portable Rust tested on all hosts alongside Kotlin Android views.

## Design

- **`motion`**: `AndroidMotionSample` (`axis_pressure`, `axis_tilt_rad`, `axis_orientation_rad`, `touch_major_px`, `MotionToolType`, `SampleOrigin::{Historical, Current, Predicted}`), `MotionBatcher` chronological history sorting, palm rejection via `toolType` and `Palms`, and `excellent_ink` prediction.
- **`surface`**: `AndroidTileSurfaceBridge` packing MuPDF tiles into `RGBA8888` (`HardwareBuffer` / `NativeWindowRgba` / `OpenGlEsTexture`) or `RGB565`.
- **`access` (`U.3`)**: `AndroidAccessibilityTree` modeling `HOST_VIEW_ID = -1`, pages, links, annotations, text search, and `ACTION_ACCESSIBILITY_FOCUS` / `ACTION_CLICK` / `ACTION_SCROLL_*`.
- **`jni_bridge` + `android/`**: `NativeCanvasBridge.kt`, `ExcellentCanvasView.kt`, `CanvasAccessibilityProvider.kt`, and `SplitTabletContainer.kt`.

## Acceptance tests

- [x] `android_tilt_and_orientation_map_to_ink_tilt`: `AXIS_TILT` and `AXIS_ORIENTATION` map accurately to `excellent_ink::sample::Tilt`.
- [x] `historical_samples_sort_chronologically_and_finger_palms_are_rejected`: historical `MotionEvent` samples ingest in order and finger palms are rejected while stylus is active.
- [x] `packs_rgba8888_and_renders_pdf_tiles_for_surface_view`: MuPDF pages render and pack into `RGBA8888` tile surfaces.
- [x] `accessibility_node_provider_tree_resolves_hierarchy_search_and_click`: `AccessibilityNodeProvider` tree supports focus, text search, and link click actions.
- [x] `jni_primitive_lifecycle_round_trips`: JNI lifecycle (`nativeCreate`, `nativeOnMotionSample`, `nativeDestroy`) passes end-to-end.
- [ ] By hand on reference Android stylus tablet: verify Phase 1 reading checklist and ink latency budget.
