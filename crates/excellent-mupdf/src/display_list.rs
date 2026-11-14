//! Display lists: a page recorded once and rendered many times, from any thread.

use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    ex_display_list_bounds, ex_drop_display_list, ex_render_display_list, fz_display_list, fz_rect,
};

use crate::context::{check, drop_with_context, with_context};
use crate::cookie::{self, Cookie};
use crate::error::Result;
use crate::geometry::{DeviceRect, Matrix, PageRect};
use crate::image::{self, PageImage};
use crate::pixmap::{Background, Pixmap};

/// A page's drawing commands. Immutable, so it is `Send + Sync`: share it in an `Arc` and render tiles of it from
/// several workers at once (plan §5.3).
#[derive(Debug)]
pub struct DisplayList {
    raw: NonNull<fz_display_list>,
}

// SAFETY: a display list is immutable once recorded, and what it holds (fonts, images, colorspaces) is reference
// counted under MuPDF's locks. MuPDF supports running one list from several threads, each with its own cloned context
// (its docs' multi-threaded example; Sumatra's patch 0007 makes this safe for PDF).
unsafe impl Send for DisplayList {}
// SAFETY: as above.
unsafe impl Sync for DisplayList {}

impl DisplayList {
    pub(crate) fn new(raw: NonNull<fz_display_list>) -> Self {
        Self { raw }
    }

    /// The area the list draws, in page points.
    ///
    /// # Errors
    ///
    /// MuPDF can't compute it.
    pub fn bounds(&self) -> Result<PageRect> {
        let mut rect = fz_rect::default();
        with_context(|ctx| {
            // SAFETY: the list is live and only read.
            check(|err| unsafe { ex_display_list_bounds(ctx, self.raw.as_ptr(), &mut rect, err) })
        })?;
        Ok(rect.into())
    }

    /// The images the list draws, in drawing order (at most 4,096), for smart dark mode (R.14). `cookie` cancels it.
    ///
    /// # Errors
    ///
    /// MuPDF fails to run the list, or `cookie` was aborted.
    pub fn images(&self, cookie: Option<&Cookie>) -> Result<Vec<PageImage>> {
        image::images_of(self.raw, cookie)
    }

    /// Renders the whole list through `ctm`.
    ///
    /// # Errors
    ///
    /// MuPDF fails to render.
    pub fn render(&self, ctm: &Matrix, background: Background) -> Result<Pixmap> {
        let area = ctm.apply_rect(&self.bounds()?);
        self.render_area(ctm, area, background, None)
    }

    /// Renders only `area` of the page as `ctm` maps it onto the device: one tile. The pixmap covers `area` exactly.
    /// `cookie` cancels it.
    ///
    /// # Errors
    ///
    /// MuPDF fails to render, or `cookie` was aborted.
    pub fn render_area(
        &self,
        ctm: &Matrix,
        area: DeviceRect,
        background: Background,
        cookie: Option<&Cookie>,
    ) -> Result<Pixmap> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the list is live and only read; the cookie, if any, outlives the call.
            check(|err| unsafe {
                ex_render_display_list(
                    ctx,
                    self.raw.as_ptr(),
                    (*ctm).into(),
                    area.into(),
                    background.alpha(),
                    cookie::raw(cookie),
                    &mut raw,
                    err,
                )
            })
        })?;
        let pixmap =
            Pixmap::new(NonNull::new(raw).expect("ex_render_display_list succeeded with a pixmap"));
        cookie::finish(cookie, pixmap)
    }
}

impl Drop for DisplayList {
    fn drop(&mut self) {
        // SAFETY: we own this reference; MuPDF counts it under its locks.
        drop_with_context(|ctx| unsafe { ex_drop_display_list(ctx, self.raw.as_ptr()) });
    }
}
