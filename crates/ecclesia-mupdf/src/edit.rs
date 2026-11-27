//! Editing and saving a PDF (K.14, K.15): operations, undo and redo, outline edits (B.1–B.5), link destinations, and
//! writing the document with its edits.
//!
//! A change happens only through the [`Edit`] that [`Document::operation`] hands out, so every change is one step of
//! MuPDF's undo journal: it is kept when the step works and taken back when it fails.

use core::ffi::{CStr, c_char, c_int};
use core::ptr;
use std::ffi::CString;
use std::path::Path;

use ecclesia_mupdf_sys::{
    ex_abandon_operation, ex_annot_create, ex_annot_delete, ex_annot_set_stamp_art,
    ex_annot_update, ex_begin_operation, ex_compress_flags, ex_compress_stats, ex_end_operation,
    ex_is_pdf, ex_layer_set, ex_link_uri, ex_merge_source, ex_outline_delete, ex_outline_edit,
    ex_outline_insert, ex_outline_move, ex_outline_style, ex_outline_update, ex_page_box_edit,
    ex_page_label_range, ex_pdf_compress_bytes, ex_pdf_info_entries,
    ex_pdf_insert_blank_page_bytes, ex_pdf_merge_bytes, ex_pdf_read_page_labels,
    ex_pdf_rearrange_bytes, ex_pdf_redact_and_save, ex_pdf_rotate_pages_bytes,
    ex_pdf_set_page_boxes_bytes, ex_pdf_set_page_labels_bytes, ex_redact_flags, ex_redact_rect,
    ex_redo, ex_save_flattened, ex_save_pdf, ex_undo, ex_undo_state, fz_rect,
};

use crate::annot::AnnotData;
use crate::buffer;
use crate::context::{check, with_context};
use crate::document::{Document, c_path};
use crate::error::{MuError, Result};
use crate::geometry::{PagePoint, PageRect};
use crate::outline::OutlineStyle;

/// What a stamp is drawn as (A.20), besides the standard stamps MuPDF draws from their names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampArt {
    /// The first page of a PDF: a library stamp or a dynamic one. The stamp's name must not be a standard one, or
    /// MuPDF would draw that name over it.
    Page(Vec<u8>),
    /// An image file's bytes (PNG, JPEG and the other kinds MuPDF reads).
    Image(Vec<u8>),
}

/// Where a PDF is in its undo history: `position` of `steps` operations are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UndoState {
    /// The operations applied; 0 is the document as it was opened.
    pub position: usize,
    /// The operations in the history.
    pub steps: usize,
}

impl UndoState {
    /// Whether an operation can be undone.
    #[must_use]
    pub fn can_undo(self) -> bool {
        self.position > 0
    }

    /// Whether an undone operation can be done again.
    #[must_use]
    pub fn can_redo(self) -> bool {
        self.position < self.steps
    }
}

/// What a flattened copy does with the form fields (A.25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Forms {
    /// They stay fillable.
    #[default]
    Keep,
    /// They are drawn into the page with their values, like the annotations.
    Flatten,
}

/// How [`Document::save_pdf`] wrote the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Save {
    /// The opened file's bytes, unchanged, then the edits: the file as opened is the start of the new one.
    Incremental,
    /// The whole document written again (a file MuPDF repaired, for one, can't take an incremental update).
    Rewritten,
}

/// A new outline entry (B.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewOutlineEntry<'a> {
    /// Its title.
    pub title: &'a str,
    /// Where it goes, as [`Document::link_uri`] makes it.
    pub uri: &'a str,
}

/// A change to an outline entry (B.2, B.4, B.5); `None` keeps the entry's own. Its style (bold, italic, colour)
/// stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlineChange<'a> {
    /// A new title.
    pub title: Option<&'a str>,
    /// A new destination.
    pub uri: Option<&'a str>,
    /// Whether the entry shows the entries under it.
    pub open: bool,
}

/// The edits allowed inside one operation.
#[derive(Debug)]
pub struct Edit<'doc> {
    doc: &'doc mut Document,
}

