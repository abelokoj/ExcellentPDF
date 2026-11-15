//! The GTK side of the page canvas (plan D5, K.10): the widget as the shared canvas's [`Host`], drawing a frame as
//! GSK render nodes through `GtkSnapshot`, tiles as `GdkTexture`s, and waking the widget from workers.
//!
//! GTK draws only in a widget's `snapshot`: the widget lends its snapshot for the frame ([`GtkHost::lend`]), and
//! between frames [`Host::frame_ready`] says no, so the canvas asks for a redraw instead of drawing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};

use excellent_canvas::Rect;
use excellent_canvas::page::host::{InkPixel, Quad, WASH_OPACITY, Wake, Wash};
use excellent_canvas::page::{Host, PageCanvas};
use excellent_mupdf::Pixmap;
use nsh_core::JsonValue;
use nsh_platform_linux::ComponentContext;
use nsh_platform_linux::gtk4 as gtk;

use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use crate::widget::CanvasWidget;

/// The page canvas as GTK runs it.
pub(crate) type Canvas = PageCanvas<GtkHost>;

thread_local! {
    /// The canvases on this (the UI) thread, by id, for the wake-ups workers send.
    static WIDGETS: RefCell<HashMap<u64, glib::WeakRef<CanvasWidget>>> = RefCell::default();
}

/// The next canvas's id.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The widget and what draws into it.
pub(crate) struct GtkHost {
    widget: glib::WeakRef<CanvasWidget>,
    /// The widget's key in [`WIDGETS`].
    id: u64,
    /// Where the canvas tells the app about the document and the page shown.
    events: ComponentContext,
    /// The widget's size in device pixels, as last given to the canvas.
    size: Cell<(f32, f32)>,
    /// Device pixels per widget pixel.
    scale: Cell<f64>,
    /// The widget's snapshot while it draws.
    target: Option<gtk::Snapshot>,
    /// The frame being drawn, in device pixels.
    frame: gtk::Snapshot,
}

impl GtkHost {
    /// The host of the canvas in `widget`, telling the app through `events`.
    pub(crate) fn new(widget: &CanvasWidget, events: ComponentContext) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        WIDGETS.with_borrow_mut(|widgets| widgets.insert(id, widget.downgrade()));
        Self {
            widget: widget.downgrade(),
            id,
            events,
            size: Cell::new((0.0, 0.0)),
            scale: Cell::new(1.0),
            target: None,
            frame: gtk::Snapshot::new(),
        }
    }

    /// The canvas is `size` device pixels at `scale` device pixels per widget pixel; whether that changed.
    pub(crate) fn set_size(&self, size: (f32, f32), scale: f64) -> bool {
        let changed = self.size.get() != size || self.scale.get() != scale;
        self.size.set(size);
        self.scale.set(scale);
        changed
    }

    /// Device pixels per widget pixel.
    pub(crate) fn scale(&self) -> f64 {
        self.scale.get()
    }

    /// Lends the widget's `snapshot` for a frame (`None`: the frame is over).
    pub(crate) fn lend(&mut self, snapshot: Option<&gtk::Snapshot>) {
        self.target = snapshot.cloned();
    }

    /// Draws `draw` over the frame so far with darkest-wins blending: GSK blends a node with the one under it, so the
    /// frame so far becomes the blend's bottom.
    fn darkest(&mut self, draw: impl FnOnce(&gtk::Snapshot)) {
        let below = std::mem::replace(&mut self.frame, gtk::Snapshot::new()).to_node();
        let frame = &self.frame;
        frame.push_blend(gsk::BlendMode::Darken);
        if let Some(below) = below {
            frame.append_node(&below);
        }
        frame.pop();
        draw(frame);
        frame.pop();
    }

    /// Lays `draw`'s fills in `rgb` as `wash` says.
    fn wash(&mut self, rgb: u32, wash: Wash, draw: impl FnOnce(&gtk::Snapshot, &gdk::RGBA)) {
        match wash {
            Wash::Darkest => {
                let color = rgba(rgb, 1.0);
                self.darkest(|frame| draw(frame, &color));
            }
            Wash::Translucent => draw(&self.frame, &rgba(rgb, WASH_OPACITY)),
        }
    }
}

