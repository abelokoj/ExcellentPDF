//! JNI-compatible and C-ABI FFI bridge for the Android Kotlin `EcclesiaCanvasView` (plan K.12, U.3, V.14).
//!
//! Exports both primitive-signature `Java_com_ecclesiapdf_canvas_NativeCanvasBridge_*` JNI symbols
//! (callable directly from Kotlin `external fun` without requiring a compile-time `jni` crate dependency)
//! and a portable C-ABI + safe Rust wrapper [`SafeAndroidCanvas`] verified on all host targets.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

use crate::access::{AndroidAccessibilityAction, AndroidAccessibilityTree, HOST_VIEW_ID};
use crate::motion::{
    AndroidMotionSample, MotionAcceptance, MotionAction, MotionToolType, SampleOrigin,
};
use crate::split::{PaneSlot, SplitEligibility, SplitScreenMetrics, TabletSplitController};
use crate::surface::{AndroidSurfaceKind, AndroidTileSurfaceBridge, SurfacePixelFormat};
use crate::{AndroidCanvasError, Result};

/// Status code returned across the Android FFI/JNI bridge.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FfiAndroidStatus {
    /// Operation succeeded.
    Ok = 0,
    /// A required pointer or handle was null/zero.
    NullPointer = 1,
    /// A numeric parameter or discriminant was invalid.
    InvalidArgument = 2,
    /// An internal panic was caught at the FFI boundary.
    PanicCaught = 3,
    /// Opening or rendering PDF tiles failed.
    RenderFailed = 4,
    /// Requested node or index was out of bounds.
    OutOfBounds = 5,
}

/// C-ABI representation of an Android `MotionEvent` sample.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiAndroidMotionSample {
    /// Pointer ID (`MotionEvent.getPointerId`).
    pub pointer_id: u32,
    /// Action (`0`=down, `1`=move, `2`=up, `3`=cancel, `4`=hover_enter, `5`=hover_move, `6`=hover_exit).
    pub action: u8,
    /// Tool type (`0`=stylus, `1`=eraser, `2`=finger, `3`=mouse, `4`=unknown).
    pub tool_type: u8,
    /// Origin (`0`=historical, `1`=current, `2`=predicted).
    pub origin: u8,
    /// Reserved padding.
    pub reserved: u8,
    /// View X in pixels.
    pub x: f32,
    /// View Y in pixels.
    pub y: f32,
    /// `AXIS_PRESSURE`.
    pub axis_pressure: f32,
    /// `AXIS_TILT` in radians.
    pub axis_tilt_rad: f32,
    /// `AXIS_ORIENTATION` in radians.
    pub axis_orientation_rad: f32,
    /// `AXIS_TOUCH_MAJOR` in pixels.
    pub touch_major_px: f32,
    /// Event timestamp in milliseconds.
    pub event_time_ms: f64,
}

/// C-ABI result of routing an Android motion sample through tablet split routing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfiAndroidRouteResult {
    /// Pane (`0` = primary, `1` = secondary).
    pub pane: u8,
    /// Acceptance (`0` = committed ink, `1` = front buffer preview, `2` = navigation, `3` = rejected palm).
    pub acceptance: u8,
    /// Split eligibility (`0` = two-pane, `1` = single-pane portrait, `2` = single-pane phone).
    pub eligibility: u8,
    /// Reserved padding.
    pub reserved: u8,
}

/// Opaque heap-allocated Android canvas handle.
pub struct AndroidCanvasHandle {
    surface: AndroidTileSurfaceBridge,
    split: TabletSplitController,
    accessibility: AndroidAccessibilityTree,
}

/// Safe Rust wrapper owning an [`AndroidCanvasHandle`] and releasing it cleanly on [`Drop`].
#[derive(Debug)]
pub struct SafeAndroidCanvas {
    raw: NonNull<AndroidCanvasHandle>,
}

// SAFETY: `AndroidCanvasHandle` owns all internal Rust state with no JVM references stored inside.
unsafe impl Send for SafeAndroidCanvas {}