impl Document {
    /// Whether this is a PDF, which alone can be edited and saved.
    #[must_use]
    pub fn is_pdf(&self) -> bool {
        // shortcut: a failed context gives "not a PDF", which only hides editing; there is nothing else to report.
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; the shim only reads its type.
            Ok(unsafe { ex_is_pdf(ctx, self.raw().as_ptr()) } != 0)
        })
        .unwrap_or(false)
    }

    /// Runs `edit` as one operation named `name` (shown in undo history): kept when it returns `Ok`, taken back when
    /// it returns `Err`.
    ///
    /// # Errors
    ///
    /// The document is not a PDF, the name contains a NUL, MuPDF can't start or end the operation, or `edit` fails.
    pub fn operation<T>(
        &mut self,
        name: &str,
        edit: impl FnOnce(&mut Edit<'_>) -> Result<T>,
    ) -> Result<T> {
        let name = CString::new(name)
            .map_err(|_| MuError::argument("the operation's name contains a NUL character"))?;
        let raw = self.raw();
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; `name` is NUL-terminated.
            check(|err| unsafe { ex_begin_operation(ctx, raw.as_ptr(), name.as_ptr(), err) })
        })?;
        let done = edit(&mut Edit { doc: self }).and_then(|value| {
            with_context(|ctx| {
                // SAFETY: as above; the operation begun above is still open.
                check(|err| unsafe { ex_end_operation(ctx, raw.as_ptr(), err) })
            })?;
            Ok(value)
        });
        if done.is_err() {
            // The operation is still open: take back what it changed. A failed context can't do that, and then the
            // document is left as the failed edit made it, which the error already reports.
            let _ = with_context(|ctx| {
                // SAFETY: as above.
                unsafe { ex_abandon_operation(ctx, raw.as_ptr()) };
                Ok(())
            });
        }
        done
    }

    /// Where the document is in its undo history; empty for a document that was never edited or is not a PDF.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the journal.
    pub fn undo_state(&self) -> Result<UndoState> {
        let (mut position, mut steps) = (0, 0);
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe {
                ex_undo_state(ctx, self.raw().as_ptr(), &mut position, &mut steps, err)
            })
        })?;
        Ok(UndoState {
            position: usize::try_from(position).unwrap_or(0),
            steps: usize::try_from(steps).unwrap_or(0),
        })
    }

    /// Takes back the last operation.
    ///
    /// # Errors
    ///
    /// There is nothing to undo, or the document is not a PDF.
    pub fn undo(&mut self) -> Result<()> {
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_undo(ctx, self.raw().as_ptr(), err) })
        })
    }

    /// Does the last undone operation again.
    ///
    /// # Errors
    ///
    /// There is nothing to redo, or the document is not a PDF.
    pub fn redo(&mut self) -> Result<()> {
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only.
            check(|err| unsafe { ex_redo(ctx, self.raw().as_ptr(), err) })
        })
    }

    /// The link to `point` on page `page` (counting from zero) at `zoom` (1 is 100 %; `None` keeps the reader's), as
    /// MuPDF names it: `#page=4&zoom=150,72,92`.
    ///
    /// # Errors
    ///
    /// The page number is out of range for MuPDF, or it can't make the link.
    pub fn link_uri(&self, page: usize, point: PagePoint, zoom: Option<f32>) -> Result<String> {
        let page = c_int::try_from(page).map_err(|_| MuError::argument("page out of range"))?;
        let mut buf = [0 as c_char; 256];
        let size = c_int::try_from(buf.len()).expect("the buffer is small");
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; `buf` holds `size` bytes, which the shim
            // NUL-terminates.
            check(|err| unsafe {
                ex_link_uri(
                    ctx,
                    self.raw().as_ptr(),
                    page,
                    point.x,
                    point.y,
                    zoom.unwrap_or(0.0),
                    buf.as_mut_ptr(),
                    size,
                    err,
                )
            })
        })?;
        // SAFETY: the shim NUL-terminated `buf`.
        let uri = unsafe { CStr::from_ptr(buf.as_ptr()) };
        Ok(uri.to_string_lossy().into_owned())
    }

    /// Writes a flattened copy of the PDF, its edits included, to `path` (A.25): its annotations drawn into the page
    /// content and taken out, so every reader shows them and none can change them; its form fields too when `forms`
    /// says so. The open document, its edits and their undo history are left as they are. `path` must not be the
    /// file it was opened from.
    ///
    /// # Errors
    ///
    /// The document is not a PDF, the path isn't valid, or MuPDF can't write the file.
    pub fn save_flattened(&self, path: &Path, forms: Forms) -> Result<()> {
        let path = c_path(path)?;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; `path` is NUL-terminated UTF-8.
            check(|err| unsafe {
                ex_save_flattened(
                    ctx,
                    self.raw().as_ptr(),
                    path.as_ptr(),
                    c_int::from(forms == Forms::Flatten),
                    err,
                )
            })
        })
    }

    /// Writes the PDF with its edits to `path`, which must not be the file it was opened from: the opened bytes and
    /// the edits as an incremental update when the file allows it, else the whole document rewritten. The edits stay
    /// in the document, with their undo history, and the next save writes them all again.
    ///
    /// # Errors
    ///
    /// The document is not a PDF, the path isn't valid, or MuPDF can't write the file.
    pub fn save_pdf(&mut self, path: &Path) -> Result<Save> {
        let path = c_path(path)?;
        let mut incremental = 0;
        with_context(|ctx| {
            // SAFETY: the document is live and used by this thread only; `path` is NUL-terminated UTF-8, and
            // `incremental` outlives the call.
            check(|err| unsafe {
                ex_save_pdf(
                    ctx,
                    self.raw().as_ptr(),
                    path.as_ptr(),
                    &mut incremental,
                    err,
                )
            })
        })?;
        Ok(if incremental != 0 {
            Save::Incremental
        } else {
            Save::Rewritten
        })
    }
}

/// `path` as the shim takes it.
fn c_ints(path: &[usize]) -> Result<(Vec<c_int>, c_int)> {
    let ints = path
        .iter()
        .map(|&index| c_int::try_from(index))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| MuError::argument("outline path out of range"))?;
    if ints.is_empty() {
        return Err(MuError::argument(
            "an outline path names at least one level",
        ));
    }
    let depth =
        c_int::try_from(ints.len()).map_err(|_| MuError::argument("outline path too deep"))?;
    Ok((ints, depth))
}