impl Drop for GtkHost {
    fn drop(&mut self) {
        let id = self.id;
        // The thread-local is gone when the UI thread ends; there is nothing to forget then.
        let _ = WIDGETS.try_with(|widgets| widgets.borrow_mut().remove(&id));
    }
}

/// Wakes the canvas `id` after a worker finished: its jobs are taken on the UI thread.
fn jobs_done(id: u64) {
    let Some(widget) =
        WIDGETS.with_borrow(|widgets| widgets.get(&id).and_then(glib::WeakRef::upgrade))
    else {
        return;
    };
    if !widget.jobs_done() {
        // The canvas is busy further up this thread's stack: try again once it is done.
        glib::idle_add_local_once(move || jobs_done(id));
    }
}

impl Host for GtkHost {
    type Image = gdk::Texture;
    type Error = Infallible;

    fn size(&self) -> (f32, f32) {
        self.size.get()
    }

    fn invalidate(&self) {
        if let Some(widget) = self.widget.upgrade() {
            widget.queue_draw();
        }
    }

    fn emit(&self, name: &str, data: JsonValue) {
        self.events.emit(name, data);
    }

    fn waker(&self) -> Wake {
        let id = self.id;
        // The default main context is the UI thread's: GTK runs it.
        Box::new(move || glib::MainContext::default().invoke(move || jobs_done(id)))
    }

    fn upload(&mut self, pixmap: &Pixmap) -> Option<(gdk::Texture, usize)> {
        let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let (n, stride, samples) = (pixmap.components(), pixmap.stride(), pixmap.samples());
        let bytes = if n == 3 {
            glib::Bytes::from(&samples[..stride * height])
        } else {
            // Not RGB: keep the first three channels, as Windows does.
            let mut rgb = Vec::with_capacity(width * height * 3);
            for row in samples.chunks_exact(stride).take(height) {
                for px in row[..width * n].chunks_exact(n) {
                    rgb.extend_from_slice(&px[..3]);
                }
            }
            glib::Bytes::from_owned(rgb)
        };
        let stride = if n == 3 { stride } else { width * 3 };
        let size = bytes.len();
        let texture = gdk::MemoryTexture::new(
            i32::try_from(width).ok()?,
            i32::try_from(height).ok()?,
            gdk::MemoryFormat::R8g8b8,
            &bytes,
            stride,
        );
        Some((texture.upcast(), size))
    }

    fn begin(&mut self, _width: f32, _height: f32) -> Result<(), Infallible> {
        self.frame = gtk::Snapshot::new();
        Ok(())
    }

    fn end(&mut self) -> Result<(), Infallible> {
        let frame = std::mem::replace(&mut self.frame, gtk::Snapshot::new());
        match (&self.target, frame.to_node()) {
            (Some(target), Some(node)) => {
                // The frame is in device pixels; the widget draws in its own.
                let scale = (1.0 / self.scale.get()) as f32;
                target.save();
                target.scale(scale, scale);
                target.append_node(&node);
                target.restore();
            }
            // Drawn outside the widget's snapshot: draw it again there.
            (None, _) => self.invalidate(),
            (Some(_), None) => {}
        }
        Ok(())
    }

    fn clear(&mut self, rgb: u32) {
        let (width, height) = self.size.get();
        self.frame.append_color(
            &rgba(rgb, 1.0),
            &graphene::Rect::new(0.0, 0.0, width, height),
        );
    }

    fn fill(&mut self, rect: &Rect, rgb: u32) -> Result<(), Infallible> {
        self.frame.append_color(&rgba(rgb, 1.0), &bounds(rect));
        Ok(())
    }

