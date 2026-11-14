//! Loaded pages.

use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    ex_drop_page, ex_new_display_list, ex_new_stext_page, ex_page_bounds, ex_render_page, fz_page,
    fz_rect,
};

use crate::annot::{self, Annot};
use crate::context::{check, drop_with_context, with_context};
use crate::cookie::{self, Cookie};
use crate::display_list::DisplayList;
use crate::document::Document;
use crate::error::Result;
use crate::geometry::{Matrix, PageRect};
use crate::link::{self, Link};
use crate::pixmap::{Background, Pixmap};
use crate::text::StextPage;

/// A page of a [`Document`], which it borrows. Not `Send`: it stays on the thread that loaded it.
#[derive(Debug)]
pub struct Page<'doc> {
    raw: NonNull<fz_page>,
    _doc: PhantomData<&'doc Document>,
}

impl Page<'_> {
    pub(crate) fn new(raw: NonNull<fz_page>) -> Self {
        Self {
            raw,
            _doc: PhantomData,
        }
    }

    pub(crate) fn raw(&self) -> NonNull<fz_page> {
        self.raw
    }

    /// The page's bounds in points: its crop box, with the page's own rotation applied, top-left at `(0, 0)` for
    /// most documents.
    ///
    /// # Errors
    ///
    /// MuPDF can't compute them.
    pub fn bounds(&self) -> Result<PageRect> {
        let mut rect = fz_rect::default();
        with_context(|ctx| {
            // SAFETY: the page is live and on its own thread.
            check(|err| unsafe { ex_page_bounds(ctx, self.raw.as_ptr(), &mut rect, err) })
        })?;
        Ok(rect.into())
    }

    /// Renders the whole page through `ctm` into a new RGB pixmap.
    ///
    /// # Errors
    ///
    /// MuPDF fails to render (out of memory, a broken content stream it can't repair).
    pub fn render(&self, ctm: &Matrix, background: Background) -> Result<Pixmap> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the page is live and on its own thread.
            check(|err| unsafe {
                ex_render_page(
                    ctx,
                    self.raw.as_ptr(),
                    (*ctm).into(),
                    background.alpha(),
                    &mut raw,
                    err,
                )
            })
        })?;
        Ok(Pixmap::new(
            NonNull::new(raw).expect("ex_render_page succeeded with a pixmap"),
        ))
    }

    /// Records the page once into a display list, which any thread can render. `cookie` cancels it.
    ///
    /// # Errors
    ///
    /// MuPDF fails to run the page, or `cookie` was aborted.
    pub fn display_list(&self, cookie: Option<&Cookie>) -> Result<DisplayList> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the page is live and on its own thread; the cookie, if any, outlives the call.
            check(|err| unsafe {
                ex_new_display_list(ctx, self.raw.as_ptr(), cookie::raw(cookie), &mut raw, err)
            })
        })?;
        let list =
            DisplayList::new(NonNull::new(raw).expect("ex_new_display_list succeeded with a list"));
        cookie::finish(cookie, list)
    }

    /// Extracts the page's structured text as the text layer reads it (hyphens joined, structure kept, glyph boxes
    /// from outlines). `cookie` cancels it.
    ///
    /// # Errors
    ///
    /// MuPDF fails to run the page, or `cookie` was aborted.
    pub fn text(&self, cookie: Option<&Cookie>) -> Result<StextPage> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the page is live and on its own thread; the cookie, if any, outlives the call.
            check(|err| unsafe {
                ex_new_stext_page(
                    ctx,
                    self.raw.as_ptr(),
                    crate::text::READING_FLAGS,
                    cookie::raw(cookie),
                    &mut raw,
                    err,
                )
            })
        })?;
        let text = StextPage::new(
            NonNull::new(raw).expect("ex_new_stext_page succeeded with a text page"),
        );
        cookie::finish(cookie, text)
    }

    /// The page's PDF annotations, in order; empty for other formats.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the annotation list.
    pub fn annots(&self) -> Result<Vec<Annot<'_>>> {
        annot::list(self)
    }

    /// The page's links, in the order MuPDF reads them, each resolved to its page when it goes somewhere in this
    /// document.
    ///
    /// # Errors
    ///
    /// MuPDF can't read the links.
    pub fn links(&self) -> Result<Vec<Link>> {
        link::load(self)
    }
}

impl Drop for Page<'_> {
    fn drop(&mut self) {
        // SAFETY: we own the page, and its document outlives it (the borrow).
        drop_with_context(|ctx| unsafe { ex_drop_page(ctx, self.raw.as_ptr()) });
    }
}
