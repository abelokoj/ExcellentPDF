//! H.5's "Done when", on a real PDF through the operation log: area-erasing through a stroke leaves two correct
//! strokes, saved as such, and one undo brings the whole stroke back.

use std::path::{Path, PathBuf};

use excellent_core::annot::{Annotation, Shape, Style, page_annotations};
use excellent_core::document::Document;
use excellent_core::edit::{Change, Operation};
use excellent_features::annotate::erase::Erased;
use excellent_mupdf::PagePoint;
use serde_json::json;

/// A copy of a corpus PDF in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn copy() -> Self {
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/base14-outline-annot.pdf");
        let to = std::env::temp_dir().join(format!("excellent-erase-{}.pdf", std::process::id()));
        std::fs::copy(from, &to).expect("copy the corpus file");
        Self(to)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn annotations(doc: &Document) -> Vec<Annotation> {
    let shared = doc.mupdf().expect("the document");
    let mupdf = shared.lock().expect("document lock poisoned");
    page_annotations(&mupdf, 0).expect("read the page")
}

fn strokes(found: &[Annotation], id: &str) -> Option<Vec<Vec<PagePoint>>> {
    match &found.iter().find(|a| a.id == id)?.shape {
        Shape::Ink(strokes) => Some(strokes.clone()),
        other => panic!("not ink: {other:?}"),
    }
}

#[test]
fn area_erasing_through_a_stroke_leaves_two_strokes_undone_in_one_step() {
    let scratch = Scratch::copy();
    let doc = Document::open(&scratch.0).expect("open");
    let stroke = vec![
        PagePoint { x: 100.0, y: 300.0 },
        PagePoint { x: 300.0, y: 300.0 },
    ];
    let line = Annotation::new(
        0,
        Shape::Ink(vec![stroke.clone()]),
        Style {
            color: Some([0.1, 0.2, 0.6]),
            opacity: 1.0,
            width: 2.0,
        },
        "Ada",
    );
    let id = line.id.clone();
    doc.apply(&Operation::new("Ink", vec![Change::CreateAnnotation(line)]))
        .expect("draw");

    let erased = Erased::parse(&json!({
        "page": 0, "eraser": "area", "radius": 4.0, "points": [[200, 280], [200, 320]],
    }))
    .expect("parses");
    let changes = erased.changes(&annotations(&doc));
    assert_eq!(changes.len(), 1, "{changes:?}");
    doc.apply(&Operation::new("Erase", changes)).expect("erase");

    // Reach: the radius and half the stroke's width, 5 points either side of x = 200.
    let pieces = strokes(&annotations(&doc), &id).expect("the annotation stays");
    assert_eq!(pieces.len(), 2, "two strokes: {pieces:?}");
    let near = |a: PagePoint, x: f32| (a.x - x).abs() < 0.05 && (a.y - 300.0).abs() < 0.05;
    let ends = |piece: &[PagePoint]| (piece[0], piece[piece.len() - 1]);
    let (first, second) = (ends(&pieces[0]), ends(&pieces[1]));
    assert!(near(first.0, 100.0) && near(first.1, 195.0), "{pieces:?}");
    assert!(near(second.0, 205.0) && near(second.1, 300.0), "{pieces:?}");

    doc.save().expect("save");
    let reopened = excellent_mupdf::Document::open_pdf(&scratch.0).expect("reopen");
    let saved = page_annotations(&reopened, 0).expect("read the saved page");
    assert_eq!(
        strokes(&saved, &id).map(|s| s.len()),
        Some(2),
        "saved as two strokes"
    );

    assert!(doc.undo().expect("undo"), "there was a step to undo");
    assert_eq!(
        strokes(&annotations(&doc), &id),
        Some(vec![stroke]),
        "one undo gives the whole stroke back"
    );
}
