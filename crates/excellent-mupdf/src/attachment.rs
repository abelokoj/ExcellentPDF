//! A PDF's attachments (O.25): its own embedded files (`/EmbeddedFiles`) and its pages' file attachment
//! annotations, read here; a file is embedded by [`crate::Edit::add_attachment`]. Names are the file's: untrusted, so
//! whoever saves one makes the name safe first.

use core::ffi::c_int;
use core::ptr;

use excellent_mupdf_sys::{ex_attachment_contents, ex_attachment_count, ex_attachment_info};

use crate::context::{check, with_context};
use crate::document::Document;
use crate::error::{MuError, Result};

/// One attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Its place among the document's attachments, which [`Document::attachment_contents`] takes.
    pub index: usize,
    /// Its file name as the PDF gives it (untrusted).
    pub name: String,
    /// Its size in bytes; `None` when the file doesn't say.
    pub size: Option<u64>,
    /// The page whose annotation holds it; `None` for one of the document's own.
    pub page: Option<usize>,
}

impl Document {
    /// Its attachments: its own embedded files in name order, then each page's file attachment annotations; none for
    /// a document that isn't a PDF. Reads every page's annotations: run it on a worker for a long document.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the document.
    pub fn attachments(&self) -> Result<Vec<Attachment>> {
        with_context(|ctx| {
            let mut count: c_int = 0;
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_attachment_count(ctx, self.raw().as_ptr(), &mut count, err) })?;
            (0..count)
                .map(|index| {
                    let (mut name, mut size, mut page) = (ptr::null_mut(), 0, 0);
                    // SAFETY: as above; `name` receives a new buffer.
                    check(|err| unsafe {
                        ex_attachment_info(
                            ctx,
                            self.raw().as_ptr(),
                            index,
                            &mut name,
                            &mut size,
                            &mut page,
                            err,
                        )
                    })?;
                    Ok(Attachment {
                        index: usize::try_from(index).unwrap_or_default(),
                        // SAFETY: `name` is a new buffer we own.
                        name: unsafe { crate::buffer::into_string(ctx, name) },
                        size: u64::try_from(size).ok(),
                        page: usize::try_from(page).ok(),
                    })
                })
                .collect()
        })
    }

    /// The bytes of attachment `index` ([`Attachment::index`]).
    ///
    /// # Errors
    ///
    /// There is no such attachment, it names a file outside the PDF, or MuPDF can't read it.
    pub fn attachment_contents(&self, index: usize) -> Result<Vec<u8>> {
        let index = c_int::try_from(index).map_err(|_| MuError::argument("no such attachment"))?;
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the document is live and used by this thread only; `buf` receives a new buffer.
            check(|err| unsafe {
                ex_attachment_contents(ctx, self.raw().as_ptr(), index, &mut buf, err)
            })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { crate::buffer::into_vec(ctx, buf) })
        })
    }
}
