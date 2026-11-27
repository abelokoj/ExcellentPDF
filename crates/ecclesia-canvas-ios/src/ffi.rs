//! C-ABI FFI surface and safe Rust wrapper for the iPadOS `UIView` / `CAMetalLayer` / `UIAccessibility`
//! bridge (plan K.11, U.3, V.14).
//!
//! Every `extern "C"` entrypoint wraps execution in [`std::panic::catch_unwind`], checks raw pointers
//! before dereferencing, and documents preconditions under `# Safety`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

use ecclesia_canvas::Rect;
use ecclesia_ink::stroke::InkPoint;

use crate::access::{
    AccessibilityElementKind, AccessibilityRotorKind, IosAccessibilityTree, RotorDirection,
};
use crate::pencil::{
    HoverPhase, PencilGesture, PencilHoverSample, PencilProcessor, PencilSample, PencilToolMode,
    TouchKind, TouchSampleOrigin,
};
use crate::split::{PaneSlot, SplitEligibility, SplitScreenMetrics, TabletSplitController};
use crate::surface::{TileSurfaceBridge, TileSurfaceKind};
use crate::{IosCanvasError, Result};

/// Status code returned across the C ABI.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FfiStatus {
    /// Operation succeeded.
    Ok = 0,
    /// A required pointer argument was null.
    NullPointer = 1,
    /// A byte slice or UTF-8 string was invalid.
    InvalidUtf8 = 2,
    /// A numeric parameter or enum discriminant was out of range.
    InvalidArgument = 3,
    /// An internal panic was caught at the FFI boundary.
    PanicCaught = 4,
    /// Opening or rendering PDF tiles failed.
    RenderFailed = 5,
    /// Requested index was out of bounds.
    OutOfBounds = 6,
}

/// C-ABI representation of an Apple Pencil or touch sample from UIKit `UITouch`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiPencilSample {
    /// Pointer identifier.
    pub pointer_id: u32,
    /// Touch kind (`0` = pencil, `1` = direct finger, `2` = indirect pointer).
    pub touch_kind: u8,
    /// Sample origin (`0` = actual, `1` = coalesced, `2` = predicted).
    pub origin: u8,
    /// Reserved padding bytes (set to `0`).
    pub reserved: [u8; 2],
    /// X coordinate in view points.
    pub x: f32,
    /// Y coordinate in view points.
    pub y: f32,
    /// `UITouch.force`.
    pub force: f32,
    /// `UITouch.maximumPossibleForce`.
    pub maximum_possible_force: f32,
    /// `UITouch.altitudeAngle` in radians.
    pub altitude_rad: f32,
    /// `UITouch.azimuthAngle` in radians.
    pub azimuth_rad: f32,
    /// Monotonic event timestamp in milliseconds.
    pub timestamp_ms: f64,
    /// Major touch radius in mm (`<= 0.0` if unavailable).
    pub major_radius_mm: f32,
}

/// C-ABI representation of an Apple Pencil hover event.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiHoverEvent {
    /// Hover phase (`0` = entered, `1` = moved, `2` = exited).
    pub phase: u8,
    /// Reserved padding.
    pub reserved: [u8; 3],
    /// X coordinate in view points.
    pub x: f32,
    /// Y coordinate in view points.
    pub y: f32,
    /// Normalized hover height `0.0..=1.0`.
    pub z_offset: f32,
    /// Altitude in radians.
    pub altitude_rad: f32,
    /// Azimuth in radians.
    pub azimuth_rad: f32,
    /// Monotonic timestamp in milliseconds.
    pub timestamp_ms: f64,
}

/// C-ABI descriptor for an uploaded Metal or Core Graphics tile surface.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiTileDescriptor {
    /// Zero-based page index.
    pub page: u32,
    /// Tile zoom bucket.
    pub bucket: i32,
    /// Tile grid column.
    pub col: i32,
    /// Tile grid row.
    pub row: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub bytes_per_row: u32,
    /// Pixel format (`0` = `BGRA8Unorm` Metal, `1` = `RGB8` Core Graphics).
    pub pixel_format: u8,
    /// Reserved padding.
    pub reserved: [u8; 3],
    /// Destination X in screen points/pixels.
    pub screen_x0: f32,
    /// Destination Y in screen points/pixels.
    pub screen_y0: f32,
    /// Destination right edge.
    pub screen_x1: f32,
    /// Destination bottom edge.
    pub screen_y1: f32,
}

