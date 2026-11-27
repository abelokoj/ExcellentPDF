//! Writing pages to files (PDF, images, CBZ, SVG, text, ...).

use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::path::Path;

use ecclesia_mupdf_sys::{
    ex_close_document_writer, ex_drop_document_writer, ex_new_document_writer, ex_write_page,
    fz_document_writer,
};

use crate::context::{check, drop_with_context, with_context};
use crate::error::{MuError, Result};
use crate::page::Page;

/// Writes pages, each at its own size, into one output file. Call [`Writer::close`] to finish the file; dropping an
/// unclosed writer abandons it. Not `Send`.
#[derive(Debug)]
pub struct Writer {
    raw: NonNull<fz_document_writer>,
}

fn c_string(value: &str, what: &str) -> Result<CString> {
    CString::new(value)
        .map_err(|_| MuError::argument(&format!("the {what} contains a NUL character")))
}

impl Writer {
    /// Starts writing to `path`. `format` is one of MuPDF's output formats (`"pdf"`, `"png"`, `"cbz"`, `"svg"`,
    /// `"text"`, ...), or `None` to choose by the file extension; `options` are MuPDF's writer options, such as
    /// `"compress"` for PDF.
    ///
    /// # Errors
    ///
    /// The arguments aren't valid strings, the format is unknown, or the file can't be created.
    pub fn new(
        path: impl AsRef<Path>,
        format: Option<&str>,
        options: Option<&str>,
    ) -> Result<Self> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| MuError::argument("the path is not valid UTF-8"))?;
        let path = c_string(path, "path")?;
        let format = format.map(|f| c_string(f, "format")).transpose()?;
        let options = options.map(|o| c_string(o, "options")).transpose()?;
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: every string is NUL-terminated, or NULL where the shim allows it.
            check(|err| unsafe {
                ex_new_document_writer(
                    ctx,
                    path.as_ptr(),
                    format.as_ref().map_or(ptr::null(), |f| f.as_ptr()),
                    options.as_ref().map_or(ptr::null(), |o| o.as_ptr()),
                    &mut raw,
                    err,
                )
            })
        })?;
        Ok(Self {
            raw: NonNull::new(raw).expect("ex_new_document_writer succeeded with a writer"),
        })
    }

    /// Appends `page` to the output.
    ///
    /// # Errors
    ///
    /// MuPDF fails to run the page or to write it.
    pub fn write_page(&mut self, page: &Page<'_>) -> Result<()> {
        with_context(|ctx| {
            // SAFETY: the writer and the page are live and on this thread.
            check(|err| unsafe { ex_write_page(ctx, self.raw.as_ptr(), page.raw().as_ptr(), err) })
        })
    }

    /// Finishes the file.
    ///
    /// # Errors
    ///
    /// MuPDF fails to write the end of the file.
    pub fn close(self) -> Result<()> {
        with_context(|ctx| {
            // SAFETY: the writer is live; `Drop` releases it after this.
            check(|err| unsafe { ex_close_document_writer(ctx, self.raw.as_ptr(), err) })
        })
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: we own the writer.
        drop_with_context(|ctx| unsafe { ex_drop_document_writer(ctx, self.raw.as_ptr()) });
    }
}
