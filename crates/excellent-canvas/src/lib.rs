//! Platform-neutral page canvas: viewport, gestures, hit testing, overlays and the ink capture model (plan §7.2).
//!
//! Nothing here touches the OS, so each platform layer (`excellent-canvas-win`, `-gtk`, `-mac`) only presents pixels
//! and delivers input (plan D5, K.8–K.10). The continuous page [`layout`], the [`viewport`] with scroll and zoom, the
//! [`tiles`] the viewport needs and their cache, the reader's [`view`] (layout, rotation, page box, zoom mode), and
//! the [`page`] canvas itself: its document, input, drawing through a platform's [`page::Host`], and the
//! component's props and commands.

pub mod layout;
pub mod page;
pub mod tiles;
pub mod view;
pub mod viewport;

/// A rectangle in floating-point units (page points or device pixels, as the field using it says).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

impl Rect {
    /// The rectangle from `(x, y)` with this width and height.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x + width,
            y1: y + height,
        }
    }

    /// Its width.
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    /// Its height.
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    /// The part both rectangles cover, if any.
    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let r = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        (r.x0 < r.x1 && r.y0 < r.y1).then_some(r)
    }
}
