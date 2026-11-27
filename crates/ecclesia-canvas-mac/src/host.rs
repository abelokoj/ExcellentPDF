//! The macOS side of the page canvas (plan D5, K.9): the view as the shared canvas's [`Host`], drawing a frame with
//! Core Graphics in the view's `drawRect:`, tiles as `CGImage`s, and waking the view from workers on the main queue.
//!
//! AppKit draws only in `drawRect:`: the view lends its context for the frame ([`MacHost::lend`]), and between frames
//! [`Host::frame_ready`] says no, so the canvas asks for a redraw instead of drawing. See DD10 for Core Graphics
//! rather than a `CAMetalLayer`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};

use ecclesia_canvas::Rect;
use ecclesia_canvas::page::host::{InkPixel, Quad, WASH_OPACITY, Wake, Wash};
use ecclesia_canvas::page::{Host, PageCanvas};
use ecclesia_mupdf::Pixmap;
use nsh_core::JsonValue;
use nsh_platform_macos::ComponentContext;
use objc2::rc::{Retained, Weak};
use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGBlendMode, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider,
    CGImage, CGImageAlphaInfo, CGInterpolationQuality, CGLineCap, CGLineJoin, CGPath,
};

use crate::view::CanvasView;

/// The page canvas as macOS runs it.
pub(crate) type Canvas = PageCanvas<MacHost>;

thread_local! {
    /// The canvases on this (the main) thread, by id, for the wake-ups workers send.
    static VIEWS: RefCell<HashMap<u64, Weak<CanvasView>>> = RefCell::default();
}

/// The next canvas's id.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The view and what draws into it.
pub(crate) struct MacHost {
    view: Weak<CanvasView>,
    /// The view's key in [`VIEWS`].
    id: u64,
    /// Where the canvas tells the app about the document and the page shown.
    events: ComponentContext,
    /// The view's size in device pixels, as last given to the canvas.
    size: Cell<(f32, f32)>,
    /// Device pixels per point (the window's backing scale).
    scale: Cell<f64>,
    /// The view's context while it draws.
    target: Option<CFRetained<CGContext>>,
}

impl MacHost {
    /// The host of the canvas in `view`, telling the app through `events`.
    pub(crate) fn new(view: &CanvasView, events: ComponentContext) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        VIEWS.with_borrow_mut(|views| views.insert(id, Weak::from(view)));
        Self {
            view: Weak::from(view),
            id,
            events,
            size: Cell::new((0.0, 0.0)),
            scale: Cell::new(1.0),
            target: None,
        }
    }

    /// The canvas is `size` device pixels at `scale` device pixels per point; whether that changed.
    pub(crate) fn set_size(&self, size: (f32, f32), scale: f64) -> bool {
        let changed = self.size.get() != size || self.scale.get() != scale;
        self.size.set(size);
        self.scale.set(scale);
        changed
    }

    /// Device pixels per point.
    pub(crate) fn scale(&self) -> f64 {
        self.scale.get()
    }

    /// Lends the view's `context` for a frame (`None`: the frame is over).
    pub(crate) fn lend(&mut self, context: Option<CFRetained<CGContext>>) {
        self.target = context;
    }

    /// The context drawn into, inside a frame.
    fn cg(&self) -> Option<&CGContext> {
        self.target.as_deref()
    }

    /// Fills the current path in `rgb` as `wash` says (`even_odd`: the even-odd rule).
    fn fill_washed(&self, rgb: u32, wash: Wash) {
        let c = self.cg();
        CGContext::save_g_state(c);
        let alpha = match wash {
            Wash::Darkest => {
                // Over a page, darkest-wins keeps text black under the colour.
                CGContext::set_blend_mode(c, CGBlendMode::Darken);
                1.0
            }
            Wash::Translucent => WASH_OPACITY,
        };
        set_fill(c, rgb, alpha);
        CGContext::fill_path(c);
        CGContext::restore_g_state(c);
    }

    /// Adds `rect` with corners of `radius` to the current path.
    fn add_rounded(&self, rect: &Rect, radius: f32) {
        let rect = cg_rect(rect);
        // Core Graphics refuses corners larger than half a side.
        let radius = CGFloat::from(radius)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0)
            .max(0.0);
        // SAFETY: no transform (null); the rect and corners are in range, as checked above.
        let path = unsafe { CGPath::with_rounded_rect(rect, radius, radius, std::ptr::null()) };
        CGContext::add_path(self.cg(), Some(&path));
    }
}

impl Drop for MacHost {
    fn drop(&mut self) {
        let id = self.id;
        // The thread-local is gone when the main thread ends; there is nothing to forget then.
        let _ = VIEWS.try_with(|views| views.borrow_mut().remove(&id));
    }
}

