//! HTML laid out by MuPDF's story engine into a one-page PDF (A.20's dynamic stamps).

use core::ptr;
use std::ffi::CString;

use excellent_mupdf_sys::ex_html_to_pdf;

use crate::buffer;
use crate::context::{check, with_context};
use crate::error::{MuError, Result};

/// `html`, styled by `css`, laid out `width` points wide, as a one-page PDF just as tall as the text.
///
/// # Errors
///
/// The HTML or the CSS holds a NUL, or MuPDF can't lay it out or write the PDF.
pub fn html_to_pdf(html: &str, css: &str, width: f32) -> Result<Vec<u8>> {
    let html = CString::new(html).map_err(|_| MuError::argument("the HTML contains a NUL"))?;
    let css = CString::new(css).map_err(|_| MuError::argument("the CSS contains a NUL"))?;
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: both strings are NUL-terminated and outlive the call; `buf` receives a new buffer.
        check(|err| unsafe {
            ex_html_to_pdf(ctx, html.as_ptr(), css.as_ptr(), width, &mut buf, err)
        })?;
        // SAFETY: `buf` is a new buffer we own.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn html_becomes_a_page_as_tall_as_its_text() {
        let pdf = html_to_pdf(
            "<p>APPROVED</p><p>Ada, 2026-11-16</p>",
            "p { margin: 0 }",
            200.0,
        )
        .expect("pdf");
        let document = Document::from_bytes(&pdf, "application/pdf").expect("open");
        let bounds = document
            .load_page(0)
            .expect("page")
            .bounds()
            .expect("bounds");
        assert!((bounds.x1 - bounds.x0 - 200.0).abs() < 0.5, "{bounds:?}");
        let height = bounds.y1 - bounds.y0;
        assert!(height > 12.0 && height < 100.0, "two lines tall: {height}");
    }
}