/// C-ABI descriptor for one `UIAccessibility` node.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiAccessibilityNode {
    /// Unique element ID.
    pub id: u64,
    /// Element kind (`0` = container, `1` = page, `2` = link, `3` = annotation).
    pub kind: u8,
    /// Reserved padding.
    pub reserved: [u8; 3],
    /// Zero-based page index, or `-1` for document container.
    pub page_index: i32,
    /// Screen frame left.
    pub x0: f32,
    /// Screen frame top.
    pub y0: f32,
    /// Screen frame right.
    pub x1: f32,
    /// Screen frame bottom.
    pub y1: f32,
}

/// C-ABI screen metrics for V.14 tablet two-pane evaluation.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiSplitMetrics {
    /// Screen width in UIKit points.
    pub width_pt: f32,
    /// Screen height in UIKit points.
    pub height_pt: f32,
    /// Divider width in UIKit points.
    pub divider_pt: f32,
}

/// C-ABI result of routing a stylus event in tablet split mode.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfiSplitRouteResult {
    /// Target pane (`0` = primary, `1` = secondary).
    pub pane: u8,
    /// Acceptance (`0` = committed ink, `1` = preview only, `2` = navigation, `3` = rejected palm).
    pub acceptance: u8,
    /// Split eligibility (`0` = two-pane, `1` = single-pane portrait, `2` = single-pane phone).
    pub eligibility: u8,
    /// Active tool mode (`0` = pen, `1` = eraser).
    pub tool_mode: u8,
}

/// Opaque heap-allocated iPadOS canvas handle owned across the C ABI.
pub struct IosCanvasHandle {
    pencil: PencilProcessor,
    surface: TileSurfaceBridge,
    split: TabletSplitController,
    accessibility: IosAccessibilityTree,
}

/// Safe Rust wrapper owning an [`IosCanvasHandle`] and freeing it cleanly on [`Drop`].
#[derive(Debug)]
pub struct SafeIosCanvas {
    raw: NonNull<IosCanvasHandle>,
}

// SAFETY: `IosCanvasHandle` owns all internal state with no thread-local or UIKit pointer references.
unsafe impl Send for SafeIosCanvas {}

impl SafeIosCanvas {
    /// Creates a new iPadOS canvas instance for `surface_kind`, `pixels_per_point`, and `metrics`.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError`] if geometry values are non-positive or non-finite.
    pub fn try_new(
        surface_kind: TileSurfaceKind,
        pixels_per_point: f32,
        metrics: SplitScreenMetrics,
    ) -> Result<Self> {
        let kind_code = match surface_kind {
            TileSurfaceKind::MetalTexture => 0,
            TileSurfaceKind::CoreGraphicsImage => 1,
        };
        let ffi_metrics = FfiSplitMetrics {
            width_pt: metrics.width_dp,
            height_pt: metrics.height_dp,
            divider_pt: metrics.divider_dp,
        };
        let mut out_handle: *mut IosCanvasHandle = ptr::null_mut();
        // SAFETY: `out_handle` is a valid stack pointer for the duration of this call.
        let status = unsafe {
            ecclesia_ios_canvas_create(kind_code, pixels_per_point, &ffi_metrics, &mut out_handle)
        };
        if status != FfiStatus::Ok {
            return Err(IosCanvasError::FfiFailure(format!(
                "canvas create failed with status {status:?}"
            )));
        }
        let raw = NonNull::new(out_handle).ok_or_else(|| {
            IosCanvasError::FfiFailure("canvas create returned null handle".to_owned())
        })?;
        Ok(Self { raw })
    }

    /// Loads PDF bytes and initializes page bounds and `UIAccessibility` elements.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::RenderFailed`] if MuPDF fails to parse the PDF.
    pub fn open_pdf_bytes(&mut self, pdf_bytes: &[u8]) -> Result<()> {
        // SAFETY: `self.raw` is non-null and uniquely owned; `pdf_bytes` pointer is valid for `len` bytes.
        let status = unsafe {
            ecclesia_ios_canvas_open_pdf(self.raw.as_ptr(), pdf_bytes.as_ptr(), pdf_bytes.len())
        };
        if status != FfiStatus::Ok {
            return Err(IosCanvasError::RenderFailed(format!(
                "open_pdf failed with status {status:?}"
            )));
        }
        Ok(())
    }

