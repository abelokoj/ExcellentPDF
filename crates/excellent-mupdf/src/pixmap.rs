//! Rendered pixels.

use core::ptr::{self, NonNull};

use excellent_mupdf_sys::{
    ex_drop_pixmap, ex_pixmap_info, ex_pixmap_info_of, ex_pixmap_png, fz_pixmap,
};

use crate::buffer;
use crate::context::{check, drop_with_context, with_context};
use crate::error::Result;

/// What a render draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Background {
    /// Opaque RGB on white.
    #[default]
    White,
    /// RGBA, transparent where the page draws nothing (premultiplied, as MuPDF renders).
    Transparent,
}

impl Background {
    pub(crate) fn alpha(self) -> i32 {
        match self {
            Self::White => 0,
            Self::Transparent => 1,
        }
    }
}

/// An RGB or RGBA image MuPDF rendered.
#[derive(Debug)]
pub struct Pixmap {
    raw: NonNull<fz_pixmap>,
    info: ex_pixmap_info,
}

// SAFETY: a pixmap owns its samples; its colorspace reference is counted under MuPDF's locks, so it may be used and
// dropped from any thread. Reading the samples from several threads at once is fine; only `samples_mut` writes them,
// through `&mut self`.
unsafe impl Send for Pixmap {}
// SAFETY: as above; every `&self` method only reads.
unsafe impl Sync for Pixmap {}

impl Pixmap {
    /// A new RGB pixmap of `width` by `height` pixels, white, without alpha: for pictures the app draws itself, such
    /// as paper grain (R.23).
    ///
    /// # Errors
    ///
    /// A size of zero or one MuPDF can't allocate.
    pub fn new_rgb(width: u32, height: u32) -> Result<Self> {
        let (Ok(w), Ok(h)) = (
            core::ffi::c_int::try_from(width),
            core::ffi::c_int::try_from(height),
        ) else {
            return Err(crate::error::MuError::argument("a pixmap that large"));
        };
        if w == 0 || h == 0 {
            return Err(crate::error::MuError::argument("a pixmap has an area"));
        }
        let mut raw = ptr::null_mut();
        // SAFETY: plain allocation; `raw` receives a new pixmap we own.
        with_context(|ctx| {
            check(|err| unsafe { excellent_mupdf_sys::ex_new_pixmap_rgb(ctx, w, h, &mut raw, err) })
        })?;
        Ok(Self::new(
            NonNull::new(raw).expect("ex_new_pixmap_rgb succeeded with a pixmap"),
        ))
    }

    pub(crate) fn new(raw: NonNull<fz_pixmap>) -> Self {
        let mut info = ex_pixmap_info::default();
        // A pixmap's fields can't fail to read; with_context fails only if MuPDF never started, and then no pixmap
        // could exist.
        let _ = with_context(|ctx| {
            // SAFETY: `raw` is a live pixmap we own.
            unsafe { ex_pixmap_info_of(ctx, raw.as_ptr(), &mut info) };
            Ok(())
        });
        Self { raw, info }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        u32::try_from(self.info.width).expect("MuPDF pixmaps have a non-negative width")
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        u32::try_from(self.info.height).expect("MuPDF pixmaps have a non-negative height")
    }

    /// Bytes per row of [`Self::samples`].
    pub fn stride(&self) -> usize {
        usize::try_from(self.info.stride).expect("rendered pixmaps have a positive stride")
    }

    /// Bytes per pixel: 3 for RGB, 4 for RGBA.
    pub fn components(&self) -> usize {
        usize::try_from(self.info.components)
            .expect("MuPDF pixmaps have a positive component count")
    }

    /// True for RGBA.
    pub fn has_alpha(&self) -> bool {
        self.info.alpha != 0
    }

    /// The pixels, row by row, [`Self::stride`] bytes per row.
    pub fn samples(&self) -> &[u8] {
        let len = self.stride() * self.height() as usize;
        if len == 0 {
            return &[];
        }
        // SAFETY: `samples` points at `stride * height` bytes owned by the pixmap, which lives as long as `self`.
        unsafe { core::slice::from_raw_parts(self.info.samples, len) }
    }

    /// The pixels, to change in place (smart dark mode recolours a rendered tile).
    pub fn samples_mut(&mut self) -> &mut [u8] {
        let len = self.stride() * self.height() as usize;
        if len == 0 {
            return &mut [];
        }
        // SAFETY: as in `samples`; `&mut self` makes this the only access, and nothing else holds the pixmap.
        unsafe { core::slice::from_raw_parts_mut(self.info.samples, len) }
    }

    /// The pixmap encoded as PNG.
    ///
    /// # Errors
    ///
    /// MuPDF can't encode it (out of memory).
    pub fn to_png(&self) -> Result<Vec<u8>> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the pixmap is live; encoding only reads it.
            check(|err| unsafe { ex_pixmap_png(ctx, self.raw.as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            Ok(unsafe { buffer::into_vec(ctx, buf) })
        })
    }
}

impl Drop for Pixmap {
    fn drop(&mut self) {
        // SAFETY: we own the pixmap.
        drop_with_context(|ctx| unsafe { ex_drop_pixmap(ctx, self.raw.as_ptr()) });
    }
}
