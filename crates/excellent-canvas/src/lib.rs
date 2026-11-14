//! Platform-neutral page canvas: viewport, gestures, hit testing, overlays and the ink capture model (plan §7.2).
//!
//! Everything here is pure Rust with no OS or MuPDF types, so each platform layer (`excellent-canvas-win` and the
//! others) only presents pixels and delivers input. Built so far (T7): the continuous page [`layout`], the
//! [`viewport`] with scroll and zoom, and the [`tiles`] the viewport needs and their cache.

pub mod layout;
pub mod tiles;
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
