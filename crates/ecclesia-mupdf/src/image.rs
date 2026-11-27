//! The images a page draws, for smart dark mode (plan R.14).

use core::ptr::{self, NonNull};

use ecclesia_mupdf_sys::{
    ex_display_list_images, ex_drop_image, ex_drop_page_images, ex_image_sample, fz_display_list,
    fz_image,
};

use crate::context::{check, drop_with_context, with_context};
use crate::cookie::{self, Cookie};
use crate::error::Result;
use crate::geometry::PageRect;
use crate::pixmap::Pixmap;

/// One image a page draws: where it lands and how many pixels it has.
#[derive(Debug)]
pub struct PageImage {
    raw: NonNull<fz_image>,
    bounds: PageRect,
    width: u32,
    height: u32,
}

// SAFETY: an image is immutable once loaded and reference counted under MuPDF's locks; decoding it from another
// thread with that thread's own context is what rendering a display list (which holds it) already does.
unsafe impl Send for PageImage {}
// SAFETY: as above; every `&self` method only reads or decodes.
unsafe impl Sync for PageImage {}

impl PageImage {
    /// Where the image lands, in page points: the whole image, even where a clip hides part of it.
    pub fn bounds(&self) -> PageRect {
        self.bounds
    }

    /// The image's own width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The image's own height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// An RGB copy of the image (no alpha) for its colour statistics, decoded as if drawn 64 pixels across. MuPDF
    /// subsamples only by powers of two and only formats that allow it, so it can come back larger, up to full size
    /// for an image already decoded.
    ///
    /// # Errors
    ///
    /// The image can't be decoded, or holds no colours (a mask).
    pub fn sample(&self) -> Result<Pixmap> {
        let mut raw = ptr::null_mut();
        with_context(|ctx| {
            // SAFETY: the image is live (we hold a reference) and only read.
            check(|err| unsafe { ex_image_sample(ctx, self.raw.as_ptr(), &mut raw, err) })
        })?;
        Ok(Pixmap::new(
            NonNull::new(raw).expect("ex_image_sample succeeded with a pixmap"),
        ))
    }
}

impl Drop for PageImage {
    fn drop(&mut self) {
        // SAFETY: we own this reference; MuPDF counts it under its locks.
        drop_with_context(|ctx| unsafe { ex_drop_image(ctx, self.raw.as_ptr()) });
    }
}

/// The images `list` draws, in drawing order (at most 4,096). `cookie` cancels the run.
pub(crate) fn images_of(
    list: NonNull<fz_display_list>,
    cookie: Option<&Cookie>,
) -> Result<Vec<PageImage>> {
    let mut items = ptr::null_mut();
    let mut count = 0;
    with_context(|ctx| {
        // SAFETY: the list is live and only read; the cookie, if any, outlives the call.
        check(|err| unsafe {
            ex_display_list_images(
                ctx,
                list.as_ptr(),
                cookie::raw(cookie),
                &mut items,
                &mut count,
                err,
            )
        })
    })?;
    let count = usize::try_from(count).expect("an image count is never negative");
    let images = if items.is_null() || count == 0 {
        Vec::new()
    } else {
        // SAFETY: on success the shim hands over `count` items, each with a kept image reference we now own.
        let items = unsafe { core::slice::from_raw_parts(items, count) };
        items
            .iter()
            .map(|item| PageImage {
                raw: NonNull::new(item.image).expect("every listed image is kept"),
                bounds: item.bbox.into(),
                width: u32::try_from(item.width).unwrap_or(0),
                height: u32::try_from(item.height).unwrap_or(0),
            })
            .collect()
    };
    // SAFETY: the array is ours; its images now belong to the `PageImage`s.
    drop_with_context(|ctx| unsafe { ex_drop_page_images(ctx, items) });
    cookie::finish(cookie, images)
}
