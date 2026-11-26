//! Integration tests for `excellent-canvas-android` covering K.12, U.3, and V.14 public APIs.

use excellent_canvas_android::{
    AndroidAccessibilityAction, AndroidMotionSample, AndroidSurfaceKind, AndroidTileSurfaceBridge,
    HOST_VIEW_ID, MotionAction, MotionToolType, PaneSlot, SafeAndroidCanvas, SampleOrigin,
    SplitEligibility, SplitLinkTarget, SplitScreenMetrics, SurfacePixelFormat,
    TabletSplitController, TileUploadOrder,
};
use excellent_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

#[test]
fn android_canvas_end_to_end_surface_motion_split_and_talkback_tree() {
    let rgb = vec![140u8; 48 * 48 * 3];
    let pdf = images_to_pdf(
        &[
            ImageInput::Rgb8 {
                width: 48,
                height: 48,
                dpi: 72.0,
                samples: rgb.clone(),
            },
            ImageInput::Rgb8 {
                width: 48,
                height: 48,
                dpi: 72.0,
                samples: rgb,
            },
        ],
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
    .expect("create safe android canvas");
    canvas.open_pdf_bytes(&pdf).expect("open pdf");
    assert_eq!(canvas.surface().page_count(), 2);
    assert_eq!(canvas.split().eligibility(), SplitEligibility::TwoPane);

    let root = canvas
        .accessibility()
        .node(HOST_VIEW_ID)
        .expect("accessibility root");
    assert_eq!(root.child_ids.len(), 2);

    let outcome = canvas
        .route_motion_batch(&[
            AndroidMotionSample {
                pointer_id: 4,
                x: 200.0,
                y: 200.0,
                axis_pressure: 0.6,
                axis_tilt_rad: 0.2,
                axis_orientation_rad: 0.1,
                touch_major_px: 0.0,
                event_time_ms: 10.0,
                tool_type: MotionToolType::Stylus,
                action: MotionAction::Down,
                origin: SampleOrigin::Current,
            },
            AndroidMotionSample {
                pointer_id: 4,
                x: 215.0,
                y: 210.0,
                axis_pressure: 0.65,
                axis_tilt_rad: 0.2,
                axis_orientation_rad: 0.1,
                touch_major_px: 0.0,
                event_time_ms: 14.0,
                tool_type: MotionToolType::Stylus,
                action: MotionAction::Move,
                origin: SampleOrigin::Historical,
            },
        ])
        .expect("route motion batch");
    assert_eq!(outcome.pane, 0);
    assert_eq!(outcome.acceptance, 0);
}

#[test]
fn android_split_and_surface_tiles_work_independently() {
    let mut split = TabletSplitController::new(SplitScreenMetrics {
        width_dp: 1280.0,
        height_dp: 800.0,
        divider_dp: 8.0,
        density: 2.0,
    });
    let other = split.open_link_in_other_pane(
        PaneSlot::Primary,
        SplitLinkTarget {
            page: 5,
            scroll_y: 60.0,
        },
    );
    assert_eq!(other, Some(PaneSlot::Secondary));
    assert_eq!(split.pane(PaneSlot::Primary).page, 0);
    assert_eq!(split.pane(PaneSlot::Secondary).page, 5);

    let mut bridge = AndroidTileSurfaceBridge::try_new(
        AndroidSurfaceKind::OpenGlEsTexture,
        SurfacePixelFormat::Rgb565,
        2.0,
    )
    .expect("bridge");
    bridge.resize(1280, 800, 2.0).expect("resize");
    assert!(
        bridge
            .visible_tile_keys(TileUploadOrder::TopToBottom)
            .is_empty()
    );
    let _ = AndroidAccessibilityAction::AccessibilityFocus;
}
