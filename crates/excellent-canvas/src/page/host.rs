//! What a platform gives the page canvas (plan D5): a surface to draw on and the services around it. The canvas's
//! logic ([`super::PageCanvas`]) is the same on every platform; each one (`excellent-canvas-win`, `-gtk`, `-mac`)
//! implements [`Host`] for its view and forwards its input to the canvas.

use std::fmt;

use excellent_mupdf::Pixmap;
use nsh_core::JsonValue;

use crate::Rect;

/// A quadrilateral on screen: its corners in pixels, in order around it.
pub type Quad = [(f32, f32); 4];

/// A point of a stroke on screen: `(x, y, width)`, all in pixels.
pub type InkPixel = (f32, f32, f32);

/// How a highlight or the reading ruler's band lies over the pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wash {
    /// Darkest wins: on light paper the colour tints the paper and dark text stays as it is.
    Darkest,
    /// Laid over at [`WASH_OPACITY`]: on dark paper (R.14), where darkest-wins would leave the paper as it is.
    Translucent,
}

/// How much of a translucent wash shows: enough to see on dark paper, little enough to keep its light text readable.
pub const WASH_OPACITY: f32 = 0.3;

/// Wakes the canvas's thread from a worker: the host posts a message, schedules an idle callback or the like, and
/// then calls [`super::PageCanvas::jobs_done`] on its own thread. It is called at most once per batch of jobs.
pub type Wake = Box<dyn Fn() + Send + Sync>;

/// A platform's view of a page canvas. Colours are `0xRRGGBB`; positions and sizes are the view's pixels, from its
/// top left.
///
/// A frame is drawn between [`Host::begin`] and [`Host::end`], with the drawing calls in between. A drawing call
/// that fails ends the frame: the canvas then calls [`Host::lose_device`], forgets its images and asks for another
/// frame.
pub trait Host {
    /// A tile uploaded for drawing.
    type Image;
    /// Why drawing failed.
    type Error: fmt::Display;

    /// The view's size in pixels.
    fn size(&self) -> (f32, f32);

    /// Asks for a frame: the host calls [`super::PageCanvas::paint`] soon, on its thread.
    fn invalidate(&self);

    /// Tells the app something happened (the component's events: `pages`, `page`, `view`, `copy` and the rest).
    fn emit(&self, name: &str, data: JsonValue);

    /// Makes what wakes the canvas when a worker finishes.
    fn waker(&self) -> Wake;

    /// Uploads a rendered tile (RGB, `pixmap.components()` bytes a pixel), with its size in bytes; `None` (logged)
    /// when it can't. Called outside a frame, only while the host can draw.
    fn upload(&mut self, pixmap: &Pixmap) -> Option<(Self::Image, usize)>;

    /// Whether the host can upload images now: it has a device, or needs none.
    fn can_upload(&self) -> bool {
        true
    }

    /// Starts a frame of `width` × `height` pixels, making the device first if there is none.
    ///
    /// # Errors
    ///
    /// The device can't be made or the frame can't start.
    fn begin(&mut self, width: f32, height: f32) -> Result<(), Self::Error>;

    /// Ends the frame and shows it.
    ///
    /// # Errors
    ///
    /// The frame couldn't be shown: the device is lost.
    fn end(&mut self) -> Result<(), Self::Error>;

    /// The view changed size; the next frame has it.
    ///
    /// # Errors
    ///
    /// The device couldn't follow: it is lost.
    fn resize(&mut self, _width: u32, _height: u32) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Drops the device and every image made with it; the next frame makes a new one.
    fn lose_device(&mut self) {}

    /// Fills the whole frame with `rgb`.
    fn clear(&mut self, rgb: u32);

    /// Fills `rect` with `rgb`.
    ///
    /// # Errors
    ///
    /// Drawing failed.
    fn fill(&mut self, rect: &Rect, rgb: u32) -> Result<(), Self::Error>;

    /// Draws `image` stretched over `dest`.
    fn image(&mut self, image: &Self::Image, dest: &Rect);

    /// Fills `quads` with `rgb` the way a highlighter marks paper, laid as `wash` says (with [`Wash::Darkest`],
    /// each pixel keeps the darker of itself and `rgb`, channel by channel).
    ///
    /// # Errors
    ///
    /// Drawing failed.
    fn highlight(&mut self, quads: &[Quad], rgb: u32, wash: Wash) -> Result<(), Self::Error>;

    /// Fills `rect` with its corners rounded by `radius` pixels, laid as `wash` says: the reading ruler's band.
    ///
    /// # Errors
    ///
    /// Drawing failed.
    fn band(&mut self, rect: &Rect, radius: f32, rgb: u32, wash: Wash) -> Result<(), Self::Error>;

    /// Lays `rgb` at `alpha` (0 to 1) over all of `screen` but `hole`, whose corners are rounded by `radius` pixels.
    ///
    /// # Errors
    ///
    /// Drawing failed.
    fn dim_outside(
        &mut self,
        screen: &Rect,
        hole: &Rect,
        radius: f32,
        rgb: u32,
        alpha: f32,
    ) -> Result<(), Self::Error>;

    /// Draws a stroke through `points`, each as wide as it says, with round joins and ends.
    ///
    /// # Errors
    ///
    /// Drawing failed.
    fn polyline(&mut self, points: &[InkPixel], rgb: u32) -> Result<(), Self::Error>;

    /// The text, links or notes screen readers read changed.
    fn content_changed(&self) {}

    /// The selected text changed.
    fn selection_changed(&self) {}

    /// The system draws a stroke's newest points ahead of the app's frames (Windows' ink trail), in `rgb`.
    fn trail_start(&mut self, _rgb: u32) {}

    /// Hands `points` to that trail; a token to take them off it again, or `None` without a trail.
    fn trail_add(&mut self, _points: &[InkPixel]) -> Option<u32> {
        None
    }

    /// Takes the points of `token` off the trail: the app's frame shows them now.
    fn trail_remove(&mut self, _token: u32) {}

    /// Whether a frame can be drawn now without waiting for the display.
    fn frame_ready(&mut self) -> bool {
        true
    }

    /// Milliseconds since input time stamp `input` (the host's clock, as its pen samples carry), for the ink latency
    /// log; `None` when the host can't tell.
    fn since_input_ms(&self, _input: u64) -> Option<f64> {
        None
    }
}
