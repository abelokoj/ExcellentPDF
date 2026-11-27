//! B.9 on a real file: an outline with an entry at a named destination the file lacks and one with no destination at
//! all has both found, and its working entry and web link aren't. (An entry past the last page can't be written to a
//! file MuPDF reads: it clamps a page object it can't find to the first page, and refuses a page number past the
//! last; the model's own test covers the `#page=N` link that leaves.)

use std::fs;

use ecclesia_features::outline::{self, Broken, Entry};
use ecclesia_mupdf::{Document, OutlineItem};

fn entry(item: &OutlineItem) -> Entry {
    Entry {
        title: item.title.clone(),
        page: item.page,
        uri: item.uri.clone(),
        open: item.open,
        style: item.style,
        children: item.children.iter().map(entry).collect(),
    }
}

/// A two-page PDF whose outline has four entries: page 2 (works), a missing named destination, no destination, and a
/// web link. Written without an xref table; MuPDF rebuilds it.
const BROKEN_OUTLINE: &str = "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj
5 0 obj << /Type /Outlines /First 6 0 R /Last 10 0 R /Count 4 >> endobj
6 0 obj << /Title (Works) /Parent 5 0 R /Next 8 0 R /Dest [4 0 R /XYZ 0 200 0] >> endobj
8 0 obj << /Title (Named) /Parent 5 0 R /Prev 6 0 R /Next 9 0 R /Dest (nowhere) >> endobj
9 0 obj << /Title (Nothing) /Parent 5 0 R /Prev 8 0 R /Next 10 0 R >> endobj
10 0 obj << /Title (Web) /Parent 5 0 R /Prev 9 0 R /A << /S /URI /URI (https://example.org/) >> >> endobj
trailer << /Root 1 0 R >>
%%EOF
";

#[test]
fn every_broken_entry_in_a_test_file_is_found() {
    let dir = std::env::temp_dir().join(format!("ecclesia-b9-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch folder");
    let pdf = dir.join("broken-outline.pdf");
    fs::write(&pdf, BROKEN_OUTLINE).expect("write");
    let doc = Document::open(&pdf).expect("open");
    let pages = doc.page_count().expect("pages");
    let entries: Vec<Entry> = doc.outline().expect("outline").iter().map(entry).collect();
    drop(doc);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(pages, 2);
    assert_eq!(entries.len(), 4, "{entries:#?}");
    assert_eq!(entries[0].page, Some(1), "{entries:#?}");
    assert_eq!(
        outline::check(&entries, pages),
        [
            ("1".to_owned(), Broken::Unresolved),
            ("2".to_owned(), Broken::NoTarget),
        ],
        "{entries:#?}"
    );
}
