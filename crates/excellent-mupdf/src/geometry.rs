//! Typed geometry: page space (points, y down, as MuPDF reports it) and device space (pixels) are different types,
//! so one can't be passed for the other. Building the page-to-device [`Matrix`] from zoom and rotation happens in one
//! place, `excellent-core::transform`.

use excellent_mupdf_sys::{fz_irect, fz_matrix, fz_point, fz_quad, fz_rect};

/// A point on a page, in points (1/72 inch).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PagePoint {
    /// Rightwards.
    pub x: f32,
    /// Downwards.
    pub y: f32,
}

/// Which of a PDF page's boxes is shown (plan R.4). Other formats have one box, which every choice gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PageBox {
    /// The whole medium the page is printed on.
    Media,
    /// The visible page, as a reader shows it.
    #[default]
    Crop,
    /// The page with the bleed printers need around it.
    Bleed,
    /// The finished page after trimming.
    Trim,
    /// The page's meaningful content, as its author marked it.
    Art,
}

impl PageBox {
    /// MuPDF's `fz_box_type` for it.
    pub(crate) fn raw(self) -> core::ffi::c_int {
        match self {
            Self::Media => 0,
            Self::Crop => 1,
            Self::Bleed => 2,
            Self::Trim => 3,
            Self::Art => 4,
        }
    }
}

/// A point on the screen or a pixmap, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DevicePoint {
    /// Rightwards.
    pub x: f32,
    /// Downwards.
    pub y: f32,
}

/// A rectangle on a page, in points, from its top-left `(x0, y0)` to its bottom-right `(x1, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageRect {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

impl PageRect {
    /// Width in points.
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    /// Height in points.
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
}

/// Four corners on a page, in points: a run of text or a markup area, which may be rotated or skewed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Quad {
    /// Upper left, as the text reads.
    pub ul: PagePoint,
    /// Upper right.
    pub ur: PagePoint,
    /// Lower left.
    pub ll: PagePoint,
    /// Lower right.
    pub lr: PagePoint,
}

impl Quad {
    /// The smallest page rectangle holding all four corners.
    pub fn bounds(&self) -> PageRect {
        let corners = [self.ul, self.ur, self.ll, self.lr];
        let fold = |f: fn(f32, f32) -> f32, start: f32, get: fn(&PagePoint) -> f32| {
            corners.iter().map(get).fold(start, f)
        };
        PageRect {
            x0: fold(f32::min, f32::MAX, |p| p.x),
            y0: fold(f32::min, f32::MAX, |p| p.y),
            x1: fold(f32::max, f32::MIN, |p| p.x),
            y1: fold(f32::max, f32::MIN, |p| p.y),
        }
    }
}

/// A rectangle of whole pixels, from `(x0, y0)` inclusive to `(x1, y1)` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeviceRect {
    /// Left edge.
    pub x0: i32,
    /// Top edge.
    pub y0: i32,
    /// Right edge (exclusive).
    pub x1: i32,
    /// Bottom edge (exclusive).
    pub y1: i32,
}

impl DeviceRect {
    /// Width in pixels (0 when empty).
    pub fn width(&self) -> u32 {
        u32::try_from(self.x1 - self.x0).unwrap_or(0)
    }

    /// Height in pixels (0 when empty).
    pub fn height(&self) -> u32 {
        u32::try_from(self.y1 - self.y0).unwrap_or(0)
    }

    /// True when the rectangle holds no pixel.
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
}

/// An affine map from page space to device space: `x' = a×x + c×y + e`, `y' = b×x + d×y + f` (MuPDF's `fz_matrix`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    /// Row 1, column 1.
    pub a: f32,
    /// Row 1, column 2.
    pub b: f32,
    /// Row 2, column 1.
    pub c: f32,
    /// Row 2, column 2.
    pub d: f32,
    /// Horizontal translation.
    pub e: f32,
    /// Vertical translation.
    pub f: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    /// One point to one pixel.
    pub const IDENTITY: Self = Self::scale(1.0, 1.0);

    /// Scales by `sx` horizontally and `sy` vertically.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Moves by `(tx, ty)`.
    pub const fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// Rotates clockwise (on a y-down page) by a multiple of 90 degrees, exactly.
    pub const fn rotate_quarter_turns(turns: u8) -> Self {
        let (sin, cos) = match turns % 4 {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        };
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// This map followed by `next` (MuPDF's `fz_concat(self, next)`).
    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            e: self.e * next.a + self.f * next.c + next.e,
            f: self.e * next.b + self.f * next.d + next.f,
        }
    }

    /// Where a page point lands on the device.
    pub fn apply(&self, p: PagePoint) -> DevicePoint {
        DevicePoint {
            x: p.x * self.a + p.y * self.c + self.e,
            y: p.x * self.b + p.y * self.d + self.f,
        }
    }

    /// The pixels a page rectangle covers on the device: the bounding box of its four corners, rounded outwards
    /// the way MuPDF's `fz_round_rect` does.
    pub fn apply_rect(&self, r: &PageRect) -> DeviceRect {
        let corners = [
            self.apply(PagePoint { x: r.x0, y: r.y0 }),
            self.apply(PagePoint { x: r.x1, y: r.y0 }),
            self.apply(PagePoint { x: r.x0, y: r.y1 }),
            self.apply(PagePoint { x: r.x1, y: r.y1 }),
        ];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in corners {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        // `as` saturates, which is what we want for absurd sizes; the 0.001 matches fz_round_rect.
        DeviceRect {
            x0: (x0 + 0.001).floor() as i32,
            y0: (y0 + 0.001).floor() as i32,
            x1: (x1 - 0.001).ceil() as i32,
            y1: (y1 - 0.001).ceil() as i32,
        }
    }
}