impl SafeAndroidCanvas {
    /// Creates a new Android canvas handle.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::BridgeFailure`] if screen metrics are invalid.
    pub fn try_new(
        surface_kind: AndroidSurfaceKind,
        pixel_format: SurfacePixelFormat,
        metrics: SplitScreenMetrics,
    ) -> Result<Self> {
        let surface_code = match surface_kind {
            AndroidSurfaceKind::HardwareBuffer => 0,
            AndroidSurfaceKind::NativeWindowRgba => 1,
            AndroidSurfaceKind::OpenGlEsTexture => 2,
        };
        let fmt_code = match pixel_format {
            SurfacePixelFormat::Rgba8888 => 0,
            SurfacePixelFormat::Rgb565 => 1,
        };
        let mut out: *mut AndroidCanvasHandle = ptr::null_mut();
        // SAFETY: `out` is a valid stack pointer for the duration of the call.
        let status = unsafe {
            ecclesia_android_canvas_create(
                surface_code,
                fmt_code,
                metrics.width_dp,
                metrics.height_dp,
                metrics.divider_dp,
                metrics.density,
                &mut out,
            )
        };
        if status != FfiAndroidStatus::Ok {
            return Err(AndroidCanvasError::BridgeFailure(format!(
                "android canvas create failed: {status:?}"
            )));
        }
        let raw = NonNull::new(out).ok_or_else(|| {
            AndroidCanvasError::BridgeFailure("android canvas create returned null".to_owned())
        })?;
        Ok(Self { raw })
    }

    /// Loads PDF bytes and refreshes the `AccessibilityNodeProvider` tree.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::RenderFailed`] if MuPDF fails to open the document.
    pub fn open_pdf_bytes(&mut self, pdf_bytes: &[u8]) -> Result<()> {
        // SAFETY: `self.raw` is non-null and uniquely owned; `pdf_bytes` is valid for `len` bytes.
        let status = unsafe {
            ecclesia_android_canvas_open_pdf(self.raw.as_ptr(), pdf_bytes.as_ptr(), pdf_bytes.len())
        };
        if status != FfiAndroidStatus::Ok {
            return Err(AndroidCanvasError::RenderFailed(format!(
                "android open_pdf failed: {status:?}"
            )));
        }
        Ok(())
    }

    /// Feeds a batch of `AndroidMotionSample`s through the FFI boundary.
    ///
    /// # Errors
    ///
    /// Returns [`AndroidCanvasError::BridgeFailure`] if the batch is empty or contains invalid discriminants.
    pub fn route_motion_batch(
        &mut self,
        samples: &[AndroidMotionSample],
    ) -> Result<FfiAndroidRouteResult> {
        let ffi_samples: Vec<FfiAndroidMotionSample> = samples.iter().map(to_ffi_sample).collect();
        let mut out = FfiAndroidRouteResult {
            pane: 0,
            acceptance: 0,
            eligibility: 0,
            reserved: 0,
        };
        // SAFETY: `self.raw`, `ffi_samples`, and `&mut out` are valid aligned pointers.
        let status = unsafe {
            ecclesia_android_canvas_motion_batch(
                self.raw.as_ptr(),
                ffi_samples.as_ptr(),
                ffi_samples.len(),
                &mut out,
            )
        };
        if status != FfiAndroidStatus::Ok {
            return Err(AndroidCanvasError::BridgeFailure(format!(
                "motion batch failed: {status:?}"
            )));
        }
        Ok(out)
    }

    /// Returns a reference to the underlying [`AndroidAccessibilityTree`].
    #[must_use]
    pub fn accessibility(&self) -> &AndroidAccessibilityTree {
        // SAFETY: `self.raw` is uniquely owned for the lifetime of `self`.
        unsafe { &self.raw.as_ref().accessibility }
    }

    /// Returns a reference to the underlying [`AndroidTileSurfaceBridge`].
    #[must_use]
    pub fn surface(&self) -> &AndroidTileSurfaceBridge {
        // SAFETY: `self.raw` is uniquely owned for the lifetime of `self`.
        unsafe { &self.raw.as_ref().surface }
    }

    /// Returns a reference to the underlying [`TabletSplitController`].
    #[must_use]
    pub fn split(&self) -> &TabletSplitController {
        // SAFETY: `self.raw` is uniquely owned for the lifetime of `self`.
        unsafe { &self.raw.as_ref().split }
    }
}

impl Drop for SafeAndroidCanvas {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was allocated by `Box::into_raw` in `ecclesia_android_canvas_create`.
        unsafe { ecclesia_android_canvas_destroy(self.raw.as_ptr()) };
    }
}

