//! PDF annotations.

use core::ffi::c_int;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    self as sys, PDF_ANNOT_UNKNOWN, ex_annot_contents, ex_annot_info, ex_drop_annot,
    ex_first_annot, ex_next_annot, fz_rect, pdf_annot,
};

use crate::buffer;
use crate::context::{check, drop_with_context, with_context};
use crate::error::Result;
use crate::geometry::PageRect;
use crate::page::Page;

/// The subtype of a PDF annotation (PDF 2.0, table 171).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotKind {
    /// A note icon (sticky note).
    Text,
    /// A link.
    Link,
    /// Text drawn on the page.
    FreeText,
    /// A straight line.
    Line,
    /// A rectangle.
    Square,
    /// An ellipse.
    Circle,
    /// A closed polygon.
    Polygon,
    /// An open polyline.
    PolyLine,
    /// Text highlight.
    Highlight,
    /// Text underline.
    Underline,
    /// Squiggly underline.
    Squiggly,
    /// Strikeout.
    StrikeOut,
    /// Area marked for redaction.
    Redact,
    /// A rubber stamp.
    Stamp,
    /// An insertion caret.
    Caret,
    /// Freehand ink.
    Ink,
    /// The pop-up window of another annotation.
    Popup,
    /// An attached file.
    FileAttachment,
    /// A sound.
    Sound,
    /// A movie (deprecated in PDF 2.0).
    Movie,
    /// Rich media (video, 3D).
    RichMedia,
    /// A form field.
    Widget,
    /// A screen region for media.
    Screen,
    /// A printer's mark.
    PrinterMark,
    /// A trap network.
    TrapNet,
    /// A watermark.
    Watermark,
    /// 3D artwork.
    ThreeD,
    /// A projection (PDF 2.0).
    Projection,
    /// A subtype MuPDF doesn't know.
    Unknown,
}

impl AnnotKind {
    fn from_raw(raw: c_int) -> Self {
        match raw {
            sys::PDF_ANNOT_TEXT => Self::Text,
            sys::PDF_ANNOT_LINK => Self::Link,
            sys::PDF_ANNOT_FREE_TEXT => Self::FreeText,
            sys::PDF_ANNOT_LINE => Self::Line,
            sys::PDF_ANNOT_SQUARE => Self::Square,
            sys::PDF_ANNOT_CIRCLE => Self::Circle,
            sys::PDF_ANNOT_POLYGON => Self::Polygon,
            sys::PDF_ANNOT_POLY_LINE => Self::PolyLine,
            sys::PDF_ANNOT_HIGHLIGHT => Self::Highlight,
            sys::PDF_ANNOT_UNDERLINE => Self::Underline,
            sys::PDF_ANNOT_SQUIGGLY => Self::Squiggly,
            sys::PDF_ANNOT_STRIKE_OUT => Self::StrikeOut,
            sys::PDF_ANNOT_REDACT => Self::Redact,
            sys::PDF_ANNOT_STAMP => Self::Stamp,
            sys::PDF_ANNOT_CARET => Self::Caret,
            sys::PDF_ANNOT_INK => Self::Ink,
            sys::PDF_ANNOT_POPUP => Self::Popup,
            sys::PDF_ANNOT_FILE_ATTACHMENT => Self::FileAttachment,
            sys::PDF_ANNOT_SOUND => Self::Sound,
            sys::PDF_ANNOT_MOVIE => Self::Movie,
            sys::PDF_ANNOT_RICH_MEDIA => Self::RichMedia,
            sys::PDF_ANNOT_WIDGET => Self::Widget,
            sys::PDF_ANNOT_SCREEN => Self::Screen,
            sys::PDF_ANNOT_PRINTER_MARK => Self::PrinterMark,
            sys::PDF_ANNOT_TRAP_NET => Self::TrapNet,
            sys::PDF_ANNOT_WATERMARK => Self::Watermark,
            sys::PDF_ANNOT_3D => Self::ThreeD,
            sys::PDF_ANNOT_PROJECTION => Self::Projection,
            _ => Self::Unknown,
        }
    }
}

/// An annotation on a PDF page, which it borrows. Not `Send`.
#[derive(Debug)]
pub struct Annot<'page> {
    raw: NonNull<pdf_annot>,
    _page: PhantomData<&'page Page<'page>>,
}

impl Annot<'_> {
    fn info(&self) -> Result<(AnnotKind, PageRect)> {
        let (mut kind, mut rect) = (PDF_ANNOT_UNKNOWN, fz_rect::default());
        with_context(|ctx| {
            // SAFETY: the annotation is live and on its page's thread.
            check(|err| unsafe { ex_annot_info(ctx, self.raw.as_ptr(), &mut kind, &mut rect, err) })
        })?;
        Ok((AnnotKind::from_raw(kind), rect.into()))
    }

    /// The annotation's subtype.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn kind(&self) -> Result<AnnotKind> {
        Ok(self.info()?.0)
    }

    /// Where the annotation sits on the page, in points.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn rect(&self) -> Result<PageRect> {
        Ok(self.info()?.1)
    }

    /// The annotation's text (`/Contents`), empty when it has none.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation's dictionary.
    pub fn contents(&self) -> Result<String> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the annotation is live and on its page's thread.
            check(|err| unsafe { ex_annot_contents(ctx, self.raw.as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { buffer::into_string(ctx, buf) })
        })
    }
}

impl Drop for Annot<'_> {
    fn drop(&mut self) {
        // SAFETY: we own this reference (the shim kept it).
        drop_with_context(|ctx| unsafe { ex_drop_annot(ctx, self.raw.as_ptr()) });
    }
}

/// A page's annotations, in order.
pub(crate) fn list<'page>(page: &'page Page<'_>) -> Result<Vec<Annot<'page>>> {
    let mut annots = Vec::new();
    with_context(|ctx| {
        let mut raw = ptr::null_mut();
        // SAFETY: the page is live and on its own thread.
        check(|err| unsafe { ex_first_annot(ctx, page.raw().as_ptr(), &mut raw, err) })?;
        while let Some(current) = NonNull::new(raw) {
            // Owned from here, so it is dropped even if the next step fails.
            annots.push(Annot {
                raw: current,
                _page: PhantomData,
            });
            raw = ptr::null_mut();
            // SAFETY: `current` is a live annotation of this page.
            check(|err| unsafe { ex_next_annot(ctx, current.as_ptr(), &mut raw, err) })?;
        }
        Ok(())
    })?;
    Ok(annots)
}
