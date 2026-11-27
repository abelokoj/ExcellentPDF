//! Open documents.

use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::path::Path;

use ecclesia_mupdf_sys::{
    ex_count_pages, ex_drop_document, ex_has_page_labels, ex_is_reflowable, ex_load_page,
    ex_open_document, ex_open_document_from_bytes, ex_open_pdf, ex_page_box, ex_page_label,
    ex_reads_right_to_left, ex_reflow, fz_document, fz_rect,
};

use crate::context::{check, drop_with_context, with_context};
use crate::error::{MuError, Result};
use crate::geometry::{PageBox, PageRect};
use crate::outline::{self, OutlineItem};
use crate::page::Page;

/// The pages a reflowable document's text is laid out on (R.28): their size and the text's size, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reflow {
    /// The page's width.
    pub width: f32,
    /// The page's height.
    pub height: f32,
    /// The body text's size (the em).
    pub font_size: f32,
}

/// An open document of any format MuPDF reads (PDF, XPS, EPUB, CBZ, images, OFD, ...).
///
/// `Send` but not `Sync`: use it from one thread at a time, behind a `Mutex` when shared. Its pages borrow it, so a
/// document can't move to another thread while one of its pages is loaded.
#[derive(Debug)]
pub struct Document {
    raw: NonNull<fz_document>,
}

// SAFETY: an fz_document may be used from any thread, one at a time, with that thread's context of the same family.
// `Document` is not `Sync`, and the `Page`s that use it borrow it and are not `Send`, so only one thread uses it at
// once.
unsafe impl Send for Document {}