/// The canvas view `id`, while it lives. Called on the main thread, where the views are.
fn view(id: u64) -> Option<Retained<CanvasView>> {
    VIEWS.with_borrow(|views| views.get(&id).and_then(Weak::load))
}

/// Wakes the canvas `id` after a worker finished: its jobs are taken on the main thread.
fn jobs_done(id: u64) {
    let Some(view) = view(id) else {
        return;
    };
    if !view.jobs_done() {
        // The canvas is busy further up this thread's stack: try again once it is done.
        dispatch2::DispatchQueue::main().exec_async(move || jobs_done(id));
    }
}

impl Host for MacHost {
    /// AppKit lays the UI out in points.
    fn ui_pixels_per_point(&self) -> f32 {
        1.0
    }

    type Image = CFRetained<CGImage>;
    type Error = Infallible;

    fn size(&self) -> (f32, f32) {
        self.size.get()
    }

    fn invalidate(&self) {
        if let Some(view) = self.view.load() {
            view.setNeedsDisplay(true);
        }
    }

    fn emit(&self, name: &str, data: JsonValue) {
        self.events.emit(name, data);
    }

    fn waker(&self) -> Wake {
        let id = self.id;
        Box::new(move || dispatch2::DispatchQueue::main().exec_async(move || jobs_done(id)))
    }