    /// Routes a UIKit stylus/touch sample through the FFI boundary.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::FfiFailure`] if the sample cannot be decoded.
    pub fn route_touch_down(&mut self, sample: PencilSample) -> Result<FfiSplitRouteResult> {
        let ffi_sample = to_ffi_sample(sample);
        let mut out = FfiSplitRouteResult {
            pane: 0,
            acceptance: 0,
            eligibility: 0,
            tool_mode: 0,
        };
        // SAFETY: `self.raw`, `&ffi_sample`, and `&mut out` are valid aligned pointers.
        let status =
            unsafe { ecclesia_ios_canvas_touch_down(self.raw.as_ptr(), &ffi_sample, &mut out) };
        if status != FfiStatus::Ok {
            return Err(IosCanvasError::FfiFailure(format!(
                "touch_down failed with status {status:?}"
            )));
        }
        Ok(out)
    }

    /// Toggles the pencil tool on squeeze or double-tap and returns the resulting tool mode.
    ///
    /// # Errors
    ///
    /// Returns [`IosCanvasError::FfiFailure`] if the FFI call fails.
    pub fn trigger_pencil_gesture(&mut self, gesture: PencilGesture) -> Result<PencilToolMode> {
        let code = match gesture {
            PencilGesture::DoubleTap => 0,
            PencilGesture::Squeeze => 1,
        };
        let mut tool_out: u8 = 0;
        // SAFETY: `self.raw` and `&mut tool_out` are valid pointers.
        let status =
            unsafe { ecclesia_ios_canvas_pencil_gesture(self.raw.as_ptr(), code, &mut tool_out) };
        if status != FfiStatus::Ok {
            return Err(IosCanvasError::FfiFailure(format!(
                "pencil_gesture failed with status {status:?}"
            )));
        }
        Ok(if tool_out == 1 {
            PencilToolMode::Eraser
        } else {
            PencilToolMode::Pen
        })
    }

    /// Returns the number of `UIAccessibility` elements currently exposed.
    #[must_use]
    pub fn accessibility_count(&self) -> usize {
        // SAFETY: `self.raw` is a valid live handle.
        unsafe { ecclesia_ios_canvas_a11y_count(self.raw.as_ptr()) }
    }

    /// Steps the VoiceOver custom rotor and returns the selected node descriptor, if any.
    #[must_use]
    pub fn rotor_step(
        &self,
        rotor: AccessibilityRotorKind,
        current_id: Option<u64>,
        direction: RotorDirection,
    ) -> Option<FfiAccessibilityNode> {
        let rotor_code = match rotor {
            AccessibilityRotorKind::Pages => 0,
            AccessibilityRotorKind::Headings => 1,
            AccessibilityRotorKind::Links => 2,
            AccessibilityRotorKind::Annotations => 3,
        };
        let dir_code = match direction {
            RotorDirection::Next => 0,
            RotorDirection::Previous => 1,
        };
        let mut out = FfiAccessibilityNode {
            id: 0,
            kind: 0,
            reserved: [0; 3],
            page_index: -1,
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        };
        // SAFETY: `self.raw` and `&mut out` are valid pointers.
        let status = unsafe {
            ecclesia_ios_canvas_a11y_rotor_step(
                self.raw.as_ptr(),
                rotor_code,
                current_id.unwrap_or(0),
                dir_code,
                &mut out,
            )
        };
        (status == FfiStatus::Ok).then_some(out)
    }

    /// Returns a reference to the underlying [`TileSurfaceBridge`].
    #[must_use]
    pub fn surface(&self) -> &TileSurfaceBridge {
        // SAFETY: `self.raw` is uniquely owned by `SafeIosCanvas` for its lifetime.
        unsafe { &self.raw.as_ref().surface }
    }

    /// Returns a reference to the underlying [`TabletSplitController`].
    #[must_use]
    pub fn split(&self) -> &TabletSplitController {
        // SAFETY: `self.raw` is uniquely owned by `SafeIosCanvas` for its lifetime.
        unsafe { &self.raw.as_ref().split }
    }