impl Document {
    /// Opens the document at `path`.
    ///
    /// # Errors
    ///
    /// The path isn't valid UTF-8 or contains a NUL, or MuPDF can't open the file (missing, unreadable, damaged, or
    /// a format it doesn't read).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = c_path(path.as_ref())?;
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: `ctx` is this thread's context; `path` is NUL-terminated UTF-8, as the shim expects.
            check(|err| unsafe { ex_open_document(ctx, path.as_ptr(), &mut raw, err) })
        })?;
        Ok(Self {
            raw: NonNull::new(raw).expect("ex_open_document succeeded with a document"),
        })
    }

    /// Opens the PDF at `path` to read and edit (K.15). The file may be replaced while it is open, as a save does,
    /// and the document goes on reading the bytes it opened.
    ///
    /// # Errors
    ///
    /// The path isn't valid UTF-8 or contains a NUL, or MuPDF can't open the file as a PDF.
    pub fn open_pdf(path: impl AsRef<Path>) -> Result<Self> {
        let path = c_path(path.as_ref())?;
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: `ctx` is this thread's context; `path` is NUL-terminated UTF-8, as the shim expects.
            check(|err| unsafe { ex_open_pdf(ctx, path.as_ptr(), &mut raw, err) })
        })?;
        Ok(Self {
            raw: NonNull::new(raw).expect("ex_open_pdf succeeded with a document"),
        })
    }

    /// The MuPDF document, for this crate's other modules.
    /// Takes ownership of a document the shim made.
    pub(crate) fn from_raw(raw: NonNull<fz_document>) -> Self {
        Self { raw }
    }

    pub(crate) fn raw(&self) -> NonNull<fz_document> {
        self.raw
    }

    /// Opens a document held in memory; the bytes are copied. `magic` names the format: a file name such as
    /// `"book.epub"`, or a MIME type such as `"application/pdf"`.
    ///
    /// # Errors
    ///
    /// `magic` contains a NUL, or MuPDF can't open the bytes (damaged, or a format it doesn't read).
    pub fn from_bytes(bytes: &[u8], magic: &str) -> Result<Self> {
        let magic = CString::new(magic)
            .map_err(|_| MuError::argument("the format name contains a NUL character"))?;
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: `bytes` is valid for its length and copied by the shim; `magic` is NUL-terminated.
            check(|err| unsafe {
                ex_open_document_from_bytes(
                    ctx,
                    magic.as_ptr(),
                    bytes.as_ptr(),
                    bytes.len(),
                    &mut raw,
                    err,
                )
            })
        })?;
        Ok(Self {
            raw: NonNull::new(raw).expect("ex_open_document_from_bytes succeeded with a document"),
        })
    }

    /// The number of pages.
    ///
    /// # Errors
    ///
    /// MuPDF can't lay out or parse the document far enough to count them.
    pub fn page_count(&self) -> Result<usize> {
        let mut count = 0;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only (it is not `Sync`).
            check(|err| unsafe { ex_count_pages(ctx, self.raw.as_ptr(), &mut count, err) })
        })?;
        Ok(usize::try_from(count).expect("MuPDF never reports a negative page count"))
    }

    /// Page `index`'s `page_box` in points, with the page's own rotation applied: for the crop box, what
    /// [`Page::bounds`] reports. A PDF page is read from the page tree without being loaded, so laying out a long
    /// document is quick; other formats load the page.
    ///
    /// # Errors
    ///
    /// The index is past the last page, or MuPDF can't read the page.
    pub fn page_box(&self, index: usize, page_box: PageBox) -> Result<PageRect> {
        let number =
            i32::try_from(index).map_err(|_| MuError::argument("page index out of range"))?;
        let mut rect = fz_rect::default();
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe {
                ex_page_box(
                    ctx,
                    self.raw.as_ptr(),
                    number,
                    page_box.raw(),
                    &mut rect,
                    err,
                )
            })
        })?;
        Ok(rect.into())
    }

    /// Whether the document lays its text out to a page size of the reader's choosing (EPUB, FB2, MOBI, HTML,
    /// Markdown): see [`Document::reflow`].
    ///
    /// # Errors
    ///
    /// MuPDF can't read the document.
    pub fn is_reflowable(&self) -> Result<bool> {
        let mut reflowable = 0;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_is_reflowable(ctx, self.raw.as_ptr(), &mut reflowable, err) })
        })?;
        Ok(reflowable != 0)
    }

    /// Lays a reflowable document out on `reflow`'s pages. With `keep`, returns the page that now starts with the
    /// text page `keep` started with (or the page holding it), so the reader stays where they were. A document that
    /// isn't reflowable keeps its pages. `&mut self`: no page of the document may be loaded meanwhile, as the layout
    /// replaces them.
    ///
    /// # Errors
    ///
    /// A size isn't positive, `keep` is out of range, or MuPDF can't lay the document out.
    pub fn reflow(&mut self, reflow: Reflow, keep: Option<usize>) -> Result<Option<usize>> {
        let Reflow {
            width,
            height,
            font_size,
        } = reflow;
        if !(width > 0.0 && height > 0.0 && font_size > 0.0) {
            return Err(MuError::argument("a reflow size isn't positive"));
        }
        let keep = match keep {
            Some(page) => {
                i32::try_from(page).map_err(|_| MuError::argument("page index out of range"))?
            }
            None => -1,
        };
        let mut page = -1;
        with_context(|ctx| {
            // SAFETY: the document is live, used by this thread only, and has no page loaded (`&mut self`).
            check(|err| unsafe {
                ex_reflow(
                    ctx,
                    self.raw.as_ptr(),
                    width,
                    height,
                    font_size,
                    keep,
                    &mut page,
                    err,
                )
            })
        })?;
        Ok(usize::try_from(page).ok())
    }

    /// Whether the document asks to be read right to left (a PDF's `/ViewerPreferences /Direction /R2L`).
    ///
    /// # Errors
    ///
    /// MuPDF can't read the document's catalog.
    pub fn reads_right_to_left(&self) -> Result<bool> {
        let mut rtl = 0;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_reads_right_to_left(ctx, self.raw.as_ptr(), &mut rtl, err) })
        })?;
        Ok(rtl != 0)
    }

    /// Every page's label ("i", "ii", "1", "A-3"), read without loading the pages; `None` when the document has no
    /// labels of its own and the pages go by their numbers.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the labels.
    pub fn page_labels(&self) -> Result<Option<Vec<String>>> {
        let mut has = 0;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_has_page_labels(ctx, self.raw.as_ptr(), &mut has, err) })
        })?;
        if has == 0 {
            return Ok(None);
        }
        let count =
            i32::try_from(self.page_count()?).map_err(|_| MuError::argument("too many pages"))?;
        let mut buffer = [0 as core::ffi::c_char; 128];
        let size = buffer.len() as core::ffi::c_int;
        let labels = (0..count)
            .map(|number| {
                with_context(|ctx| {
                    // SAFETY: the document is live and used by this thread only; the shim writes at most `size`
                    // bytes, the last a NUL.
                    check(|err| unsafe {
                        ex_page_label(
                            ctx,
                            self.raw.as_ptr(),
                            number,
                            buffer.as_mut_ptr(),
                            size,
                            err,
                        )
                    })
                })?;
                // SAFETY: the shim always NUL-terminates within the buffer.
                let label = unsafe { core::ffi::CStr::from_ptr(buffer.as_ptr()) };
                Ok(label.to_string_lossy().into_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(labels))
    }

    /// Loads page `index`, counting from zero.
    ///
    /// # Errors
    ///
    /// The index is past the last page, or the page can't be loaded.
    pub fn load_page(&self, index: usize) -> Result<Page<'_>> {
        let number =
            i32::try_from(index).map_err(|_| MuError::argument("page index out of range"))?;
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_load_page(ctx, self.raw.as_ptr(), number, &mut raw, err) })
        })?;
        Ok(Page::new(
            NonNull::new(raw).expect("ex_load_page succeeded with a page"),
        ))
    }

    /// The table of contents; empty when the document has none.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the outline.
    pub fn outline(&self) -> Result<Vec<OutlineItem>> {
        outline::load(self.raw)
    }
}

/// `path` as the shim takes it: NUL-terminated UTF-8.
pub(crate) fn c_path(path: &Path) -> Result<CString> {
    let path = path
        .to_str()
        .ok_or_else(|| MuError::argument("the path is not valid UTF-8"))?;
    CString::new(path).map_err(|_| MuError::argument("the path contains a NUL character"))
}

impl Drop for Document {
    fn drop(&mut self) {
        // SAFETY: we own the document; its pages borrowed it and are gone.
        drop_with_context(|ctx| unsafe { ex_drop_document(ctx, self.raw.as_ptr()) });
    }
}
