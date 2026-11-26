# Ph0: 7/13 | K.11: Page canvas on iPad

**Tag:** K.11; **Priority:** P1; **Phase:** 0 spike, Phase 8 ship; **Needs:** K.8; **Code:** `crates/excellent-canvas-ios` (`pencil`, `surface`, `access`, `split`, `ffi`)
**Plan:** §27 entry `#### Ph0: 7/13 | K.11`, §5.4, §7.3, §15 (`U.3` tablets).

## Problem

On iPadOS, ExcellentPDF had no native page canvas, Apple Pencil capture pipeline, Metal/Core Graphics tile surface bridge, C-ABI FFI surface, or `UIAccessibility` rotor model.

## User story

As an iPad reader using Apple Pencil, I want crisp Metal/Core Graphics PDF rendering, low-latency pressure and tilt ink with coalesced and predicted touches, hover previews, barrel squeeze/double-tap tool switching, palm rejection, and VoiceOver rotor navigation.

## UI sketch

- Full-bleed `UIView` backed by `CAMetalLayer` (with Core Graphics fallback) inside the tablet pane.
- Scribble suppressed (`ScribblePolicy::DisabledOnCanvas`) so pencil strokes write ink rather than triggering handwriting-to-text.
- Apple Pencil hover shows a live cursor/tip preview; barrel squeeze / double-tap toggles between Pen and Eraser.

## Mode

Every mode; the canvas is the document view.

## Speed and size impact

- Startup: tiles render lazily on demand through `TileSurfaceBridge` under the 64 MiB `TileCache` budget.
- Budgets: coalesced sub-frame samples feed `excellent-ink` while predicted touches stay in a transient preview buffer so ink-to-pixels targets <= 1 display frame.
- Size: portable Rust + zero-overhead C-ABI FFI (`excellent_ios_canvas_*`) with no extra external dependencies.

## Design

- **`pencil`**: `PencilSample` (`force`, `maximum_possible_force`, `altitude_rad`, `azimuth_rad`, `TouchKind`, `TouchSampleOrigin::{Actual, Coalesced, Predicted}`), `PencilHoverSample`, `PencilGesture::{DoubleTap, Squeeze}`, palm rejection via `excellent_canvas::page::gestures::Palms`, and `ScribblePolicy`.
- **`surface`**: `TileSurfaceBridge` coordinating `Layout`, `Viewport`, `TileCache`, and MuPDF rendering into `BGRA8Unorm` (`MetalTexture`) or `RGB8` (`CoreGraphicsImage`) tile buffers.
- **`access` (`U.3`)**: `IosAccessibilityTree` exposing `DocumentContainer`, `PageElement`, `LinkElement`, and `AnnotationElement` nodes plus `AccessibilityRotorKind::{Pages, Headings, Links, Annotations}` stepping.
- **`ffi`**: `SafeIosCanvas` wrapper over `#[repr(C)]` C-ABI `extern "C"` entrypoints guarded by `catch_unwind` and pointer validation.

## Acceptance tests

- [x] `altitude_and_azimuth_round_trip_through_tilt`: UIKit altitude/azimuth angles map accurately to `excellent_ink::sample::Tilt`.
- [x] `predicted_touches_never_commit_into_finished_stroke`: predicted UIKit touches stay in `predicted_preview` and never leak into committed strokes.
- [x] `squeeze_and_double_tap_toggle_pen_and_eraser`: `PencilGesture::{Squeeze, DoubleTap}` toggle active tool mode cleanly.
- [x] `hover_triggers_palm_rejection_for_direct_fingers`: Apple Pencil hover activates palm rejection for direct finger touches.
- [x] `renders_real_pdf_tiles_for_metal_layer`: MuPDF PDF renders and packs into `BGRA8Unorm` Metal tile surfaces.
- [x] `rotor_steps_through_pages_links_and_annotations`: VoiceOver `UIAccessibility` rotor steps through pages, links, and annotations.
- [x] `safe_wrapper_opens_pdf_routes_pencil_and_steps_a11y_rotor`: C-ABI FFI + safe wrapper round-trip.
- [ ] By hand on iPad hardware: measure Apple Pencil latency spike numbers and verify Phase 1 reading checklist.