fn to_ffi_sample(s: &AndroidMotionSample) -> FfiAndroidMotionSample {
    FfiAndroidMotionSample {
        pointer_id: s.pointer_id,
        action: match s.action {
            MotionAction::Down => 0,
            MotionAction::Move => 1,
            MotionAction::Up => 2,
            MotionAction::Cancel => 3,
            MotionAction::HoverEnter => 4,
            MotionAction::HoverMove => 5,
            MotionAction::HoverExit => 6,
        },
        tool_type: match s.tool_type {
            MotionToolType::Stylus => 0,
            MotionToolType::Eraser => 1,
            MotionToolType::Finger => 2,
            MotionToolType::Mouse => 3,
            MotionToolType::Unknown => 4,
        },
        origin: match s.origin {
            SampleOrigin::Historical => 0,
            SampleOrigin::Current => 1,
            SampleOrigin::Predicted => 2,
        },
        reserved: 0,
        x: s.x,
        y: s.y,
        axis_pressure: s.axis_pressure,
        axis_tilt_rad: s.axis_tilt_rad,
        axis_orientation_rad: s.axis_orientation_rad,
        touch_major_px: s.touch_major_px,
        event_time_ms: s.event_time_ms,
    }
}

fn from_ffi_sample(
    s: &FfiAndroidMotionSample,
) -> std::result::Result<AndroidMotionSample, FfiAndroidStatus> {
    let action = match s.action {
        0 => MotionAction::Down,
        1 => MotionAction::Move,
        2 => MotionAction::Up,
        3 => MotionAction::Cancel,
        4 => MotionAction::HoverEnter,
        5 => MotionAction::HoverMove,
        6 => MotionAction::HoverExit,
        _ => return Err(FfiAndroidStatus::InvalidArgument),
    };
    let tool_type = match s.tool_type {
        0 => MotionToolType::Stylus,
        1 => MotionToolType::Eraser,
        2 => MotionToolType::Finger,
        3 => MotionToolType::Mouse,
        4 => MotionToolType::Unknown,
        _ => return Err(FfiAndroidStatus::InvalidArgument),
    };
    let origin = match s.origin {
        0 => SampleOrigin::Historical,
        1 => SampleOrigin::Current,
        2 => SampleOrigin::Predicted,
        _ => return Err(FfiAndroidStatus::InvalidArgument),
    };
    Ok(AndroidMotionSample {
        pointer_id: s.pointer_id,
        x: s.x,
        y: s.y,
        axis_pressure: s.axis_pressure,
        axis_tilt_rad: s.axis_tilt_rad,
        axis_orientation_rad: s.axis_orientation_rad,
        touch_major_px: s.touch_major_px,
        event_time_ms: s.event_time_ms,
        tool_type,
        action,
        origin,
    })
}

fn run_ffi<F>(f: F) -> FfiAndroidStatus
where
    F: FnOnce() -> FfiAndroidStatus,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(status) => status,
        Err(_) => FfiAndroidStatus::PanicCaught,
    }
}

/// Allocates a new [`AndroidCanvasHandle`] and stores it in `out_handle`.
///
/// # Safety
///
/// `out_handle` must be a valid non-null aligned pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_android_canvas_create(
    surface_kind: u8,
    pixel_format: u8,
    width_dp: f32,
    height_dp: f32,
    divider_dp: f32,
    density: f32,
    out_handle: *mut *mut AndroidCanvasHandle,
) -> FfiAndroidStatus {
    run_ffi(|| {
        if out_handle.is_null() {
            return FfiAndroidStatus::NullPointer;
        }
        let kind = match surface_kind {
            0 => AndroidSurfaceKind::HardwareBuffer,
            1 => AndroidSurfaceKind::NativeWindowRgba,
            2 => AndroidSurfaceKind::OpenGlEsTexture,
            _ => return FfiAndroidStatus::InvalidArgument,
        };
        let fmt = match pixel_format {
            0 => SurfacePixelFormat::Rgba8888,
            1 => SurfacePixelFormat::Rgb565,
            _ => return FfiAndroidStatus::InvalidArgument,
        };
        let Ok(mut surface) = AndroidTileSurfaceBridge::try_new(kind, fmt, density) else {
            return FfiAndroidStatus::InvalidArgument;
        };
        if surface
            .resize(
                (width_dp.max(1.0) * density).round() as u32,
                (height_dp.max(1.0) * density).round() as u32,
                density,
            )
            .is_err()
        {
            return FfiAndroidStatus::InvalidArgument;
        }
        let split = TabletSplitController::new(SplitScreenMetrics {
            width_dp,
            height_dp,
            divider_dp,
            density,
        });
        let handle = Box::new(AndroidCanvasHandle {
            surface,
            split,
            accessibility: AndroidAccessibilityTree::default(),
        });
        // SAFETY: `out_handle` was checked non-null and receives ownership of `handle`.
        unsafe { *out_handle = Box::into_raw(handle) };
        FfiAndroidStatus::Ok
    })
}