    /// Returns whether Scribble should be disabled on the UIKit canvas view.
    #[must_use]
    pub fn should_suppress_scribble(&self) -> bool {
        // SAFETY: `self.raw` is uniquely owned by `SafeIosCanvas`.
        unsafe { self.raw.as_ref().pencil.should_suppress_scribble() }
    }

    /// Completes a pencil stroke through the safe wrapper.
    pub fn finish_stroke(
        &mut self,
        pointer_id: u32,
        timestamp_ms: f64,
    ) -> Option<(PaneSlot, Vec<InkPoint>)> {
        // SAFETY: `self.raw` is uniquely owned by `&mut self`.
        let handle = unsafe { self.raw.as_mut() };
        handle.split.route_touch_up(pointer_id, timestamp_ms)
    }
}

impl Drop for SafeIosCanvas {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was allocated by `Box::into_raw` inside `ecclesia_ios_canvas_create`
        // and has not been freed yet.
        unsafe { ecclesia_ios_canvas_destroy(self.raw.as_ptr()) };
    }
}

fn to_ffi_sample(s: PencilSample) -> FfiPencilSample {
    FfiPencilSample {
        pointer_id: s.pointer_id,
        touch_kind: match s.touch_kind {
            TouchKind::Pencil => 0,
            TouchKind::DirectFinger => 1,
            TouchKind::IndirectPointer => 2,
        },
        origin: match s.origin {
            TouchSampleOrigin::Actual => 0,
            TouchSampleOrigin::Coalesced => 1,
            TouchSampleOrigin::Predicted => 2,
        },
        reserved: [0; 2],
        x: s.x,
        y: s.y,
        force: s.force,
        maximum_possible_force: s.maximum_possible_force,
        altitude_rad: s.altitude_rad,
        azimuth_rad: s.azimuth_rad,
        timestamp_ms: s.timestamp_ms,
        major_radius_mm: s.major_radius_mm.unwrap_or(-1.0),
    }
}

fn from_ffi_sample(s: &FfiPencilSample) -> std::result::Result<PencilSample, FfiStatus> {
    let touch_kind = match s.touch_kind {
        0 => TouchKind::Pencil,
        1 => TouchKind::DirectFinger,
        2 => TouchKind::IndirectPointer,
        _ => return Err(FfiStatus::InvalidArgument),
    };
    let origin = match s.origin {
        0 => TouchSampleOrigin::Actual,
        1 => TouchSampleOrigin::Coalesced,
        2 => TouchSampleOrigin::Predicted,
        _ => return Err(FfiStatus::InvalidArgument),
    };
    Ok(PencilSample {
        pointer_id: s.pointer_id,
        x: s.x,
        y: s.y,
        force: s.force,
        maximum_possible_force: s.maximum_possible_force,
        altitude_rad: s.altitude_rad,
        azimuth_rad: s.azimuth_rad,
        timestamp_ms: s.timestamp_ms,
        touch_kind,
        origin,
        major_radius_mm: (s.major_radius_mm > 0.0).then_some(s.major_radius_mm),
    })
}

fn run_ffi<F>(f: F) -> FfiStatus
where
    F: FnOnce() -> FfiStatus,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(status) => status,
        Err(_) => FfiStatus::PanicCaught,
    }
}

