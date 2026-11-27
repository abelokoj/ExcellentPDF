# NSH upstream proposal: pen input and native components under NSH content

Task T9 of the plan (§7.3, §22). A draft to file with NSH as an issue or a design note. Written against NSH v0.5.0
(`C:\dev\nsh-src`, commit `f4d68f7f`). Nothing here is filed yet: the maintainer decides when and where.

## Why

EcclesiaPDF is a PDF reader and notebook with pen ink. Its page view is one native component,
`ecclesia.page-canvas`. On Windows it has to be an own-window component (`NativeComponent::window`) today, for two
reasons:

1. **Pen data.** NSH's pointer events say *that* a pen is down (`PointerType::Pen`), but not its pressure, tilt,
   rotation, eraser end or barrel button, and not the samples Windows coalesced between two messages. Ink needs all of
   them. A drawn component (`NativeComponent::draw`) gets no pointer input at all: presses and moves go to its view's
   handlers.
2. **Latency.** Ink must reach the screen within one display frame of the pen touching it. The own window reads
   `WM_POINTER*` itself and presents its own swap chain, with no hop through NSH's event path.

The own window has a cost: **it covers anything NSH draws over its view**. Our command palette, popovers and menus
drawn by NSH disappear behind the page, so the app hides the canvas while the palette is open. Every app that embeds a
map, a video or a canvas this way meets the same problem.

## What NSH v0.5.0 has

- `nsh_core::ops::PointerType` (`Mouse`, `Pen`, `Touch`) on touches (`TouchPoint::pointer_type`) and context menus.
- On Windows, `window.rs` handles `WM_POINTER*` with `GetPointerInfo` only, and passes `pen: bool` to
  `input::pointer`.
- `native/web.rs` already reads `GetPointerPenInfo` (pressure, rotation, tilt, flags, mask) to forward pen input to
  WebView2. So the data is one call away in the message handler that already exists.
- `compose.rs` already layers web views between NSH's own content: a DirectComposition target on the window, each web
  view's visual, and above each a composition swap chain with what NSH paints after it. Web views get their input
  forwarded from NSH's window (`SendPointerInput`).

## Proposal

### A. Pen state on pointer events

Add the DOM's pen fields to the pointer data NSH already delivers, with the DOM's names so TS apps read them as in a
browser (`PointerEvent.pressure`, `tiltX`, `tiltY`, `twist`, eraser as button 5):

```rust
/// A pen's state at one sample. A pen reports only some of these; the rest are `None`.
#[non_exhaustive]
pub struct PenState {
    /// 0 to 1.
    pub pressure: Option<f32>,
    /// Degrees, -90 to 90, toward the right (`tiltX`) and toward the user (`tiltY`).
    pub tilt_x: Option<f32>,
    pub tilt_y: Option<f32>,
    /// Degrees clockwise about the pen's axis, 0 to 359 (`twist`).
    pub twist: Option<f32>,
    /// The eraser end is down, or the pen is turned over (the DOM's button 5).
    pub eraser: bool,
    /// The barrel button is held (the DOM's button 2 for a pen).
    pub barrel: bool,
}
```

- On `TouchPoint` (and wherever pen presses and moves are delivered): `pen: Option<PenState>`, `Some` when
  `pointer_type` is `Pen`.
- **Coalesced samples:** `coalesced: Vec<(Point<Points>, PenState, f64)>` on moves, oldest first, as the DOM's
  `getCoalescedEvents()`. Without them a fast stroke loses its shape at 60 Hz.
- **Timestamps** from the input, not from the time of handling (Windows `PerformanceCount`), so apps can measure and
  predict.

Sources per platform:

| Platform | Pen state | Coalesced samples |
| --- | --- | --- |
| Windows | `GetPointerPenInfo` (`penMask` says which values are valid; pressure 0 to 1024) | `GetPointerPenInfoHistory`, newest first |
| macOS | `NSEvent.pressure`, `tilt`, `rotation`, `pointingDeviceType == .eraser` | none: turn off AppKit's coalescing (`NSEvent.isMouseCoalescingEnabled = false`) so each sample is an event |
| Linux (GTK 4) | `gdk_event_get_axis` with `GDK_AXIS_PRESSURE`, `XTILT`, `YTILT`, `ROTATION`; `GdkDeviceTool` type eraser | `gdk_event_get_history` |
| iOS / iPadOS | `UITouch.force / maximumPossibleForce`, `altitudeAngle`, `azimuthAngle`, `rollAngle` | `coalescedTouches(for:)`, `predictedTouches(for:)` |
| Android | `MotionEvent.AXIS_PRESSURE`, `AXIS_TILT`, `AXIS_ORIENTATION`, `TOOL_TYPE_ERASER` | `getHistorical*` |

EcclesiaPDF has a tested Windows normalization of this data (clamping, the validity mask, history order) that it can
offer as the starting point: `crates/ecclesia-canvas-win/src/pointer.rs`.

### B. Pointer input for drawn components

`NativeComponent` gets wheel events (`fn wheel(&mut self, event: &Wheel) -> bool`) but not presses and moves. Add the
same for the pointer, so a drawn component can be an ink surface:

```rust
/// A press, move or release over the component (or captured by it after a press), with pen state for a pen.
/// True when it took it; otherwise its view's handlers get it, as today.
fn pointer(&mut self, event: &Pointer) -> bool { false }
```

`Pointer` carries the kind (down, move, up, cancel), the pointer id, the position in the component's points, the
buttons, the modifiers, `PenState` and the coalesced samples of part A. Taking a down captures the pointer for the
component until up or cancel.

### C. Native components in the composition tree

Let an own-window component be layered like a web view, so NSH content can be drawn over it:

- A component may return a DirectComposition visual (`fn visual(&self) -> Option<IDCompositionVisual2>`) instead of a
  window. `compose.rs` places it exactly as it places a web view's `Portal`: below the layer with what NSH paints after
  it, clipped and transformed with its view.
- Input for such a component comes from NSH's window. For pen latency, the component must get the raw message on the
  same thread, before any other handling: `fn pointer_message(&mut self, msg: u32, pointer_id: u32) -> bool`
  (Windows), so it can call `GetPointerPenInfoHistory` itself and draw in the same frame. Other platforms have their
  equivalent hook (the `NSEvent`, the `GdkEvent`, the `UITouch` set, the `MotionEvent`).
- A component's own swap chain and its delegated ink trail (`IDCompositionDelegatedInkTrail`, Windows 11 24H2) keep
  working: the trail is created for the component's swap chain and lives in its visual.

With C, EcclesiaPDF drops the palette workaround and draws NSH popovers, menus and the selection toolbar over the
page.

## What we ask, in order

1. A: pen state and coalesced samples. Small, useful to every app with a pen, and the Windows data is already read in
   `native/web.rs`.
2. C: composition-visual components with raw pointer messages. Fixes overlays for every embedded native view.
3. B: pointer input for drawn components. With C in place, EcclesiaPDF does not need it, but other apps drawing ink
   with NSH's Direct2D context do.

EcclesiaPDF can contribute the Windows parts of A and C, with tests, if NSH agrees with the shape.

## What EcclesiaPDF does until then

It keeps its own child window, reads `WM_POINTER*` with `GetPointerPenInfo` and `GetPointerPenInfoHistory`, and
presents through its own DirectComposition target with the ink trail (plan K.8, "Built (T8)"). It hides the canvas while
NSH content must show over it.