fn c_page(page: usize) -> Result<c_int> {
    c_int::try_from(page).map_err(|_| MuError::argument("page out of range"))
}

fn c_text(text: &str) -> Result<CString> {
    CString::new(text).map_err(|_| MuError::argument("the text contains a NUL character"))
}

impl Edit<'_> {
    /// The document being edited.
    #[must_use]
    pub fn document(&self) -> &Document {
        self.doc
    }

    /// Puts `entry` before the outline entry at `path` (its index on each level from the top), which may also be one
    /// past the last entry of its level. The first entry of a document without an outline makes one.
    ///
    /// # Errors
    ///
    /// The path isn't in the outline, or MuPDF can't write the entry.
    pub fn insert_outline_entry(
        &mut self,
        path: &[usize],
        entry: NewOutlineEntry<'_>,
    ) -> Result<()> {
        let (path, depth) = c_ints(path)?;
        let (title, uri) = (c_text(entry.title)?, c_text(entry.uri)?);
        let item = ex_outline_edit {
            title: title.as_ptr(),
            uri: uri.as_ptr(),
            ..ex_outline_edit::default()
        };
        with_context(|ctx| {
            // SAFETY: the document is live, used by this thread only, and inside an operation; `path` holds `depth`
            // ints; the strings outlive the call and the shim copies them.
            check(|err| unsafe {
                ex_outline_insert(
                    ctx,
                    self.doc.raw().as_ptr(),
                    path.as_ptr(),
                    depth,
                    &item,
                    err,
                )
            })
        })
    }

    /// Changes the outline entry at `path`.
    ///
    /// # Errors
    ///
    /// No entry is at the path, or MuPDF can't write it.
    pub fn update_outline_entry(
        &mut self,
        path: &[usize],
        change: OutlineChange<'_>,
    ) -> Result<()> {
        let (path, depth) = c_ints(path)?;
        let title = change.title.map(c_text).transpose()?;
        let uri = change.uri.map(c_text).transpose()?;
        let item = ex_outline_edit {
            title: title
                .as_ref()
                .map_or(core::ptr::null(), |title| title.as_ptr()),
            uri: uri.as_ref().map_or(core::ptr::null(), |uri| uri.as_ptr()),
            is_open: c_int::from(change.open),
            ..ex_outline_edit::default()
        };
        with_context(|ctx| {
            // SAFETY: as for `insert_outline_entry`; NULL strings keep the entry's own.
            check(|err| unsafe {
                ex_outline_update(
                    ctx,
                    self.doc.raw().as_ptr(),
                    path.as_ptr(),
                    depth,
                    &item,
                    err,
                )
            })
        })
    }

    /// Draws the outline entry at `path` with `style` (B.7); its title, destination and open state stay.
    ///
    /// # Errors
    ///
    /// No entry is at the path, or MuPDF can't write it.
    pub fn style_outline_entry(&mut self, path: &[usize], style: OutlineStyle) -> Result<()> {
        let (path, depth) = c_ints(path)?;
        let (flags, color) = style.to_raw();
        with_context(|ctx| {
            // SAFETY: the document is live, used by this thread only, and inside an operation; `path` holds `depth`
            // ints.
            check(|err| unsafe {
                ex_outline_style(
                    ctx,
                    self.doc.raw().as_ptr(),
                    path.as_ptr(),
                    depth,
                    flags,
                    color,
                    err,
                )
            })
        })
    }

    /// Deletes the outline entry at `path` and the entries under it.
    ///
    /// # Errors
    ///
    /// No entry is at the path, or MuPDF can't delete it.
    pub fn delete_outline_entry(&mut self, path: &[usize]) -> Result<()> {
        let (path, depth) = c_ints(path)?;
        with_context(|ctx| {
            // SAFETY: as for `insert_outline_entry`.
            check(|err| unsafe {
                ex_outline_delete(ctx, self.doc.raw().as_ptr(), path.as_ptr(), depth, err)
            })
        })
    }

    /// Adds `annot` to page `page` (counting from zero), named by its id, which must be new on the page.
    ///
    /// # Errors
    ///
    /// The page isn't in the document, the id is empty, the kind is unknown, or MuPDF can't write the annotation.
    pub fn create_annot(&mut self, page: usize, annot: &AnnotData) -> Result<()> {
        let page = c_page(page)?;
        annot.with_raw(|raw| {
            with_context(|ctx| {
                // SAFETY: the document is live, used by this thread only, and inside an operation; `raw` and what
                // it points at outlive the call, and the shim copies them.
                check(|err| unsafe {
                    ex_annot_create(ctx, self.doc.raw().as_ptr(), page, raw, err)
                })
            })
        })
    }

    /// Writes every property of `annot` to the annotation of page `page` with its id and kind.
    ///
    /// # Errors
    ///
    /// No annotation of that kind has the id on the page, or MuPDF can't write it.
    pub fn update_annot(&mut self, page: usize, annot: &AnnotData) -> Result<()> {
        let page = c_page(page)?;
        annot.with_raw(|raw| {
            with_context(|ctx| {
                // SAFETY: as for `create_annot`.
                check(|err| unsafe {
                    ex_annot_update(ctx, self.doc.raw().as_ptr(), page, raw, err)
                })
            })
        })
    }

    /// Draws the stamp of page `page` whose id is `id` as `art` (A.20).
    ///
    /// # Errors
    ///
    /// No stamp has the id on the page, the art isn't a PDF or an image MuPDF reads, or MuPDF can't write it.
    pub fn set_stamp_art(&mut self, page: usize, id: &str, art: &StampArt) -> Result<()> {
        let page = c_page(page)?;
        let id = c_text(id)?;
        let (kind, bytes) = match art {
            StampArt::Page(bytes) => (0, bytes),
            StampArt::Image(bytes) => (1, bytes),
        };
        with_context(|ctx| {
            // SAFETY: as for `create_annot`; `id` is NUL-terminated and `bytes` outlives the call, which copies it.
            check(|err| unsafe {
                ex_annot_set_stamp_art(
                    ctx,
                    self.doc.raw().as_ptr(),
                    page,
                    id.as_ptr(),
                    kind,
                    bytes.as_ptr(),
                    bytes.len(),
                    err,
                )
            })
        })
    }

    /// Embeds `bytes` as the file `name` among the document's own attachments (O.25).
    ///
    /// # Errors
    ///
    /// The document isn't a PDF, the name contains a NUL, or MuPDF can't write it.
    pub fn add_attachment(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let name = std::ffi::CString::new(name)
            .map_err(|_| MuError::argument("a file name contains a NUL"))?;
        with_context(|ctx| {
            // SAFETY: the document is live and in an operation; `name` is NUL-terminated and `bytes` outlives the
            // call, which copies it.
            check(|err| unsafe {
                ecclesia_mupdf_sys::ex_attachment_add(
                    ctx,
                    self.doc.raw().as_ptr(),
                    name.as_ptr(),
                    bytes.as_ptr(),
                    bytes.len(),
                    err,
                )
            })
        })
    }

    /// Shows or hides, locks or unlocks, the layer named `name` (A.22) in the file's default configuration, which
    /// other readers open with.
    ///
    /// # Errors
    ///
    /// The document has no layer of that name, the name contains a NUL, or MuPDF can't write it.
    pub fn set_layer(&mut self, name: &str, visible: bool, locked: bool) -> Result<()> {
        let name = c_text(name)?;
        with_context(|ctx| {
            // SAFETY: as for `create_annot`; `name` is NUL-terminated.
            check(|err| unsafe {
                ex_layer_set(
                    ctx,
                    self.doc.raw().as_ptr(),
                    name.as_ptr(),
                    c_int::from(visible),
                    c_int::from(locked),
                    err,
                )
            })
        })
    }

    /// Deletes the annotation of page `page` whose id is `id`.
    ///
    /// # Errors
    ///
    /// No annotation has the id on the page, or MuPDF can't delete it.
    pub fn delete_annot(&mut self, page: usize, id: &str) -> Result<()> {
        let page = c_page(page)?;
        let id = c_text(id)?;
        with_context(|ctx| {
            // SAFETY: as for `create_annot`; `id` is NUL-terminated.
            check(|err| unsafe {
                ex_annot_delete(ctx, self.doc.raw().as_ptr(), page, id.as_ptr(), err)
            })
        })
    }

    /// Moves the outline entry at `from`, with the entries under it, their styles and open state, to `to`: a path in
    /// the outline as it is once the entry has left (B.3).
    ///
    /// # Errors
    ///
    /// No entry is at `from`, `to` isn't in the outline, or MuPDF can't write the entries.
    pub fn move_outline_entry(&mut self, from: &[usize], to: &[usize]) -> Result<()> {
        let (from, from_depth) = c_ints(from)?;
        let (to, to_depth) = c_ints(to)?;
        with_context(|ctx| {
            // SAFETY: as for `insert_outline_entry`; both paths hold their depth's ints.
            check(|err| unsafe {
                ex_outline_move(
                    ctx,
                    self.doc.raw().as_ptr(),
                    from.as_ptr(),
                    from_depth,
                    to.as_ptr(),
                    to_depth,
                    err,
                )
            })
        })
    }
}

