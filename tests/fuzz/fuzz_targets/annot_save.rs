//! Fuzzes opening arbitrary PDF bytes under strict security limits, inspecting annotations, and
//! inspecting outline / page boxes without panicking (plan K.23).

#![no_main]

use ecclesia_core::annot::page_annotations;
use ecclesia_core::sandbox::{SecurityLimits, open_pdf_with_limits};
use ecclesia_mupdf::PageBox;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = SecurityLimits::strict();
    let Ok(doc) = open_pdf_with_limits(data, &limits) else {
        return;
    };
    let Ok(count) = doc.page_count() else {
        return;
    };
    let _ = doc.outline();
    let _ = doc.page_labels();
    for page in 0..count.min(4) {
        let _ = doc.page_box(page, PageBox::Crop);
        let _ = page_annotations(&doc, page);
    }
});