    fn upload(&mut self, pixmap: &Pixmap) -> Option<(CFRetained<CGImage>, usize)> {
        let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let (n, stride, samples) = (pixmap.components(), pixmap.stride(), pixmap.samples());
        let rgb: Vec<u8> = if n == 3 {
            samples[..stride * height].to_vec()
        } else {
            // Not RGB: keep the first three channels, as Windows does.
            samples
                .chunks_exact(stride)
                .take(height)
                .flat_map(|row| {
                    row[..width * n]
                        .chunks_exact(n)
                        .flat_map(|px| px[..3].iter().copied())
                })
                .collect()
        };
        let row = if n == 3 { stride } else { width * 3 };
        let size = rgb.len();
        // Core Graphics keeps its own copy of the pixels.
        let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&rgb)))?;
        let space = CGColorSpace::new_device_rgb()?;
        // SAFETY: 8-bit RGB without alpha, `row` bytes a row, `height` rows in the provider; no decode array.
        let image = unsafe {
            CGImage::new(
                width,
                height,
                8,
                24,
                row,
                Some(&space),
                CGBitmapInfo(CGImageAlphaInfo::None.0),
                Some(&provider),
                std::ptr::null(),
                true,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }?;
        Some((image, size))
    }

    fn begin(&mut self, _width: f32, _height: f32) -> Result<(), Infallible> {
        let Some(c) = self.cg() else {
            // Drawn outside `drawRect:`: draw it again there.
            self.invalidate();
            return Ok(());
        };
        CGContext::save_g_state(Some(c));
        // The view is flipped (top-left origin, y down) and in points; the canvas draws in device pixels.
        let points = 1.0 / self.scale.get();
        CGContext::scale_ctm(Some(c), points, points);
        CGContext::set_interpolation_quality(Some(c), CGInterpolationQuality::High);
        Ok(())
    }

    fn end(&mut self) -> Result<(), Infallible> {
        CGContext::restore_g_state(self.cg());
        Ok(())
    }

    fn clear(&mut self, rgb: u32) {
        let (width, height) = self.size.get();
        set_fill(self.cg(), rgb, 1.0);
        CGContext::fill_rect(
            self.cg(),
            CGRect::new(CGPoint::ZERO, CGSize::new(width.into(), height.into())),
        );
    }

    fn fill(&mut self, rect: &Rect, rgb: u32) -> Result<(), Infallible> {
        set_fill(self.cg(), rgb, 1.0);
        CGContext::fill_rect(self.cg(), cg_rect(rect));
        Ok(())
    }

    fn image(&mut self, image: &CFRetained<CGImage>, dest: &Rect) {
        let c = self.cg();
        // Images draw bottom-up; the view is flipped, so flip them back about the destination.
        CGContext::save_g_state(c);
        CGContext::translate_ctm(c, dest.x0.into(), dest.y1.into());
        CGContext::scale_ctm(c, 1.0, -1.0);
        CGContext::draw_image(
            c,
            CGRect::new(
                CGPoint::ZERO,
                CGSize::new(dest.width().into(), dest.height().into()),
            ),
            Some(image),
        );
        CGContext::restore_g_state(c);
    }

    fn faint_image(&mut self, image: &CFRetained<CGImage>, dest: &Rect, opacity: f32) {
        let c = self.cg();
        CGContext::save_g_state(c);
        CGContext::set_alpha(c, f64::from(opacity.clamp(0.0, 1.0)).into());
        self.image(image, dest);
        let c = self.cg();
        CGContext::restore_g_state(c);
    }

    fn highlight(&mut self, quads: &[Quad], rgb: u32, wash: Wash) -> Result<(), Infallible> {
        if quads.is_empty() {
            return Ok(());
        }
        let c = self.cg();
        CGContext::begin_path(c);
        for [first, rest @ ..] in quads {
            CGContext::move_to_point(c, first.0.into(), first.1.into());
            for (x, y) in rest {
                CGContext::add_line_to_point(c, (*x).into(), (*y).into());
            }
            CGContext::close_path(c);
        }
        self.fill_washed(rgb, wash);
        Ok(())
    }

    fn band(&mut self, rect: &Rect, radius: f32, rgb: u32, wash: Wash) -> Result<(), Infallible> {
        CGContext::begin_path(self.cg());
        self.add_rounded(rect, radius);
        self.fill_washed(rgb, wash);
        Ok(())
    }

    fn dim_outside(
        &mut self,
        screen: &Rect,
        hole: &Rect,
        radius: f32,
        rgb: u32,
        alpha: f32,
    ) -> Result<(), Infallible> {
        let c = self.cg();
        CGContext::begin_path(c);
        CGContext::add_rect(c, cg_rect(screen));
        self.add_rounded(hole, radius);
        // Even-odd filling leaves the hole, inside the screen, empty.
        set_fill(c, rgb, alpha.clamp(0.0, 1.0));
        CGContext::eo_fill_path(c);
        Ok(())
    }

    // shortcut: one stroke a segment (each as wide as the mean of its ends); merge runs of one width if long strokes
    // show in K.24.
    fn polyline(&mut self, points: &[InkPixel], rgb: u32) -> Result<(), Infallible> {
        let c = self.cg();
        let (r, g, b) = channels(rgb);
        if let [(x, y, width)] = points {
            let radius = CGFloat::from(*width) / 2.0;
            set_fill(c, rgb, 1.0);
            CGContext::fill_ellipse_in_rect(
                c,
                CGRect::new(
                    CGPoint::new(CGFloat::from(*x) - radius, CGFloat::from(*y) - radius),
                    CGSize::new(2.0 * radius, 2.0 * radius),
                ),
            );
            return Ok(());
        }
        CGContext::save_g_state(c);
        CGContext::set_rgb_stroke_color(c, r, g, b, 1.0);
        CGContext::set_line_cap(c, CGLineCap::Round);
        CGContext::set_line_join(c, CGLineJoin::Round);
        for pair in points.windows(2) {
            let ((x0, y0, w0), (x1, y1, w1)) = (pair[0], pair[1]);
            CGContext::set_line_width(c, CGFloat::from((w0 + w1) / 2.0));
            CGContext::begin_path(c);
            CGContext::move_to_point(c, x0.into(), y0.into());
            CGContext::add_line_to_point(c, x1.into(), y1.into());
            CGContext::stroke_path(c);
        }
        CGContext::restore_g_state(c);
        Ok(())
    }

    fn frame_ready(&mut self) -> bool {
        self.target.is_some()
    }

    fn content_changed(&self) {
        // Later: VoiceOver may read the text back at once, and the canvas is busy now.
        let id = self.id;
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(view) = view(id) {
                view.text_changed();
            }
        });
    }

    fn selection_changed(&self) {
        let id = self.id;
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(view) = view(id) {
                view.selection_moved();
            }
        });
    }
}

/// `0xRRGGBB` as Core Graphics' channels.
fn channels(rgb: u32) -> (CGFloat, CGFloat, CGFloat) {
    let channel =
        |shift: u32| CGFloat::from(u8::try_from(rgb >> shift & 0xff).expect("a byte")) / 255.0;
    (channel(16), channel(8), channel(0))
}

/// Sets the fill to `rgb` at `alpha`.
fn set_fill(c: Option<&CGContext>, rgb: u32, alpha: f32) {
    let (r, g, b) = channels(rgb);
    CGContext::set_rgb_fill_color(c, r, g, b, alpha.into());
}

/// `rect` as Core Graphics takes it.
fn cg_rect(rect: &Rect) -> CGRect {
    CGRect::new(
        CGPoint::new(rect.x0.into(), rect.y0.into()),
        CGSize::new(rect.width().into(), rect.height().into()),
    )
}
