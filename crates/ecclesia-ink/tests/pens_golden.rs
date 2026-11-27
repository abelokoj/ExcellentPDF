//! H.2: each pen draws the recorded session as its golden image does (`tests/golden/<pen>.pgm`). After a deliberate
//! change to a pen, write the images again with `ECCLESIAPDF_UPDATE_GOLDEN=1 cargo test -p ecclesia-ink` and
//! look at them before committing.

use std::path::{Path, PathBuf};

use ecclesia_ink::pens::PenKind;
use ecclesia_ink::raster::{Frame, Raster};
use ecclesia_ink::session::Session;

/// The session every pen draws.
const SESSION: &str = "synthetic-e-line-circle.session";
/// The part of the page it covers, at one pixel a point.
const FRAME: Frame = Frame {
    left: 60.0,
    top: 150.0,
    scale: 1.0,
};
const WIDTH: usize = 240;
const HEIGHT: usize = 100;
/// The pen's size, in points.
const SIZE: f32 = 3.0;
/// Rounding in `sin`, `powf` and the like differs a little between platforms: a pixel may move a few levels.
const LEVELS: u8 = 6;

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

#[test]
fn each_pen_matches_its_golden_image() {
    let text =
        std::fs::read_to_string(tests_dir().join("sessions").join(SESSION)).expect("the session");
    let session = Session::parse(&text).expect("parses");
    let update = std::env::var_os("ECCLESIAPDF_UPDATE_GOLDEN").is_some();
    let mut images = Vec::new();
    for kind in PenKind::ALL {
        let pen = kind.pen(SIZE);
        let mut drawn = Raster {
            width: WIDTH,
            height: HEIGHT,
            pixels: vec![0; WIDTH * HEIGHT],
        };
        for samples in &session.strokes {
            let points = pen.finish(&pen.stroke(samples), 0.1);
            let stroke = Raster::draw(&points, &pen, WIDTH, HEIGHT, FRAME);
            for (pixel, &new) in drawn.pixels.iter_mut().zip(&stroke.pixels) {
                *pixel = (*pixel).max(new);
            }
        }
        assert!(
            drawn.pixels.iter().any(|&p| p > 0),
            "{kind} draws something"
        );
        images.push((kind, drawn));
    }
    // No two pens look alike.
    for (i, (a, first)) in images.iter().enumerate() {
        for (b, second) in &images[i + 1..] {
            assert!(
                first.differing(second, LEVELS) > 100,
                "{a} and {b} look alike"
            );
        }
    }
    for (kind, drawn) in images {
        let path = tests_dir().join("golden").join(format!("{kind}.pgm"));
        if update {
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("make the folder");
            std::fs::write(&path, drawn.to_pgm()).expect("write the golden image");
            continue;
        }
        let golden = std::fs::read(&path)
            .ok()
            .and_then(|bytes| Raster::from_pgm(&bytes))
            .unwrap_or_else(|| panic!("no golden image at {}", path.display()));
        let differing = drawn.differing(&golden, LEVELS);
        assert!(
            differing * 1000 <= drawn.pixels.len(),
            "{kind}: {differing} pixels differ from {}",
            path.display()
        );
    }
}