/// One redaction rectangle on a PDF page, in top-down page coordinates (as [`crate::TextLayout`] reports).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RedactionSpec {
    /// Zero-based page index.
    pub page: usize,
    /// Redaction rectangle in top-down page space (points).
    pub rect: PageRect,
    /// Solid overlay RGB fill color components in `0.0..=1.0`.
    pub fill_color: [f32; 3],
}

/// Policy flags for [`redact_pdf_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactionFlags {
    /// Draw opaque solid boxes over redacted areas.
    pub black_boxes: bool,
    /// Remove text overlapping redaction rectangles.
    pub redact_text: bool,
    /// Remove vector line art touching redaction rectangles.
    pub redact_vectors: bool,
    /// Blank image pixels overlapping redaction rectangles.
    pub redact_images: bool,
    /// Scrub `/Info`, XMP `/Metadata`, `/Thumb`, `/PieceInfo`, and `/StructTreeRoot`.
    pub scrub_metadata: bool,
    /// Scrub `/EmbeddedFiles`, `/AF`, and `FileAttachment` annotations.
    pub scrub_attachments: bool,
    /// Scrub `/OCProperties` and hidden-layer annotations.
    pub scrub_hidden_layers: bool,
    /// Remove non-widget annotations overlapping redaction rectangles.
    pub scrub_overlapping_annots: bool,
    /// Clear `/V`, `/DV`, and `/AP` on form widgets overlapping redaction rectangles.
    pub scrub_overlapping_forms: bool,
}