/// Destroys an [`AndroidCanvasHandle`].
///
/// # Safety
///
/// `handle` must be null or a valid pointer previously returned by [`ecclesia_android_canvas_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_android_canvas_destroy(handle: *mut AndroidCanvasHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !handle.is_null() {
            // SAFETY: caller guarantees `handle` is uniquely owned and freed at most once.
            drop(unsafe { Box::from_raw(handle) });
        }
    }));
}

/// Opens raw PDF bytes and refreshes the `AccessibilityNodeProvider` tree.
///
/// # Safety
///
/// `handle` must be a valid mutable pointer and `bytes_ptr` must be valid for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_android_canvas_open_pdf(
    handle: *mut AndroidCanvasHandle,
    bytes_ptr: *const u8,
    len: usize,
) -> FfiAndroidStatus {
    run_ffi(|| {
        if handle.is_null() || bytes_ptr.is_null() || len == 0 {
            return FfiAndroidStatus::NullPointer;
        }
        // SAFETY: `handle` is non-null and exclusively borrowed for this call.
        let canvas = unsafe { &mut *handle };
        // SAFETY: `bytes_ptr` is non-null and readable for `len` bytes.
        let pdf_bytes = unsafe { std::slice::from_raw_parts(bytes_ptr, len) };
        if canvas.surface.open_pdf_bytes(pdf_bytes).is_err() {
            return FfiAndroidStatus::RenderFailed;
        }
        let empty = vec![String::new(); canvas.surface.page_count()];
        canvas.accessibility = AndroidAccessibilityTree::rebuild(
            canvas.surface.layout(),
            canvas.surface.viewport(),
            &empty,
            &[],
            &[],
        );
        FfiAndroidStatus::Ok
    })
}

/// Processes a batch of Android `MotionEvent` samples (historical + current + predicted).
///
/// # Safety
///
/// `handle`, `samples_ptr`, and `out_result` must be valid non-null pointers, with `samples_ptr`
/// readable for `count` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ecclesia_android_canvas_motion_batch(
    handle: *mut AndroidCanvasHandle,
    samples_ptr: *const FfiAndroidMotionSample,
    count: usize,
    out_result: *mut FfiAndroidRouteResult,
) -> FfiAndroidStatus {
    run_ffi(|| {
        if handle.is_null() || samples_ptr.is_null() || out_result.is_null() {
            return FfiAndroidStatus::NullPointer;
        }
        if count == 0 {
            return FfiAndroidStatus::InvalidArgument;
        }
        // SAFETY: `handle` is non-null and valid.
        let canvas = unsafe { &mut *handle };
        // SAFETY: `samples_ptr` is non-null and valid for `count` items.
        let raw_samples = unsafe { std::slice::from_raw_parts(samples_ptr, count) };
        let mut parsed = Vec::with_capacity(count);
        for s in raw_samples {
            match from_ffi_sample(s) {
                Ok(sample) => parsed.push(sample),
                Err(status) => return status,
            }
        }
        let Some(outcome) = canvas.split.route_motion_batch(&parsed) else {
            return FfiAndroidStatus::InvalidArgument;
        };
        let result = FfiAndroidRouteResult {
            pane: match outcome.pane {
                PaneSlot::Primary => 0,
                PaneSlot::Secondary => 1,
            },
            acceptance: match outcome.acceptance {
                MotionAcceptance::CommittedInk => 0,
                MotionAcceptance::FrontBufferPreview => 1,
                MotionAcceptance::PassToNavigation => 2,
                MotionAcceptance::RejectedPalm => 3,
            },
            eligibility: match canvas.split.eligibility() {
                SplitEligibility::TwoPane => 0,
                SplitEligibility::SinglePanePortrait => 1,
                SplitEligibility::SinglePanePhone => 2,
            },
            reserved: 0,
        };
        // SAFETY: `out_result` is non-null and valid for writing.
        unsafe { *out_result = result };
        FfiAndroidStatus::Ok
    })
}

