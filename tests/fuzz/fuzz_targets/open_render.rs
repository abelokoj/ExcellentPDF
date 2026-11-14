//! Opens arbitrary bytes as a PDF, then renders and extracts the text of its first pages. Any crash, hang or
//! sanitizer report is a bug in MuPDF, the shim or the safe API.

#![no_main]

use excellent_mupdf::{Background, Document, Matrix};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = Document::from_bytes(data, "application/pdf") else {
        return;
    };
    let Ok(count) = doc.page_count() else {
        return;
    };
    for index in 0..count.min(3) {
        let Ok(page) = doc.load_page(index) else {
            continue;
        };
        // Small renders keep each run fast; a quarter of a huge page is still exercised in full.
        let _ = page.render(&Matrix::scale(0.25, 0.25), Background::White);
        let _ = page.text(None).and_then(|text| text.text());
        let _ = page.annots();
    }
    let _ = doc.outline();
});