impl Default for RedactionFlags {
    fn default() -> Self {
        Self {
            black_boxes: true,
            redact_text: true,
            redact_vectors: true,
            redact_images: true,
            scrub_metadata: true,
            scrub_attachments: true,
            scrub_hidden_layers: true,
            scrub_overlapping_annots: true,
            scrub_overlapping_forms: true,
        }
    }
}

/// Permanently applies redactions to `input` PDF bytes through MuPDF's `pdf_redact_page`, scrubs
/// configured metadata/attachments/layers/annotations/forms, and writes a clean non-incremental,
/// garbage-collected PDF (plan D3, D4, O.7).
///
/// # Errors
///
/// `input` is not a valid PDF, a page index is out of range, or MuPDF fails while redacting or writing.
pub fn redact_pdf_bytes(
    input: &[u8],
    specs: &[RedactionSpec],
    flags: RedactionFlags,
) -> Result<Vec<u8>> {
    let raw_rects: Vec<ex_redact_rect> = specs
        .iter()
        .map(|s| {
            Ok(ex_redact_rect {
                page: c_page(s.page)?,
                rect: fz_rect::from(s.rect),
                r: s.fill_color[0],
                g: s.fill_color[1],
                b: s.fill_color[2],
            })
        })
        .collect::<Result<_>>()?;
    let rect_count =
        c_int::try_from(raw_rects.len()).map_err(|_| MuError::argument("too many redactions"))?;
    let raw_flags = ex_redact_flags {
        black_boxes: c_int::from(flags.black_boxes),
        redact_text: c_int::from(flags.redact_text),
        redact_vectors: c_int::from(flags.redact_vectors),
        redact_images: c_int::from(flags.redact_images),
        scrub_metadata: c_int::from(flags.scrub_metadata),
        scrub_attachments: c_int::from(flags.scrub_attachments),
        scrub_hidden_layers: c_int::from(flags.scrub_hidden_layers),
        scrub_overlapping_annots: c_int::from(flags.scrub_overlapping_annots),
        scrub_overlapping_forms: c_int::from(flags.scrub_overlapping_forms),
    };
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let rects_ptr = if raw_rects.is_empty() {
        ptr::null()
    } else {
        raw_rects.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` and `rects_ptr`/`rect_count` are valid for the call; `buf` receives an owned buffer.
        check(|err| unsafe {
            ex_pdf_redact_and_save(
                ctx,
                data_ptr,
                input.len(),
                rects_ptr,
                rect_count,
                &raw_flags,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Reads `/Info` dictionary entries and any `/Root /Metadata` XMP stream from `input` PDF bytes via MuPDF.
///
/// # Errors
///
/// `input` is not a valid PDF or MuPDF cannot read its trailer.
pub fn pdf_info_entries(input: &[u8]) -> Result<Vec<(String, String)>> {
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let raw = with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` is valid for the call; `buf` receives an owned buffer.
        check(|err| unsafe { ex_pdf_info_entries(ctx, data_ptr, input.len(), &mut buf, err) })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })?;
    let mut entries = Vec::new();
    let mut parts = raw.split(|&b| b == 0);
    while let (Some(key), Some(val)) = (parts.next(), parts.next()) {
        if key.is_empty() {
            continue;
        }
        entries.push((
            String::from_utf8_lossy(key).into_owned(),
            String::from_utf8_lossy(val).into_owned(),
        ));
    }
    Ok(entries)
}

/// Options controlling [`compress_pdf_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressFlags {
    /// Maximum pixel dimension for 8-bpc RGB/Gray images (`0` disables downsampling).
    pub max_image_dim: u32,
    /// Deflate level (`1..=9`).
    pub deflate_level: u32,
    /// Merge identical indirect objects and streams (MuPDF garbage level 4).
    pub deduplicate_objects: bool,
    /// Strip `/Info` and `/Root /Metadata`.
    pub strip_metadata: bool,
    /// Clean and sanitize content streams.
    pub clean_streams: bool,
    /// Write a linearized PDF.
    pub linearize: bool,
}

impl Default for CompressFlags {
    fn default() -> Self {
        Self {
            max_image_dim: 1200,
            deflate_level: 6,
            deduplicate_objects: true,
            strip_metadata: false,
            clean_streams: true,
            linearize: false,
        }
    }
}

/// Optimization metrics returned by [`compress_pdf_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompressStats {
    /// Live indirect object count before compression.
    pub objects_before: usize,
    /// Live indirect object count after compression.
    pub objects_after: usize,
    /// Number of duplicate indirect objects coalesced.
    pub deduplicated_objects: usize,
    /// Number of uncompressed or downsampled streams compressed.
    pub streams_deflated: usize,
    /// Number of oversized raster images downsampled.
    pub images_downsampled: usize,
}

