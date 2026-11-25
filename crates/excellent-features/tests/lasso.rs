//! H.6's "Done when": a lasso move of 1,000 strokes stays at 60 fps. While the move is dragged the canvas only
//! offsets each stroke's outline; this times the work the app does around it: the loop selecting the 1,000
//! strokes, and the move committed as one operation. Each must fit one frame at 60 fps. Timings are asserted in
//! optimized builds only (`cargo test --release`); a debug build reports them.

use std::time::{Duration, Instant};

use excellent_core::annot::{Annotation, Shape, Style};
use excellent_features::annotate::edit::{self, Drag, Grip};
use excellent_features::annotate::lasso::Looped;
use excellent_mupdf::PagePoint;
use serde_json::json;

/// One frame at 60 fps.
const FRAME: Duration = Duration::from_micros(16_667);

/// Strokes, and points in each: a page of dense handwriting.
const STROKES: usize = 1_000;
const POINTS: usize = 40;

fn page_of_strokes() -> Vec<Annotation> {
    (0..STROKES)
        .map(|n| {
            let (x, y) = ((n % 25) as f32 * 20.0 + 20.0, (n / 25) as f32 * 18.0 + 20.0);
            let points = (0..POINTS)
                .map(|i| PagePoint {
                    x: x + i as f32 * 0.4,
                    y: y + (i as f32 * 0.7).sin() * 3.0,
                })
                .collect();
            Annotation::new(
                0,
                Shape::Ink(vec![points]),
                Style {
                    color: Some([0.1, 0.2, 0.6]),
                    opacity: 1.0,
                    width: 1.5,
                },
                "Ada",
            )
        })
        .collect()
}

fn report(what: &str, took: Duration) {
    eprintln!("{what}: {:.2} ms", took.as_secs_f64() * 1000.0);
    if !cfg!(debug_assertions) {
        assert!(took < FRAME, "{what} took {took:?}, more than a frame");
    }
}

#[test]
fn a_lasso_move_of_a_thousand_strokes_fits_a_frame() {
    let strokes = page_of_strokes();
    let looped = Looped::parse(&json!({
        "page": 0, "path": [[0, 0], [600, 0], [600, 800], [0, 800]], "add": false,
    }))
    .expect("parses");

    let started = Instant::now();
    let taken = looped.takes(&strokes);
    report("selecting 1,000 strokes", started.elapsed());
    assert_eq!(taken.len(), STROKES, "the loop takes every stroke");

    let drag = Drag {
        page: 0,
        id: strokes[0].id.clone(),
        grip: Grip::Move,
        from: PagePoint { x: 100.0, y: 100.0 },
        to: PagePoint { x: 130.0, y: 90.0 },
    };
    let started = Instant::now();
    let changes: Vec<_> = strokes
        .iter()
        .filter_map(|a| edit::change(a, drag.applied(a, true)?))
        .collect();
    report("moving 1,000 strokes", started.elapsed());
    assert_eq!(changes.len(), STROKES, "every stroke moves");
}
