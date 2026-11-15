//! K.14, K.15, B.1–B.5: editing a PDF's outline inside operations, undo and redo, and saving over the open file.

use std::fs;
use std::path::{Path, PathBuf};

use excellent_mupdf::{Document, NewOutlineEntry, OutlineChange, OutlineItem, PagePoint, Save};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

/// A copy of a corpus file in a folder of its own, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str, from: &str) -> (Self, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("excellent-mupdf-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch folder");
        let file = dir.join(from);
        fs::copy(corpus(from), &file).expect("copy");
        (Self(dir), file)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn titles(items: &[OutlineItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            if item.children.is_empty() {
                item.title.clone()
            } else {
                format!("{}{:?}", item.title, titles(&item.children))
            }
        })
        .collect()
}

fn outline(doc: &Document) -> Vec<String> {
    titles(&doc.outline().expect("outline"))
}

fn uri(doc: &Document, page: usize) -> String {
    doc.link_uri(page, PagePoint { x: 0.0, y: 0.0 }, None)
        .expect("uri")
}

#[test]
fn outline_edits_are_undone_and_redone_one_operation_at_a_time() {
    let (_scratch, file) = Scratch::new("undo", "base14-outline-annot.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    assert!(doc.is_pdf());
    assert_eq!(outline(&doc), ["Page one", "Page two"]);
    assert!(!doc.undo_state().expect("state").can_undo());

    let (uri, title) = (uri(&doc, 1), "Added");
    doc.operation("Add outline entry", |edit| {
        edit.insert_outline_entry(&[2], NewOutlineEntry { title, uri: &uri })
    })
    .expect("insert");
    let added = doc.outline().expect("outline");
    assert_eq!(titles(&added), ["Page one", "Page two", "Added"]);
    assert_eq!(added[2].page, Some(1), "{:?}", added[2]);

    doc.operation("Rename", |edit| {
        edit.update_outline_entry(
            &[0],
            OutlineChange {
                title: Some("First"),
                uri: None,
                open: false,
            },
        )
    })
    .expect("rename");
    let renamed = doc.outline().expect("outline");
    assert_eq!(titles(&renamed), ["First", "Page two", "Added"]);
    assert_eq!(renamed[0].page, Some(0), "a rename keeps the destination");

    // B.3: under "Page two", which is first once "First" has left.
    doc.operation("Move", |edit| edit.move_outline_entry(&[0], &[0, 0]))
        .expect("move");
    assert_eq!(outline(&doc), [r#"Page two["First"]"#, "Added"]);

    doc.operation("Delete", |edit| edit.delete_outline_entry(&[0]))
        .expect("delete");
    assert_eq!(
        outline(&doc),
        ["Added"],
        "a delete takes the entries under it"
    );

    let state = doc.undo_state().expect("state");
    assert_eq!((state.position, state.steps), (4, 4));
    doc.undo().expect("undo delete");
    assert_eq!(outline(&doc), [r#"Page two["First"]"#, "Added"]);
    doc.undo().expect("undo move");
    assert_eq!(outline(&doc), ["First", "Page two", "Added"]);
    doc.undo().expect("undo rename");
    doc.undo().expect("undo insert");
    assert_eq!(outline(&doc), ["Page one", "Page two"], "back as it was");
    assert!(doc.undo().is_err(), "nothing more to undo");
    doc.redo().expect("redo insert");
    assert_eq!(outline(&doc), ["Page one", "Page two", "Added"]);
}

#[test]
fn a_failed_operation_changes_nothing() {
    let (_scratch, file) = Scratch::new("failed", "base14-outline-annot.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    let (uri, title) = (uri(&doc, 0), "Lost");
    let failed = doc.operation("Two edits", |edit| {
        edit.insert_outline_entry(&[0], NewOutlineEntry { title, uri: &uri })?;
        // Past the end of the outline: fails, and takes the insert with it.
        edit.delete_outline_entry(&[7])
    });
    assert!(failed.is_err());
    assert_eq!(outline(&doc), ["Page one", "Page two"]);
    assert!(!doc.undo_state().expect("state").can_undo());
}

/// B.4: an entry pointed elsewhere keeps its title, opens the new page after a save and reopen, and undoes.
#[test]
fn a_repointed_entry_opens_its_new_place_after_reopening() {
    let (_scratch, file) = Scratch::new("repoint", "base14-outline-annot.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    let to = uri(&doc, 1);
    let repoint = OutlineChange {
        title: None,
        uri: Some(&to),
        open: false,
    };
    doc.operation("Repoint", |edit| edit.update_outline_entry(&[0], repoint))
        .expect("repoint");
    let entry = &doc.outline().expect("outline")[0];
    assert_eq!((entry.title.as_str(), entry.page), ("Page one", Some(1)));
    doc.undo().expect("undo");
    assert_eq!(doc.outline().expect("outline")[0].page, Some(0));
    doc.redo().expect("redo");
    doc.save_pdf(&file).expect("save");
    drop(doc);
    let reopened = Document::open_pdf(&file).expect("reopen");
    assert_eq!(reopened.outline().expect("outline")[0].page, Some(1));
}

#[test]
fn the_first_entry_makes_an_outline() {
    let (_scratch, file) = Scratch::new("first", "heavy-vector.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    assert!(doc.outline().expect("outline").is_empty());
    let (uri, title) = (uri(&doc, 0), "Start");
    doc.operation("Add", |edit| {
        edit.insert_outline_entry(&[0], NewOutlineEntry { title, uri: &uri })
    })
    .expect("insert");
    assert_eq!(outline(&doc), ["Start"]);
}

#[test]
fn saves_replace_the_open_file_and_keep_undo() {
    let (scratch, file) = Scratch::new("save", "base14-outline-annot.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    let (uri, title) = (uri(&doc, 1), "Saved");
    doc.operation("Add", |edit| {
        edit.insert_outline_entry(&[2], NewOutlineEntry { title, uri: &uri })
    })
    .expect("insert");

    // K.15: a new file beside the open one, moved over it while the document still reads the old bytes.
    let temp = scratch.0.join("save.tmp");
    assert_eq!(doc.save_pdf(&temp).expect("save"), Save::Incremental);
    fs::rename(&temp, &file).expect("replace the open file");
    assert_eq!(
        outline(&Document::open(&file).expect("reopen")),
        ["Page one", "Page two", "Saved"]
    );

    // The edits and their undo stay; the next save writes every edit since the document was opened.
    doc.operation("Delete", |edit| edit.delete_outline_entry(&[0]))
        .expect("delete");
    doc.save_pdf(&temp).expect("save again");
    fs::rename(&temp, &file).expect("replace again");
    assert_eq!(
        outline(&Document::open(&file).expect("reopen")),
        ["Page two", "Saved"]
    );
    doc.undo().expect("undo after saving");
    assert_eq!(outline(&doc), ["Page one", "Page two", "Saved"]);
    assert_eq!(doc.page_count().expect("pages"), 2);
}

#[test]
fn only_a_pdf_is_edited() {
    let mut doc = Document::open(corpus("formats.epub")).expect("open");
    assert!(!doc.is_pdf());
    assert!(doc.operation("Nothing", |_| Ok(())).is_err());
    assert!(!doc.undo_state().expect("state").can_undo());
}
