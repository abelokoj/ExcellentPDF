//! Fuzzes central parser security resource limits (plan K.23) across PDF, XFDF, ZIP/expdf,
//! SyncTeX, attachment filename sanitization, and external URI classification.

#![no_main]

use ecclesia_core::sandbox::{
    SecurityLimits, classify_external_uri, open_pdf_with_limits, sanitize_attachment_name,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = SecurityLimits::strict();
    let _ = limits.validate_pdf_bytes(data);
    let _ = limits.validate_xfdf_bytes(data);
    let _ = limits.validate_zip_archive_bytes(data);
    let _ = limits.validate_synctex_bytes(data);
    let _ = open_pdf_with_limits(data, &limits);
    if let Ok(as_str) = std::str::from_utf8(data) {
        let _ = sanitize_attachment_name(as_str);
        let _ = classify_external_uri(as_str);
    }
});
