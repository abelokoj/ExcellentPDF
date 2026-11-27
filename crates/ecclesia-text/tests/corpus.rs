//! The text layer on real pages: copying a whole page gives MuPDF's own text.

use std::path::Path;

use ecclesia_mupdf::Document;
use ecclesia_text::PageText;

#[test]
fn copying_a_page_gives_the_same_words_as_mupdf() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    let doc = Document::open(path).expect("open");
    let page = doc.load_page(0).expect("page");
    let stext = page.text(None).expect("text");
    let text = PageText::new(stext.layout());
    let words = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    let copied = text.text(0..text.len());
    assert_eq!(words(&copied), words(&stext.text().expect("text")));
    assert!(copied.contains("This version of the library"), "{copied}");
}
