//! Integration tests for `excellent-canvas-ios` covering K.11, U.3, and V.14 public APIs.

use std::f32::consts::FRAC_PI_2;

use excellent_canvas_ios::{
    AccessibilityRotorKind, FfiStatus, HoverPhase, PencilGesture, PencilHoverSample, PencilSample,
    PencilToolMode, RotorDirection, SafeIosCanvas, SampleAcceptance, SplitEligibility,
    SplitLinkTarget, SplitScreenMetrics, TabletSplitController, TileSurfaceKind, TileUploadOrder,
    TouchKind, TouchSampleOrigin,
};
use excellent_core::export_doc::{ImageInput, PdfPaperSize, images_to_pdf};

#[test]
fn ios_canvas_end_to_end_metal_pencil_split_and_voiceover_rotor() {
    let rgb = vec![120u8; 48 * 48 * 3];
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
        PdfPaperSize::A4,
        0.0,
    )
    .expect("build sample pdf");

    let mut canvas = SafeIosCanvas::try_new(
        TileSurfaceKind::MetalTexture,
        2.0,
        SplitScreenMetrics {
            width_dp: 1366.0,
            height_dp: 1024.0,
            divider_dp: 8.0,
        },
    )
    .expect("create safe ios canvas");
    canvas.open_pdf_bytes(&pdf).expect("open pdf bytes");
    assert_eq!(canvas.surface().page_count(), 2);
    assert_eq!(canvas.split().eligibility(), SplitEligibility::TwoPane);

    let first_page = canvas
        .rotor_step(AccessibilityRotorKind::Pages, None, RotorDirection::Next)
        .expect("page 1 rotor element");
    assert_eq!(first_page.page_index, 0);
    let second_page = canvas
        .rotor_step(
            AccessibilityRotorKind::Pages,
            Some(first_page.id),
            RotorDirection::Next,
        )
        .expect("page 2 rotor element");
    assert_eq!(second_page.page_index, 1);

    let outcome = canvas
        .route_touch_down(PencilSample {
            pointer_id: 11,
            x: 1000.0,
            y: 350.0,
            force: 2.5,
            maximum_possible_force: 4.0,
            altitude_rad: FRAC_PI_2 - 0.3,
            azimuth_rad: 0.7,
            timestamp_ms: 25.0,
            touch_kind: TouchKind::Pencil,
            origin: TouchSampleOrigin::Actual,
            major_radius_mm: None,
        })
        .expect("route pencil touch");
    assert_eq!(outcome.pane, 1);
    assert_eq!(outcome.acceptance, 0);

    let mode = canvas
        .trigger_pencil_gesture(PencilGesture::DoubleTap)
        .expect("double tap toggle");
    assert_eq!(mode, PencilToolMode::Eraser);

    let _ = FfiStatus::Ok;
}

#[test]
fn split_controller_routes_hover_and_v9_links_independently() {
    let mut split = TabletSplitController::new(SplitScreenMetrics {
        width_dp: 1180.0,
        height_dp: 820.0,
        divider_dp: 8.0,
    });
    assert!(split.is_two_pane_active());

    let hover_pane = split.route_hover(PencilHoverSample {
        x: 900.0,
        y: 250.0,
        z_offset: 0.3,
        altitude_rad: 1.0,
        azimuth_rad: 0.2,
        timestamp_ms: 40.0,
        phase: HoverPhase::Moved,
    });
    assert_eq!(hover_pane, excellent_canvas_ios::PaneSlot::Secondary);
    assert!(
        split
            .pane(excellent_canvas_ios::PaneSlot::Primary)
            .pencil
            .hover_state()
            .is_none()
    );

    let opened = split.open_link_in_other_pane(
        excellent_canvas_ios::PaneSlot::Primary,
        SplitLinkTarget {
            page: 7,
            scroll_y: 120.0,
        },
    );
    assert_eq!(opened, Some(excellent_canvas_ios::PaneSlot::Secondary));
    assert_eq!(split.pane(excellent_canvas_ios::PaneSlot::Primary).page, 0);
    assert_eq!(
        split.pane(excellent_canvas_ios::PaneSlot::Secondary).page,
        7
    );

    // Confirm surface upload order compiles and executes via integration crate boundary.
    let mut bridge =
        excellent_canvas_ios::TileSurfaceBridge::try_new(TileSurfaceKind::CoreGraphicsImage, 2.0)
            .expect("cg bridge");
    bridge.resize(1180, 820, 2.0).expect("resize");
    let keys = bridge.visible_tile_keys(TileUploadOrder::CenterOut);
    assert!(keys.is_empty());
    let _ = SampleAcceptance::CommittedInk;
}