// ---------------------------------------------------------------------------
// JNI primitive entrypoints for Kotlin `com.ecclesiapdf.canvas.NativeCanvasBridge`
// ---------------------------------------------------------------------------

/// JNI `create(surfaceKind, pixelFormat, widthDp, heightDp, dividerDp, density) -> jlong`.
///
/// # Safety
///
/// Must be called from JNI with valid JVM thread state.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeCreate(
    _env: *mut c_void,
    _class: *mut c_void,
    surface_kind: i32,
    pixel_format: i32,
    width_dp: f32,
    height_dp: f32,
    divider_dp: f32,
    density: f32,
) -> i64 {
    let mut handle: *mut AndroidCanvasHandle = ptr::null_mut();
    let kind = u8::try_from(surface_kind).unwrap_or(255);
    let fmt = u8::try_from(pixel_format).unwrap_or(255);
    // SAFETY: `&mut handle` is a valid aligned pointer on the stack.
    let status = unsafe {
        ecclesia_android_canvas_create(
            kind,
            fmt,
            width_dp,
            height_dp,
            divider_dp,
            density,
            &mut handle,
        )
    };
    if status == FfiAndroidStatus::Ok {
        handle as usize as i64
    } else {
        0
    }
}

/// JNI `destroy(handle)`.
///
/// # Safety
///
/// `handle` must be `0` or a handle returned by `nativeCreate` that has not yet been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeDestroy(
    _env: *mut c_void,
    _class: *mut c_void,
    handle: i64,
) {
    let ptr =
        usize::try_from(handle).map_or(ptr::null_mut(), |addr| addr as *mut AndroidCanvasHandle);
    // SAFETY: caller guarantees `handle` came from `nativeCreate` and is freed once.
    unsafe { ecclesia_android_canvas_destroy(ptr) };
}

/// JNI `onMotionSample(handle, pointerId, action, toolType, origin, x, y, pressure, tiltRad, orientationRad, touchMajorPx, timeMs) -> jint`.
///
/// Returns packed `(pane << 8) | acceptance`, or `-1` on invalid handle/arguments.
///
/// # Safety
///
/// `handle` must be a live handle returned by `nativeCreate`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeOnMotionSample(
    _env: *mut c_void,
    _class: *mut c_void,
    handle: i64,
    pointer_id: i32,
    action: i32,
    tool_type: i32,
    origin: i32,
    x: f32,
    y: f32,
    axis_pressure: f32,
    axis_tilt_rad: f32,
    axis_orientation_rad: f32,
    touch_major_px: f32,
    event_time_ms: f64,
) -> i32 {
    let Ok(addr) = usize::try_from(handle) else {
        return -1;
    };
    if addr == 0 {
        return -1;
    }
    let ptr = addr as *mut AndroidCanvasHandle;
    let sample = FfiAndroidMotionSample {
        pointer_id: u32::try_from(pointer_id).unwrap_or(0),
        action: u8::try_from(action).unwrap_or(255),
        tool_type: u8::try_from(tool_type).unwrap_or(255),
        origin: u8::try_from(origin).unwrap_or(255),
        reserved: 0,
        x,
        y,
        axis_pressure,
        axis_tilt_rad,
        axis_orientation_rad,
        touch_major_px,
        event_time_ms,
    };
    let mut out = FfiAndroidRouteResult {
        pane: 0,
        acceptance: 0,
        eligibility: 0,
        reserved: 0,
    };
    // SAFETY: `ptr` was validated non-zero from `nativeCreate`; `&sample` and `&mut out` are stack pointers.
    let status = unsafe { ecclesia_android_canvas_motion_batch(ptr, &sample, 1, &mut out) };
    if status != FfiAndroidStatus::Ok {
        return -1;
    }
    (i32::from(out.pane) << 8) | i32::from(out.acceptance)
}