/// Compresses and cleans `input` PDF bytes through MuPDF (plan D3, D4, O.8).
///
/// # Errors
///
/// `input` is not a valid PDF or MuPDF fails while optimizing or writing.
pub fn compress_pdf_bytes(input: &[u8], flags: CompressFlags) -> Result<(Vec<u8>, CompressStats)> {
    let raw_flags = ex_compress_flags {
        max_image_dim: c_int::try_from(flags.max_image_dim).unwrap_or(c_int::MAX),
        deflate_level: c_int::try_from(flags.deflate_level.clamp(1, 9)).unwrap_or(6),
        deduplicate_objects: c_int::from(flags.deduplicate_objects),
        strip_metadata: c_int::from(flags.strip_metadata),
        clean_streams: c_int::from(flags.clean_streams),
        linearize: c_int::from(flags.linearize),
    };
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let mut raw_stats = ex_compress_stats::default();
    let bytes = with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` is valid for the call; `raw_stats` and `buf` are valid out-pointers.
        check(|err| unsafe {
            ex_pdf_compress_bytes(
                ctx,
                data_ptr,
                input.len(),
                &raw_flags,
                &mut raw_stats,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })?;
    let stats = CompressStats {
        objects_before: usize::try_from(raw_stats.objects_before).unwrap_or(0),
        objects_after: usize::try_from(raw_stats.objects_after).unwrap_or(0),
        deduplicated_objects: usize::try_from(raw_stats.deduplicated_objects).unwrap_or(0),
        streams_deflated: usize::try_from(raw_stats.streams_deflated).unwrap_or(0),
        images_downsampled: usize::try_from(raw_stats.images_downsampled).unwrap_or(0),
    };
    Ok((bytes, stats))
}

/// How [`merge_pdf_bytes`] constructs the merged document's outline tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MergeOutlineMode {
    /// Omit outlines from the merged document.
    Omit,
    /// Preserve and remap existing outlines at the top level.
    PreserveExisting,
    /// Wrap each source document's outline under a top-level file bookmark.
    #[default]
    PerFile,
}

/// One input PDF for [`merge_pdf_bytes`].
#[derive(Debug, Clone, Copy)]
pub struct MergeSourceBytes<'a> {
    /// Raw PDF bytes.
    pub bytes: &'a [u8],
    /// Optional bookmark title used with [`MergeOutlineMode::PerFile`].
    pub title: Option<&'a str>,
    /// Optional zero-based page selection; `None` includes every page in order.
    pub pages: Option<&'a [usize]>,
}

/// One `/PageLabels` range entry read by [`read_pdf_page_labels`] or written by [`set_pdf_page_labels_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfPageLabelRange {
    /// Zero-based starting page index.
    pub start_page: usize,
    /// PDF numbering style character (`'D'`, `'R'`, `'r'`, `'A'`, `'a'`), or `None` for prefix-only.
    pub style: Option<char>,
    /// Starting numeric value (`>= 1`).
    pub start_number: u32,
    /// Optional label prefix.
    pub prefix: String,
}

/// Target PDF page geometry box for [`set_pdf_page_boxes_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PdfPageBoxKind {
    /// `/CropBox`.
    Crop,
    /// `/TrimBox`.
    Trim,
    /// `/BleedBox`.
    Bleed,
    /// `/ArtBox`.
    Art,
}

impl PdfPageBoxKind {
    const fn raw(self) -> c_int {
        match self {
            Self::Crop => 0,
            Self::Trim => 1,
            Self::Bleed => 2,
            Self::Art => 3,
        }
    }
}

/// One per-page PDF user-space box update for [`set_pdf_page_boxes_bytes`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfPageBoxEdit {
    /// Zero-based page index.
    pub page: usize,
    /// Target box in PDF user-space coordinates `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
}

