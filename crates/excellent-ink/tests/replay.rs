//! H.1: every recorded pen session in `tests/sessions` replays through the pipeline (plan §16): each stroke builds,
//! its widths stay within the pen's curve, and smoothing keeps its ends and stays near its samples.

use std::path::Path;

use excellent_ink::session::Session;
use excellent_ink::stroke::Stroke;
use excellent_ink::width::WidthCurve;

/// How far smoothing may pull a point from the samples, in page points: filtering moves the middle a little.
const NEAR: f32 = 2.0;

#[test]
fn recorded_sessions_replay() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sessions");
    let mut replayed = 0;
    for entry in std::fs::read_dir(&dir).expect("the sessions folder") {
        let path = entry.expect("a session").path();
        if path.extension().is_none_or(|ext| ext != "session") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        let session =
            Session::parse(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        assert!(
            !session.strokes.is_empty(),
            "{} has strokes",
            path.display()
        );
        let curve = WidthCurve {
            tilt: 0.5,
            ..WidthCurve::default()
        };
        let widest = curve.max * (1.0 + curve.tilt);
        for samples in &session.strokes {
            let stroke = Stroke::from_samples(samples, &curve);
            assert!(!stroke.is_empty());
            for point in stroke.points() {
                assert!(
                    point.width > 0.0 && point.width <= widest,
                    "width {}",
                    point.width
                );
            }
            let smooth = stroke.smoothed(0.3, 0.1);
            assert_eq!(smooth.first(), stroke.points().first());
            assert_eq!(smooth.last(), stroke.points().last());
            for point in &smooth {
                let nearest = samples
                    .iter()
                    .map(|s| (s.x - point.x).hypot(s.y - point.y))
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    nearest <= NEAR,
                    "({}, {}) is {nearest} from the samples",
                    point.x,
                    point.y
                );
            }
        }
        replayed += 1;
    }
    assert!(replayed > 0, "no sessions in {}", dir.display());
}