/// JNI `accessibilityPerformAction(handle, virtualViewId, actionCode) -> jint`.
///
/// # Safety
///
/// `handle` must be a live handle returned by `nativeCreate`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativePerformAccessibilityAction(
    _env: *mut c_void,
    _class: *mut c_void,
    handle: i64,
    virtual_view_id: i32,
    action_code: i32,
) -> i32 {
    let Ok(addr) = usize::try_from(handle) else {
        return 0;
    };
    if addr == 0 {
        return 0;
    }
    let ptr = addr as *mut AndroidCanvasHandle;
    let action = match action_code {
        16 => AndroidAccessibilityAction::Click,
        64 => AndroidAccessibilityAction::AccessibilityFocus,
        128 => AndroidAccessibilityAction::ClearAccessibilityFocus,
        4096 => AndroidAccessibilityAction::ScrollForward,
        8192 => AndroidAccessibilityAction::ScrollBackward,
        _ => return 0,
    };
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `ptr` is non-null and exclusively accessed on the UI thread.
        let canvas = unsafe { &mut *ptr };
        let vid = if virtual_view_id == 0 {
            HOST_VIEW_ID
        } else {
            virtual_view_id
        };
        let outcome = canvas.accessibility.perform_action(vid, action);
        i32::from(!matches!(outcome, crate::access::ActionOutcome::Ignored))
    }))
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

    #[test]
    fn android_ffi_handles_nulls_and_routes_stylus_over_jni_boundary() {
        // SAFETY: testing defensive null-pointer checks.
        unsafe {
            assert_eq!(
                ecclesia_android_canvas_create(0, 0, 1280.0, 800.0, 8.0, 2.0, ptr::null_mut()),
                FfiAndroidStatus::NullPointer
            );
            assert_eq!(
                ecclesia_android_canvas_open_pdf(ptr::null_mut(), ptr::null(), 0),
                FfiAndroidStatus::NullPointer
            );
            ecclesia_android_canvas_destroy(ptr::null_mut());
        }

        let rgb = vec![180u8; 32 * 32 * 3];
        let pdf = images_to_pdf(
            &[ImageInput::Rgb8 {
                width: 32,
                height: 32,
                dpi: 72.0,
                samples: rgb,
            }],
            PdfPaperSize::Letter,
            0.0,
        )
        .expect("build pdf");

        let mut canvas = SafeAndroidCanvas::try_new(
            AndroidSurfaceKind::HardwareBuffer,
            SurfacePixelFormat::Rgba8888,
            SplitScreenMetrics {
                width_dp: 1280.0,
                height_dp: 800.0,
                divider_dp: 8.0,
                density: 2.0,
            },
        )
        .expect("create android canvas");
        canvas.open_pdf_bytes(&pdf).expect("open pdf");
        assert!(canvas.accessibility().node(HOST_VIEW_ID).is_some());

        let routed = canvas
            .route_motion_batch(&[AndroidMotionSample {
                pointer_id: 2,
                x: 2000.0,
                y: 300.0,
                axis_pressure: 0.7,
                axis_tilt_rad: 0.3,
                axis_orientation_rad: 1.0,
                touch_major_px: 0.0,
                event_time_ms: 42.0,
                tool_type: MotionToolType::Stylus,
                action: MotionAction::Down,
                origin: SampleOrigin::Current,
            }])
            .expect("route");
        assert_eq!(routed.pane, 1);
        assert_eq!(routed.acceptance, 0);
    }

    #[test]
    fn jni_primitive_lifecycle_round_trips() {
        // SAFETY: exercising `nativeCreate`, `nativeOnMotionSample`, and `nativeDestroy` with valid args.
        unsafe {
            let handle = Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeCreate(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                1280.0,
                800.0,
                8.0,
                2.0,
            );
            assert_ne!(handle, 0);
            let packed = Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeOnMotionSample(
                ptr::null_mut(),
                ptr::null_mut(),
                handle,
                1,
                0,
                0,
                1,
                1800.0,
                200.0,
                0.8,
                0.2,
                0.5,
                0.0,
                100.0,
            );
            assert_eq!(packed >> 8, 1);
            assert_eq!(packed & 0xff, 0);
            Java_com_ecclesiapdf_canvas_NativeCanvasBridge_nativeDestroy(
                ptr::null_mut(),
                ptr::null_mut(),
                handle,
            );
        }
    }
}