impl From<fz_rect> for PageRect {
    fn from(r: fz_rect) -> Self {
        Self {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }
    }
}

impl From<PageRect> for fz_rect {
    fn from(r: PageRect) -> Self {
        Self {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }
    }
}

impl From<fz_point> for PagePoint {
    fn from(p: fz_point) -> Self {
        Self { x: p.x, y: p.y }
    }
}

impl From<PagePoint> for fz_point {
    fn from(p: PagePoint) -> Self {
        Self { x: p.x, y: p.y }
    }
}

impl From<fz_quad> for Quad {
    fn from(q: fz_quad) -> Self {
        Self {
            ul: q.ul.into(),
            ur: q.ur.into(),
            ll: q.ll.into(),
            lr: q.lr.into(),
        }
    }
}

impl From<Quad> for fz_quad {
    fn from(q: Quad) -> Self {
        Self {
            ul: q.ul.into(),
            ur: q.ur.into(),
            ll: q.ll.into(),
            lr: q.lr.into(),
        }
    }
}

impl From<Matrix> for fz_matrix {
    fn from(m: Matrix) -> Self {
        Self {
            a: m.a,
            b: m.b,
            c: m.c,
            d: m.d,
            e: m.e,
            f: m.f,
        }
    }
}

impl From<DeviceRect> for fz_irect {
    fn from(r: DeviceRect) -> Self {
        Self {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    #[test]
    fn scale_rounds_outwards() {
        let r = Matrix::scale(1.5, 1.5).apply_rect(&LETTER);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0, 0, 918, 1188));
        let half = Matrix::scale(0.5, 0.5).apply_rect(&PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 3.0,
            y1: 3.0,
        });
        assert_eq!((half.x1, half.y1), (2, 2), "1.5 px covers two pixels");
    }

    #[test]
    fn quarter_turn_then_translate_keeps_the_page_on_screen() {
        // Rotating a portrait page 90° clockwise puts it at negative x; moving it by its height brings it back.
        let m = Matrix::rotate_quarter_turns(1).then(&Matrix::translate(792.0, 0.0));
        let r = m.apply_rect(&LETTER);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0, 0, 792, 612));
        let top_left = m.apply(PagePoint { x: 0.0, y: 0.0 });
        assert_eq!((top_left.x, top_left.y), (792.0, 0.0));
    }

    #[test]
    fn then_matches_applying_in_turn() {
        let a = Matrix::scale(2.0, 3.0);
        let b = Matrix::rotate_quarter_turns(3).then(&Matrix::translate(5.0, 7.0));
        let p = PagePoint { x: 11.0, y: 13.0 };
        let step = a.apply(p);
        let expected = b.apply(PagePoint {
            x: step.x,
            y: step.y,
        });
        assert_eq!(a.then(&b).apply(p), expected);
    }

    #[test]
    fn quad_bounds_hold_a_skewed_quad() {
        let q = Quad {
            ul: PagePoint { x: 10.0, y: 5.0 },
            ur: PagePoint { x: 50.0, y: 1.0 },
            ll: PagePoint { x: 12.0, y: 20.0 },
            lr: PagePoint { x: 52.0, y: 16.0 },
        };
        assert_eq!(
            q.bounds(),
            PageRect {
                x0: 10.0,
                y0: 1.0,
                x1: 52.0,
                y1: 20.0
            }
        );
    }

    #[test]
    fn empty_device_rect() {
        let r = DeviceRect {
            x0: 5,
            y0: 5,
            x1: 5,
            y1: 9,
        };
        assert!(r.is_empty());
        assert_eq!((r.width(), r.height()), (0, 4));
    }
}