/// Rearranges, subsets, or duplicates pages of `input` according to zero-based `pages` via MuPDF.
///
/// # Errors
///
/// `pages` is empty, an index is out of range, `input` is not a valid PDF, or MuPDF fails.
pub fn rearrange_pdf_bytes(input: &[u8], pages: &[usize]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(MuError::argument("page list must not be empty"));
    }
    let raw_pages: Vec<c_int> = pages.iter().map(|&p| c_page(p)).collect::<Result<_>>()?;
    let count =
        c_int::try_from(raw_pages.len()).map_err(|_| MuError::argument("too many pages"))?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` and `raw_pages`/`count` are valid for the call; `buf` receives an owned buffer.
        check(|err| unsafe {
            ex_pdf_rearrange_bytes(
                ctx,
                data_ptr,
                input.len(),
                raw_pages.as_ptr(),
                count,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Merges `sources` into a single PDF through MuPDF grafting, remapping annotations, internal links, and outlines.
///
/// # Errors
///
/// `sources` is empty or yields zero pages, any page index is out of range, any source is invalid, or MuPDF fails.
pub fn merge_pdf_bytes(
    sources: &[MergeSourceBytes<'_>],
    outline_mode: MergeOutlineMode,
) -> Result<Vec<u8>> {
    if sources.is_empty() {
        return Err(MuError::argument("merge requires at least one source PDF"));
    }
    let mut titles: Vec<Option<CString>> = Vec::with_capacity(sources.len());
    let mut page_vecs: Vec<Option<Vec<c_int>>> = Vec::with_capacity(sources.len());
    let mut active_indices: Vec<usize> = Vec::with_capacity(sources.len());
    for (idx, src) in sources.iter().enumerate() {
        if let Some(pages) = src.pages {
            if pages.is_empty() {
                continue;
            }
            let mapped = pages
                .iter()
                .map(|&p| c_page(p))
                .collect::<Result<Vec<_>>>()?;
            page_vecs.push(Some(mapped));
        } else {
            page_vecs.push(None);
        }
        let title = match src.title {
            Some(t) if !t.is_empty() => Some(c_text(t)?),
            _ => None,
        };
        titles.push(title);
        active_indices.push(idx);
    }
    if active_indices.is_empty() {
        return Err(MuError::argument("merged PDF contains zero pages"));
    }
    let mut raw_sources: Vec<ex_merge_source> = Vec::with_capacity(active_indices.len());
    for (pos, &src_idx) in active_indices.iter().enumerate() {
        let src = &sources[src_idx];
        let (pages_ptr, page_count) = match &page_vecs[pos] {
            Some(vec) => {
                let n =
                    c_int::try_from(vec.len()).map_err(|_| MuError::argument("too many pages"))?;
                (vec.as_ptr(), n)
            }
            None => (ptr::null(), 0),
        };
        let title_ptr = titles[pos].as_ref().map_or(ptr::null(), |t| t.as_ptr());
        let data_ptr = if src.bytes.is_empty() {
            ptr::null()
        } else {
            src.bytes.as_ptr()
        };
        raw_sources.push(ex_merge_source {
            data: data_ptr,
            len: src.bytes.len(),
            title: title_ptr,
            pages: pages_ptr,
            page_count,
        });
    }
    let source_count =
        c_int::try_from(raw_sources.len()).map_err(|_| MuError::argument("too many sources"))?;
    let raw_mode = match outline_mode {
        MergeOutlineMode::Omit => 0,
        MergeOutlineMode::PreserveExisting => 1,
        MergeOutlineMode::PerFile => 2,
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `raw_sources`, `titles`, `page_vecs`, and every source byte slice stay alive across the call.
        check(|err| unsafe {
            ex_pdf_merge_bytes(
                ctx,
                raw_sources.as_ptr(),
                source_count,
                raw_mode,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Rotates zero-based `pages` clockwise by `delta_degrees` (multiple of 90) through MuPDF.
///
/// # Errors
///
/// Any page index is out of range, `input` is not a valid PDF, or MuPDF fails.
pub fn rotate_pages_pdf_bytes(
    input: &[u8],
    pages: &[usize],
    delta_degrees: i32,
) -> Result<Vec<u8>> {
    let raw_pages: Vec<c_int> = pages.iter().map(|&p| c_page(p)).collect::<Result<_>>()?;
    let count =
        c_int::try_from(raw_pages.len()).map_err(|_| MuError::argument("too many pages"))?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let pages_ptr = if raw_pages.is_empty() {
        ptr::null()
    } else {
        raw_pages.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` and `pages_ptr`/`count` are valid for the call.
        check(|err| unsafe {
            ex_pdf_rotate_pages_bytes(
                ctx,
                data_ptr,
                input.len(),
                pages_ptr,
                count,
                delta_degrees,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Inserts a blank page of `width_pt` × `height_pt` at zero-based `at_index` (`0..=page_count`) through MuPDF.
///
/// # Errors
///
/// Dimensions are non-positive, `at_index` is past the end, `input` is not a valid PDF, or MuPDF fails.
pub fn insert_blank_page_bytes(
    input: &[u8],
    at_index: usize,
    width_pt: f32,
    height_pt: f32,
) -> Result<Vec<u8>> {
    let at = c_page(at_index)?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` is valid for the call; `buf` receives an owned buffer.
        check(|err| unsafe {
            ex_pdf_insert_blank_page_bytes(
                ctx,
                data_ptr,
                input.len(),
                at,
                width_pt,
                height_pt,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Reads `/PageLabels` ranges from `input` PDF bytes through MuPDF.
///
/// # Errors
///
/// `input` is not a valid PDF or MuPDF fails.
pub fn read_pdf_page_labels(input: &[u8]) -> Result<Vec<PdfPageLabelRange>> {
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let raw = with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` is valid for the call; `buf` receives an owned buffer.
        check(|err| unsafe { ex_pdf_read_page_labels(ctx, data_ptr, input.len(), &mut buf, err) })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })?;
    let mut ranges = Vec::new();
    for record in raw.split(|&b| b == 0) {
        if record.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(record);
        let mut cols = line.splitn(4, '\t');
        let Some(start_str) = cols.next() else {
            continue;
        };
        let Some(style_str) = cols.next() else {
            continue;
        };
        let Some(num_str) = cols.next() else {
            continue;
        };
        let prefix = cols.next().unwrap_or_default().to_owned();
        let Ok(start_page) = start_str.parse::<usize>() else {
            continue;
        };
        let style = match style_str {
            "D" => Some('D'),
            "R" => Some('R'),
            "r" => Some('r'),
            "A" => Some('A'),
            "a" => Some('a'),
            _ => None,
        };
        let start_number = num_str.parse::<u32>().unwrap_or(1).max(1);
        ranges.push(PdfPageLabelRange {
            start_page,
            style,
            start_number,
            prefix,
        });
    }
    ranges.sort_by_key(|r| r.start_page);
    Ok(ranges)
}

/// Replaces `/PageLabels` in `input` PDF bytes (or removes `/PageLabels` when `ranges` is empty) through MuPDF.
///
/// # Errors
///
/// A range has `start_number == 0`, a prefix contains a NUL, `input` is not a valid PDF, or MuPDF fails.
pub fn set_pdf_page_labels_bytes(input: &[u8], ranges: &[PdfPageLabelRange]) -> Result<Vec<u8>> {
    let mut prefixes: Vec<Option<CString>> = Vec::with_capacity(ranges.len());
    let mut raw_ranges: Vec<ex_page_label_range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start_number == 0 {
            return Err(MuError::argument("page label start_number must be >= 1"));
        }
        let start_page = c_page(range.start_page)?;
        let start_number = c_int::try_from(range.start_number)
            .map_err(|_| MuError::argument("page label start_number out of range"))?;
        let prefix = if range.prefix.is_empty() {
            None
        } else {
            Some(c_text(&range.prefix)?)
        };
        let prefix_ptr = prefix.as_ref().map_or(ptr::null(), |p| p.as_ptr());
        prefixes.push(prefix);
        raw_ranges.push(ex_page_label_range {
            start_page,
            style: range.style.map_or(0, |c| c as c_int),
            start_number,
            prefix: prefix_ptr,
        });
    }
    let count = c_int::try_from(raw_ranges.len())
        .map_err(|_| MuError::argument("too many page label ranges"))?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let ranges_ptr = if raw_ranges.is_empty() {
        ptr::null()
    } else {
        raw_ranges.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`, `ranges_ptr`, and all `prefixes` stay alive across the call.
        check(|err| unsafe {
            ex_pdf_set_page_labels_bytes(
                ctx,
                data_ptr,
                input.len(),
                ranges_ptr,
                count,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Sets `box_kind` on each page specified in `edits` (saving original `/CropBox` in `/EcclesiaPdfOrigCrop` on first
/// crop) through MuPDF.
///
/// # Errors
///
/// Any rectangle is degenerate, any page index is out of range, `input` is not a valid PDF, or MuPDF fails.
pub fn set_pdf_page_boxes_bytes(
    input: &[u8],
    box_kind: PdfPageBoxKind,
    edits: &[PdfPageBoxEdit],
) -> Result<Vec<u8>> {
    let raw_edits: Vec<ex_page_box_edit> = edits
        .iter()
        .map(|e| {
            Ok(ex_page_box_edit {
                page: c_page(e.page)?,
                rect: fz_rect {
                    x0: e.rect[0],
                    y0: e.rect[1],
                    x1: e.rect[2],
                    y1: e.rect[3],
                },
            })
        })
        .collect::<Result<_>>()?;
    let count =
        c_int::try_from(raw_edits.len()).map_err(|_| MuError::argument("too many page edits"))?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let edits_ptr = if raw_edits.is_empty() {
        ptr::null()
    } else {
        raw_edits.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` and `edits_ptr`/`count` are valid for the call.
        check(|err| unsafe {
            ex_pdf_set_page_boxes_bytes(
                ctx,
                data_ptr,
                input.len(),
                box_kind.raw(),
                edits_ptr,
                count,
                0,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}

/// Restores `/CropBox` from `/EcclesiaPdfOrigCrop` (or removes `/CropBox`) on `pages` through MuPDF.
///
/// # Errors
///
/// Any page index is out of range, `input` is not a valid PDF, or MuPDF fails.
pub fn uncrop_pdf_pages_bytes(input: &[u8], pages: &[usize]) -> Result<Vec<u8>> {
    let raw_edits: Vec<ex_page_box_edit> = pages
        .iter()
        .map(|&page| {
            Ok(ex_page_box_edit {
                page: c_page(page)?,
                rect: fz_rect::default(),
            })
        })
        .collect::<Result<_>>()?;
    let count =
        c_int::try_from(raw_edits.len()).map_err(|_| MuError::argument("too many page edits"))?;
    let data_ptr = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr()
    };
    let edits_ptr = if raw_edits.is_empty() {
        ptr::null()
    } else {
        raw_edits.as_ptr()
    };
    with_context(|ctx| {
        let mut buf = ptr::null_mut();
        // SAFETY: `data_ptr`/`input.len()` and `edits_ptr`/`count` are valid for the call.
        check(|err| unsafe {
            ex_pdf_set_page_boxes_bytes(
                ctx,
                data_ptr,
                input.len(),
                PdfPageBoxKind::Crop.raw(),
                edits_ptr,
                count,
                1,
                &mut buf,
                err,
            )
        })?;
        // SAFETY: `buf` is a newly allocated MuPDF buffer owned by us.
        Ok(unsafe { buffer::into_vec(ctx, buf) })
    })
}