/// Allocates a new [`IosCanvasHandle`] and writes its pointer to `out_handle`.
///
/// # Safety
///
/// `metrics` and `out_handle` must be valid, non-null, properly aligned pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_create(
    surface_kind: u8,
    pixels_per_point: f32,
    metrics: *const FfiSplitMetrics,
    out_handle: *mut *mut IosCanvasHandle,
) -> FfiStatus {
    run_ffi(|| {
        if metrics.is_null() || out_handle.is_null() {
            return FfiStatus::NullPointer;
        }
        let kind = match surface_kind {
            0 => TileSurfaceKind::MetalTexture,
            1 => TileSurfaceKind::CoreGraphicsImage,
            _ => return FfiStatus::InvalidArgument,
        };
        // SAFETY: `metrics` was checked non-null and caller guarantees valid alignment/lifetime.
        let m = unsafe { *metrics };
        let Ok(mut surface) = TileSurfaceBridge::try_new(kind, pixels_per_point) else {
            return FfiStatus::InvalidArgument;
        };
        if surface
            .resize(
                (m.width_pt.max(1.0) * pixels_per_point).round() as u32,
                (m.height_pt.max(1.0) * pixels_per_point).round() as u32,
                pixels_per_point,
            )
            .is_err()
        {
            return FfiStatus::InvalidArgument;
        }
        let split = TabletSplitController::new(SplitScreenMetrics {
            width_dp: m.width_pt,
            height_dp: m.height_pt,
            divider_dp: m.divider_pt,
        });
        let handle = Box::new(IosCanvasHandle {
            pencil: PencilProcessor::default(),
            surface,
            split,
            accessibility: IosAccessibilityTree::default(),
        });
        // SAFETY: `out_handle` was checked non-null and receives ownership of the heap handle.
        unsafe { *out_handle = Box::into_raw(handle) };
        FfiStatus::Ok
    })
}

/// Destroys an [`IosCanvasHandle`] created by [`ecclesia_ios_canvas_create`].
///
/// # Safety
///
/// `handle` must be null or a pointer previously returned by [`ecclesia_ios_canvas_create`]
/// that has not already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_destroy(handle: *mut IosCanvasHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !handle.is_null() {
            // SAFETY: caller guarantees `handle` came from `Box::into_raw` and is freed once.
            drop(unsafe { Box::from_raw(handle) });
        }
    }));
}

/// Opens a PDF document from `bytes_ptr` (`len` bytes) and refreshes `UIAccessibility` elements.
///
/// # Safety
///
/// `handle` must be a valid mutable pointer; `bytes_ptr` must point to at least `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_open_pdf(
    handle: *mut IosCanvasHandle,
    bytes_ptr: *const u8,
    len: usize,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || bytes_ptr.is_null() || len == 0 {
            return FfiStatus::NullPointer;
        }
        // SAFETY: `handle` is non-null and exclusively accessed on the caller thread.
        let canvas = unsafe { &mut *handle };
        // SAFETY: `bytes_ptr` is non-null and valid for `len` bytes for this call.
        let pdf_bytes = unsafe { std::slice::from_raw_parts(bytes_ptr, len) };
        if canvas.surface.open_pdf_bytes(pdf_bytes).is_err() {
            return FfiStatus::RenderFailed;
        }
        let empty_texts = vec![String::new(); canvas.surface.page_count()];
        canvas.accessibility = IosAccessibilityTree::rebuild(
            canvas.surface.layout(),
            canvas.surface.viewport(),
            &empty_texts,
            &[],
            &[],
            &[],
        );
        FfiStatus::Ok
    })
}

/// Routes a `UITouch` down sample through tablet split routing and pencil palm rejection.
///
/// # Safety
///
/// `handle`, `sample`, and `out_result` must be valid, non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_touch_down(
    handle: *mut IosCanvasHandle,
    sample: *const FfiPencilSample,
    out_result: *mut FfiSplitRouteResult,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || sample.is_null() || out_result.is_null() {
            return FfiStatus::NullPointer;
        }
        // SAFETY: pointers were checked non-null and caller guarantees alignment/validity.
        let canvas = unsafe { &mut *handle };
        // SAFETY: `sample` is non-null and valid.
        let sample_ref = unsafe { &*sample };
        let parsed = match from_ffi_sample(sample_ref) {
            Ok(s) => s,
            Err(status) => return status,
        };

        let routed = canvas.split.route_touch_down(parsed);
        let active_tool = canvas.split.pane(routed.pane).pencil.tool_mode();
        let result = FfiSplitRouteResult {
            pane: match routed.pane {
                PaneSlot::Primary => 0,
                PaneSlot::Secondary => 1,
            },
            acceptance: match routed.acceptance {
                crate::pencil::SampleAcceptance::CommittedInk => 0,
                crate::pencil::SampleAcceptance::PreviewOnly => 1,
                crate::pencil::SampleAcceptance::PassToNavigation => 2,
                crate::pencil::SampleAcceptance::RejectedPalm => 3,
            },
            eligibility: match canvas.split.eligibility() {
                SplitEligibility::TwoPane => 0,
                SplitEligibility::SinglePanePortrait => 1,
                SplitEligibility::SinglePanePhone => 2,
            },
            tool_mode: match active_tool {
                PencilToolMode::Pen => 0,
                PencilToolMode::Eraser => 1,
            },
        };
        // SAFETY: `out_result` is non-null and valid for writing.
        unsafe { *out_result = result };
        FfiStatus::Ok
    })
}

