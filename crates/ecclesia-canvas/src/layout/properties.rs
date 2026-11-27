//! Property tests for every layout (plan K.6, item 4): pages never overlap, stay inside the layout, belong to exactly
//! one band, and the queries find each of them; and the timing of a 10,000-page layout.

use proptest::prelude::*;

use super::{Direction, Flow, Layout, Mode, PageSize, Spec};
use crate::Rect;

fn size() -> impl Strategy<Value = PageSize> {
    (50.0f32..2000.0, 50.0f32..2000.0).prop_map(|(width, height)| PageSize { width, height })
}

fn spec() -> impl Strategy<Value = Spec> {
    let mode = prop_oneof![
        Just(Mode::Single),
        Just(Mode::Facing),
        Just(Mode::Book),
        Just(Mode::Horizontal),
        (1u8..8).prop_map(|columns| Mode::Grid { columns }),
    ];
    let flow = prop_oneof![Just(Flow::Continuous), Just(Flow::Paged)];
    let direction = prop_oneof![Just(Direction::LeftToRight), Just(Direction::RightToLeft)];
    (mode, flow, direction).prop_map(|(mode, flow, direction)| Spec {
        mode,
        flow,
        direction,
    })
}

/// Overlap with some room for rounding: touching pages are fine.
fn overlap(a: &Rect, b: &Rect) -> bool {
    const SLACK: f32 = 0.01;
    a.x0 + SLACK < b.x1 && b.x0 + SLACK < a.x1 && a.y0 + SLACK < b.y1 && b.y0 + SLACK < a.y1
}

proptest! {
    #[test]
    fn pages_never_overlap_and_stay_inside(sizes in prop::collection::vec(size(), 0..120), spec in spec()) {
        let layout = Layout::new(&sizes, spec);
        prop_assert_eq!(layout.len(), sizes.len());
        for (i, page) in layout.pages().iter().enumerate() {
            prop_assert!((page.width() - sizes[i].width).abs() < 0.01, "page {} keeps its size", i);
            prop_assert!(page.x0 >= 0.0 && page.y0 >= 0.0, "page {} at {:?}", i, page);
            prop_assert!(page.x1 <= layout.width() + 0.01 && page.y1 <= layout.height() + 0.01);
            for (j, other) in layout.pages().iter().enumerate().skip(i + 1) {
                prop_assert!(!overlap(page, other), "pages {} and {} overlap: {:?} {:?}", i, j, page, other);
            }
        }
    }

    #[test]
    fn bands_hold_every_page_once_in_order(sizes in prop::collection::vec(size(), 0..120), spec in spec()) {
        let layout = Layout::new(&sizes, spec);
        let mut seen = vec![0u8; sizes.len()];
        let axis = layout.axis();
        for (b, band) in layout.bands().iter().enumerate() {
            if let Some(next) = layout.bands().get(b + 1) {
                prop_assert!(band.end <= next.start, "bands {} and {} overlap", b, b + 1);
            }
            for page in band.pages.clone() {
                seen[page] += 1;
                prop_assert_eq!(layout.band_of(page), Some(b));
                let (from, to) = axis.span(&layout.page(page).expect("page in a band"));
                prop_assert!(from >= band.start - 0.01 && to <= band.end + 0.01);
            }
        }
        prop_assert!(seen.iter().all(|&n| n == 1), "{:?}", seen);
    }

    #[test]
    fn queries_find_every_page(sizes in prop::collection::vec(size(), 1..60), spec in spec()) {
        let layout = Layout::new(&sizes, spec);
        let everything = Rect::new(0.0, 0.0, layout.width(), layout.height());
        prop_assert_eq!(layout.pages_in(&everything).count(), layout.len());
        for (i, page) in layout.pages().iter().enumerate() {
            let (cx, cy) = ((page.x0 + page.x1) / 2.0, (page.y0 + page.y1) / 2.0);
            prop_assert_eq!(layout.page_at(cx, cy), Some(i));
            let along = match layout.axis() {
                super::Axis::Vertical => cy,
                super::Axis::Horizontal => cx,
            };
            prop_assert_eq!(layout.band_at(along), layout.band_of(i));
        }
    }
}

/// K.6's budget: 10,000 pages in under 5 ms. Timing only means something in a release build:
/// `cargo test -p ecclesia-canvas --release -- --ignored lays_out_ten_thousand_pages`.
#[test]
#[ignore = "timing; run in release"]
fn lays_out_ten_thousand_pages_quickly() {
    let sizes: Vec<PageSize> = (0..10_000)
        .map(|i| PageSize {
            width: 612.0 + (i % 7) as f32,
            height: 792.0 - (i % 5) as f32,
        })
        .collect();
    for mode in [
        Mode::Single,
        Mode::Book,
        Mode::Horizontal,
        Mode::Grid { columns: 4 },
    ] {
        let spec = Spec {
            mode,
            ..Spec::default()
        };
        let start = std::time::Instant::now();
        let layout = std::hint::black_box(Layout::new(&sizes, spec));
        let took = start.elapsed();
        println!("{mode:?}: {} pages in {took:?}", layout.len());
        assert!(took.as_secs_f64() < 0.005, "{mode:?} took {took:?}");
    }
}
