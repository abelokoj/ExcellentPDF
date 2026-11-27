//! What A.24's annotation exchange needs from MuPDF: each page's map from PDF space, which XFDF's coordinates are in,
//! and FDF files written from and read into a PDF's annotations.

use core::ptr::{self, NonNull};

use ecclesia_mupdf_sys::{ex_fdf_read, ex_fdf_write, ex_page_transform, fz_matrix};

use crate::context::{check, with_context};
use crate::document::Document;
use crate::error::{MuError, Result};
use crate::geometry::Matrix;

impl Document {
    /// Page `index`'s map from PDF space (the file's own coordinates: points from the media box's corner, y up) to
    /// page space, as annotations are read here.
    ///
    /// # Errors
    ///
    /// The document isn't a PDF, the index is past the last page, or MuPDF can't read the page.
    pub fn page_transform(&self, index: usize) -> Result<Matrix> {
        let number =
            i32::try_from(index).map_err(|_| MuError::argument("page index out of range"))?;
        let mut raw = fz_matrix::default();
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; `raw` is written on success.
            check(|err| unsafe {
                ex_page_transform(ctx, self.raw().as_ptr(), number, &mut raw, err)
            })
        })?;
        Ok(Matrix {
            a: raw.a,
            b: raw.b,
            c: raw.c,
            d: raw.d,
            e: raw.e,
            f: raw.f,
        })
    }

    /// The PDF's annotations as the bytes of an FDF file, each with its page; links, popups and form fields stay
    /// out.
    ///
    /// # Errors
    ///
    /// The document isn't a PDF, or MuPDF can't read or write it.
    pub fn to_fdf(&self) -> Result<Vec<u8>> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the document is live and used by this thread only; `buf` receives a new buffer.
            check(|err| unsafe { ex_fdf_write(ctx, self.raw().as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { crate::buffer::into_vec(ctx, buf) })
        })
    }

    /// A blank PDF shaped like this one (its page count and boxes), holding the annotations of `fdf`, an FDF file
    /// opened with [`Document::from_bytes`]: read its pages to have them as this document's would read.
    ///
    /// # Errors
    ///
    /// This document isn't a PDF, `fdf` isn't an FDF file, or MuPDF can't read them.
    pub fn with_fdf_annotations(&self, fdf: &Self) -> Result<Self> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: both documents are live and used by this thread only; `raw` receives a new document we own.
            check(|err| unsafe {
                ex_fdf_read(ctx, self.raw().as_ptr(), fdf.raw().as_ptr(), &mut raw, err)
            })
        })?;
        Ok(Self::from_raw(
            NonNull::new(raw).expect("ex_fdf_read succeeded with a document"),
        ))
    }
}