    fn image(&mut self, image: &gdk::Texture, dest: &Rect) {
        self.frame
            .append_scaled_texture(image, gsk::ScalingFilter::Linear, &bounds(dest));
    }

    fn highlight(&mut self, quads: &[Quad], rgb: u32, wash: Wash) -> Result<(), Infallible> {
        if quads.is_empty() {
            return Ok(());
        }
        let path = gsk::PathBuilder::new();
        for quad in quads {
            let [first, rest @ ..] = quad;
            path.move_to(first.0, first.1);
            for (x, y) in rest {
                path.line_to(*x, *y);
            }
            path.close();
        }
        let path = path.to_path();
        self.wash(rgb, wash, |frame, color| {
            frame.append_fill(&path, gsk::FillRule::Winding, color);
        });
        Ok(())
    }

    fn band(&mut self, rect: &Rect, radius: f32, rgb: u32, wash: Wash) -> Result<(), Infallible> {
        let path = gsk::PathBuilder::new();
        path.add_rounded_rect(&gsk::RoundedRect::from_rect(bounds(rect), radius));
        let path = path.to_path();
        self.wash(rgb, wash, |frame, color| {
            frame.append_fill(&path, gsk::FillRule::Winding, color);
        });
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
        let path = gsk::PathBuilder::new();
        path.add_rect(&bounds(screen));
        path.add_rounded_rect(&gsk::RoundedRect::from_rect(bounds(hole), radius));
        // Even-odd filling leaves the hole, inside the screen, empty.
        self.frame.append_fill(
            &path.to_path(),
            gsk::FillRule::EvenOdd,
            &rgba(rgb, alpha.clamp(0.0, 1.0)),
        );
        Ok(())
    }

    // shortcut: one stroke node a segment (each as wide as the mean of its ends); merge runs of one width if long
    // strokes show in K.24.
    fn polyline(&mut self, points: &[InkPixel], rgb: u32) -> Result<(), Infallible> {
        let color = rgba(rgb, 1.0);
        if let [(x, y, width)] = points {
            let path = gsk::PathBuilder::new();
            path.add_circle(&graphene::Point::new(*x, *y), width / 2.0);
            self.frame
                .append_fill(&path.to_path(), gsk::FillRule::Winding, &color);
            return Ok(());
        }
        for pair in points.windows(2) {
            let ((x0, y0, w0), (x1, y1, w1)) = (pair[0], pair[1]);
            let path = gsk::PathBuilder::new();
            path.move_to(x0, y0);
            path.line_to(x1, y1);
            let stroke = gsk::Stroke::new((w0 + w1) / 2.0);
            stroke.set_line_cap(gsk::LineCap::Round);
            stroke.set_line_join(gsk::LineJoin::Round);
            self.frame.append_stroke(&path.to_path(), &stroke, &color);
        }
        Ok(())
    }

    fn frame_ready(&mut self) -> bool {
        self.target.is_some()
    }

    fn content_changed(&self) {
        // Later: GTK reads the text back at once, and the canvas is busy now.
        let widget = self.widget.clone();
        glib::idle_add_local_once(move || {
            if let Some(widget) = widget.upgrade() {
                widget.text_changed();
            }
        });
    }

    fn selection_changed(&self) {
        let widget = self.widget.clone();
        glib::idle_add_local_once(move || {
            if let Some(widget) = widget.upgrade() {
                widget.selection_moved();
            }
        });
    }
}

/// `0xRRGGBB` at `alpha`.
fn rgba(rgb: u32, alpha: f32) -> gdk::RGBA {
    let channel =
        |shift: u32| f32::from(u8::try_from(rgb >> shift & 0xff).expect("a byte")) / 255.0;
    gdk::RGBA::new(channel(16), channel(8), channel(0), alpha)
}

/// `rect` as GSK takes it.
fn bounds(rect: &Rect) -> graphene::Rect {
    graphene::Rect::new(rect.x0, rect.y0, rect.width(), rect.height())
}
