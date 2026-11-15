//! Editing and saving a PDF (K.14, K.15): operations, undo and redo, outline edits (B.1–B.5), link destinations, and
//! writing the document with its edits.
//!
//! A change happens only through the [`Edit`] that [`Document::operation`] hands out, so every change is one step of
//! MuPDF's undo journal: it is kept when the step works and taken back when it fails.

use core::ffi::{CStr, c_char, c_int};
use std::ffi::CString;
use std::path::Path;

use excellent_mupdf_sys::{
    ex_abandon_operation, ex_begin_operation, ex_end_operation, ex_is_pdf, ex_link_uri,
    ex_outline_delete, ex_outline_edit, ex_outline_insert, ex_outline_move, ex_outline_update,
    ex_redo, ex_save_pdf, ex_undo, ex_undo_state,
};

use crate::context::{check, with_context};
use crate::document::{Document, c_path};
use crate::error::{MuError, Result};
use crate::geometry::PagePoint;

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