/// Processes a UIKit `UIPencilInteraction` gesture (`0` = double-tap, `1` = squeeze).
///
/// # Safety
///
/// `handle` and `out_tool_mode` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_pencil_gesture(
    handle: *mut IosCanvasHandle,
    gesture_code: u8,
    out_tool_mode: *mut u8,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || out_tool_mode.is_null() {
            return FfiStatus::NullPointer;
        }
        let gesture = match gesture_code {
            0 => PencilGesture::DoubleTap,
            1 => PencilGesture::Squeeze,
            _ => return FfiStatus::InvalidArgument,
        };
        // SAFETY: `handle` is non-null and valid.
        let canvas = unsafe { &mut *handle };
        let slot = canvas.split.active_pane();
        let _ = canvas
            .split
            .pane_mut(slot)
            .pencil
            .handle_pencil_gesture(gesture);
        let _ = canvas.pencil.handle_pencil_gesture(gesture);
        let mode = match canvas.split.pane(slot).pencil.tool_mode() {
            PencilToolMode::Pen => 0,
            PencilToolMode::Eraser => 1,
        };
        // SAFETY: `out_tool_mode` is non-null and valid for writing.
        unsafe { *out_tool_mode = mode };
        FfiStatus::Ok
    })
}

/// Processes a hover event (`FfiHoverEvent`) on the tablet split canvas.
///
/// # Safety
///
/// `handle` and `event` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_hover(
    handle: *mut IosCanvasHandle,
    event: *const FfiHoverEvent,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || event.is_null() {
            return FfiStatus::NullPointer;
        }
        // SAFETY: `handle` and `event` are non-null and valid.
        let canvas = unsafe { &mut *handle };
        // SAFETY: `event` is non-null and valid.
        let ev = unsafe { &*event };
        let phase = match ev.phase {
            0 => HoverPhase::Entered,
            1 => HoverPhase::Moved,
            2 => HoverPhase::Exited,
            _ => return FfiStatus::InvalidArgument,
        };
        let hover = PencilHoverSample {
            x: ev.x,
            y: ev.y,
            z_offset: ev.z_offset,
            altitude_rad: ev.altitude_rad,
            azimuth_rad: ev.azimuth_rad,
            timestamp_ms: ev.timestamp_ms,
            phase,
        };
        let _ = canvas.split.route_hover(hover);
        FfiStatus::Ok
    })
}

/// Returns the number of `UIAccessibility` elements currently in the tree.
///
/// # Safety
///
/// `handle` must be null or a valid pointer to [`IosCanvasHandle`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_a11y_count(handle: *const IosCanvasHandle) -> usize {
    if handle.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `handle` was checked non-null and caller guarantees validity.
        let canvas = unsafe { &*handle };
        canvas.accessibility.len()
    }))
    .unwrap_or(0)
}

/// Queries the next/previous element on a `UIAccessibilityCustomRotor`.
///
/// # Safety
///
/// `handle` and `out_node` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_a11y_rotor_step(
    handle: *const IosCanvasHandle,
    rotor_code: u8,
    current_id: u64,
    direction_code: u8,
    out_node: *mut FfiAccessibilityNode,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || out_node.is_null() {
            return FfiStatus::NullPointer;
        }
        let rotor = match rotor_code {
            0 => AccessibilityRotorKind::Pages,
            1 => AccessibilityRotorKind::Headings,
            2 => AccessibilityRotorKind::Links,
            3 => AccessibilityRotorKind::Annotations,
            _ => return FfiStatus::InvalidArgument,
        };
        let direction = match direction_code {
            0 => RotorDirection::Next,
            1 => RotorDirection::Previous,
            _ => return FfiStatus::InvalidArgument,
        };
        // SAFETY: `handle` is non-null and valid.
        let canvas = unsafe { &*handle };
        let current = (current_id != 0).then_some(current_id);
        let Some(outcome) = canvas.accessibility.rotor_step(rotor, current, direction) else {
            return FfiStatus::OutOfBounds;
        };
        let kind = match outcome.node.kind {
            AccessibilityElementKind::DocumentContainer => 0,
            AccessibilityElementKind::PageElement => 1,
            AccessibilityElementKind::LinkElement => 2,
            AccessibilityElementKind::AnnotationElement => 3,
        };
        let page_index = outcome
            .node
            .page_index
            .and_then(|idx| i32::try_from(idx).ok())
            .unwrap_or(-1);
        let Rect { x0, y0, x1, y1 } = outcome.node.frame;
        // SAFETY: `out_node` is non-null and valid for writing.
        unsafe {
            *out_node = FfiAccessibilityNode {
                id: outcome.node.id,
                kind,
                reserved: [0; 3],
                page_index,
                x0,
                y0,
                x1,
                y1,
            };
        }
        FfiStatus::Ok
    })
}

