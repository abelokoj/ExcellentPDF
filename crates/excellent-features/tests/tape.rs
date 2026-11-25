//! H.4's "Done when", on a real PDF through the operation log: tape toggles with one tap and survives save.

use std::path::{Path, PathBuf};

use excellent_core::annot::{Annotation, page_annotations};
use excellent_core::document::Document;
use excellent_core::edit::{Change, Operation};
use excellent_features::annotate::tape;
use excellent_mupdf::PageRect;

/// A copy of a corpus PDF in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn find(doc: &Document, id: &str) -> Annotation {
    let shared = doc.mupdf().expect("the document");
    let mupdf = shared.lock().expect("document lock poisoned");
    page_annotations(&mupdf, 0)
        .expect("read")
        .into_iter()
        .find(|a| a.id == id)
        .expect("the tape")
}

#[test]
fn tape_toggles_with_one_tap_and_survives_save() {
    let from =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/base14-outline-annot.pdf");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("excellent-tape-{}.pdf", std::process::id())));
    std::fs::copy(from, &scratch.0).expect("copy");
    let doc = Document::open(&scratch.0).expect("open");
    let strip = tape::tape(
        0,
        PageRect {
            x0: 72.0,
            y0: 100.0,
            x1: 300.0,
            y1: 116.0,
        },
        "Ada",
    );
    let id = strip.id.clone();
    doc.apply(&Operation::new(
        "Tape",
        vec![Change::CreateAnnotation(strip)],
    ))
    .expect("laid");
    assert!(
        tape::is_hiding(&find(&doc, &id)),
        "it hides what is under it"
    );

    let tap = |doc: &Document| {
        let change = tape::tapped(&find(doc, &id)).expect("tape answers a tap");
        doc.apply(&Operation::new("Tap", vec![change]))
            .expect("tapped");
    };
    tap(&doc);
    assert!(!tape::is_hiding(&find(&doc, &id)), "one tap reveals it");

    doc.save().expect("save");
    let reopened = excellent_mupdf::Document::open_pdf(&scratch.0).expect("reopen");
    let saved = page_annotations(&reopened, 0)
        .expect("read")
        .into_iter()
        .find(|a| a.id == id)
        .expect("saved");
    assert!(saved.tape, "still tape in the file");
    assert!(!tape::is_hiding(&saved), "still revealed");

    tap(&doc);
    assert!(
        tape::is_hiding(&find(&doc, &id)),
        "another tap hides it again"
    );
}
