//! O.25 on a real PDF through the operation log: a file embedded is listed with its name and size, reads back byte
//! for byte from the saved file, and one undo takes it out again.

use std::path::{Path, PathBuf};

use excellent_core::document::Document;
use excellent_core::edit::{Change, Operation};

/// A copy of a corpus PDF in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn names(doc: &Document) -> Vec<(String, Option<u64>, Option<usize>)> {
    let shared = doc.mupdf().expect("the document");
    let mupdf = shared.lock().expect("document lock poisoned");
    mupdf
        .attachments()
        .expect("listed")
        .into_iter()
        .map(|a| (a.name, a.size, a.page))
        .collect()
}

#[test]
fn an_embedded_file_is_listed_saved_and_undone() {
    let from =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/base14-outline-annot.pdf");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("excellent-attach-{}.pdf", std::process::id())));
    std::fs::copy(from, &scratch.0).expect("copy");
    let doc = Document::open(&scratch.0).expect("open");
    let before = names(&doc);

    let bytes = b"Quarter,Total\nQ1,12\nQ2,19\n".to_vec();
    for name in ["zeta.csv", "alpha data.csv"] {
        doc.apply(&Operation::new(
            "Attach",
            vec![Change::AddAttachment {
                name: name.into(),
                bytes: bytes.clone(),
            }],
        ))
        .expect("attach");
    }
    let listed = names(&doc);
    let ours: Vec<&String> = listed
        .iter()
        .filter(|(_, _, page)| page.is_none())
        .map(|(name, ..)| name)
        .collect();
    assert!(ours.contains(&&"alpha data.csv".to_owned()) && ours.contains(&&"zeta.csv".to_owned()));
    let alpha = ours
        .iter()
        .position(|n| *n == "alpha data.csv")
        .expect("listed");
    let zeta = ours.iter().position(|n| *n == "zeta.csv").expect("listed");
    assert!(
        alpha < zeta,
        "the document's own come in name order: {ours:?}"
    );

    doc.save().expect("save");
    let reopened = excellent_mupdf::Document::open_pdf(&scratch.0).expect("reopen");
    let found = reopened
        .attachments()
        .expect("listed")
        .into_iter()
        .find(|a| a.name == "zeta.csv")
        .expect("saved");
    assert_eq!(found.size, Some(bytes.len() as u64));
    assert_eq!(
        reopened.attachment_contents(found.index).expect("read"),
        bytes
    );

    assert!(doc.undo().expect("undo"));
    let after = names(&doc);
    assert!(
        !after.iter().any(|(name, ..)| name == "alpha data.csv"),
        "one undo takes the last one out"
    );
    assert_eq!(after.len(), before.len() + 1);
}