/// Placeholder for opaque Darwin `UIView`/`CAMetalLayer` host pointer attachment when building for iOS.
///
/// # Safety
///
/// `handle` and `ca_metal_layer` must be valid, non-null pointers.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_ios_canvas_attach_metal_layer(
    handle: *mut IosCanvasHandle,
    ca_metal_layer: *mut std::ffi::c_void,
) -> FfiStatus {
    run_ffi(|| {
        if handle.is_null() || ca_metal_layer.is_null() {
            return FfiStatus::NullPointer;
        }
        FfiStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

    #[test]
    fn ffi_rejects_null_pointers_safely() {
        // SAFETY: deliberately passing null pointers to verify defensive FFI validation.
        unsafe {
            assert_eq!(
                ecclesia_ios_canvas_create(0, 2.0, ptr::null(), ptr::null_mut()),
                FfiStatus::NullPointer
            );
            assert_eq!(
                ecclesia_ios_canvas_open_pdf(ptr::null_mut(), ptr::null(), 0),
                FfiStatus::NullPointer
            );
            assert_eq!(ecclesia_ios_canvas_a11y_count(ptr::null()), 0);
            ecclesia_ios_canvas_destroy(ptr::null_mut());
        }
    }

    #[test]
    fn safe_wrapper_opens_pdf_routes_pencil_and_steps_a11y_rotor() {
        let rgb = vec![200u8; 32 * 32 * 3];
        let pdf = images_to_pdf(
            &[
                ImageInput::Rgb8 {
                    width: 32,
                    height: 32,
                    dpi: 72.0,
                    samples: rgb.clone(),
                },
                ImageInput::Rgb8 {
                    width: 32,
                    height: 32,
                    dpi: 72.0,
                    samples: rgb,
                },
            ],
            PdfPaperSize::Letter,
            0.0,
        )
        .expect("build pdf");

        let mut canvas = SafeIosCanvas::try_new(
            TileSurfaceKind::MetalTexture,
            2.0,
            SplitScreenMetrics {
                width_dp: 1194.0,
                height_dp: 834.0,
                divider_dp: 8.0,
            },
        )
        .expect("create canvas");
        assert!(canvas.should_suppress_scribble());
        canvas.open_pdf_bytes(&pdf).expect("open pdf");
        assert!(canvas.accessibility_count() >= 3);

        let first = canvas
            .rotor_step(AccessibilityRotorKind::Pages, None, RotorDirection::Next)
            .expect("first page");
        assert_eq!(first.page_index, 0);

        let route = canvas
            .route_touch_down(PencilSample {
                pointer_id: 3,
                x: 900.0,
                y: 200.0,
                force: 1.5,
                maximum_possible_force: 4.0,
                altitude_rad: 1.0,
                azimuth_rad: 0.2,
                timestamp_ms: 50.0,
                touch_kind: TouchKind::Pencil,
                origin: TouchSampleOrigin::Actual,
                major_radius_mm: None,
            })
            .expect("route touch");
        assert_eq!(route.pane, 1);
        assert_eq!(route.acceptance, 0);

        let mode = canvas
            .trigger_pencil_gesture(PencilGesture::Squeeze)
            .expect("squeeze toggle");
        assert_eq!(mode, PencilToolMode::Eraser);
    }
}
