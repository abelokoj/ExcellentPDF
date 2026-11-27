//! The one place where page geometry becomes device geometry (plan §6.3, AGENTS.md §6).
//!
//! MuPDF already applies a page's crop box and its own `/Rotate` in [`ecclesia_mupdf::Page::bounds`]. On top of
//! that, the view adds zoom and the user's rotation; [`Transform`] turns both into the page-to-device [`Matrix`],
//! with the page's top-left corner, as the user sees it, at device `(0, 0)`.

use ecclesia_mupdf::{DevicePoint, DeviceRect, Matrix, PagePoint, PageRect};

/// The user's view rotation, clockwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Rotation {
    /// Upright.
    #[default]
    Upright,
    /// A quarter turn clockwise.
    Clockwise,
    /// Upside down.
    UpsideDown,
    /// A quarter turn anticlockwise.
    Anticlockwise,
}

impl Rotation {
    fn quarter_turns(self) -> u8 {
        match self {
            Self::Upright => 0,
            Self::Clockwise => 1,
            Self::UpsideDown => 2,
            Self::Anticlockwise => 3,
        }
    }

    fn from_quarter_turns(turns: u8) -> Self {
        match turns % 4 {
            0 => Self::Upright,
            1 => Self::Clockwise,
            2 => Self::UpsideDown,
            _ => Self::Anticlockwise,
        }
    }

    /// A quarter turn further clockwise.
    #[must_use]
    pub fn turned_clockwise(self) -> Self {
        Self::from_quarter_turns(self.quarter_turns() + 1)
    }

    /// A quarter turn further anticlockwise.
    #[must_use]
    pub fn turned_anticlockwise(self) -> Self {
        Self::from_quarter_turns(self.quarter_turns() + 3)
    }

    /// Whether the page lies on its side: width and height swap.
    pub fn is_sideways(self) -> bool {
        self.quarter_turns() % 2 == 1
    }
}

/// How a page is shown: its zoom and the user's rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Device pixels per page point (1.0 is 72 dpi, before the screen's scale factor).
    pub zoom: f32,
    /// The user's rotation.
    pub rotation: Rotation,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            rotation: Rotation::Upright,
        }
    }
}

impl Transform {
    /// The page-to-device matrix for a page with these `bounds`.
    pub fn matrix(&self, bounds: &PageRect) -> Matrix {
        let turned = Matrix::translate(-bounds.x0, -bounds.y0)
            .then(&Matrix::scale(self.zoom, self.zoom))
            .then(&Matrix::rotate_quarter_turns(self.rotation.quarter_turns()));
        // Rotation swings the page into negative coordinates; move its new top-left corner back to the origin.
        let corners = [
            (bounds.x0, bounds.y0),
            (bounds.x1, bounds.y0),
            (bounds.x0, bounds.y1),
            (bounds.x1, bounds.y1),
        ]
        .map(|(x, y)| turned.apply(PagePoint { x, y }));
        let min_x = corners.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let min_y = corners.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        turned.then(&Matrix::translate(-min_x, -min_y))
    }

    /// The page's size on the device, unrounded: its bounds' width and height at this zoom, swapped when the
    /// rotation lays it on its side.
    pub fn shown_size(&self, bounds: &PageRect) -> (f32, f32) {
        let (w, h) = (bounds.width() * self.zoom, bounds.height() * self.zoom);
        if self.rotation.is_sideways() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// The page point at device point `p`: the inverse of [`Self::matrix`], for pointer and pen input.
    pub fn to_page(&self, bounds: &PageRect, p: DevicePoint) -> PagePoint {
        let m = self.matrix(bounds);
        // The matrix is a scale, a quarter turn and a move: never singular while the zoom isn't zero.
        let det = m.a * m.d - m.b * m.c;
        let (a, b, c, d) = (m.d / det, -m.b / det, -m.c / det, m.a / det);
        let (e, f) = (-(m.e * a + m.f * c), -(m.e * b + m.f * d));
        PagePoint {
            x: p.x * a + p.y * c + e,
            y: p.x * b + p.y * d + f,
        }
    }

    /// The pixels the whole page covers: from `(0, 0)` to its width and height on the device.
    pub fn device_rect(&self, bounds: &PageRect) -> DeviceRect {
        self.matrix(bounds).apply_rect(bounds)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const LETTER: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    fn rect(t: Transform, bounds: &PageRect) -> (i32, i32, i32, i32) {
        let r = t.device_rect(bounds);
        (r.x0, r.y0, r.x1, r.y1)
    }

    #[test]
    fn every_rotation_starts_at_the_origin() {
        for (rotation, size) in [
            (Rotation::Upright, (0, 0, 918, 1188)),
            (Rotation::Clockwise, (0, 0, 1188, 918)),
            (Rotation::UpsideDown, (0, 0, 918, 1188)),
            (Rotation::Anticlockwise, (0, 0, 1188, 918)),
        ] {
            let t = Transform {
                zoom: 1.5,
                rotation,
            };
            assert_eq!(rect(t, &LETTER), size, "{rotation:?}");
        }
    }

    #[test]
    fn clockwise_puts_the_top_left_corner_top_right() {
        let t = Transform {
            zoom: 1.0,
            rotation: Rotation::Clockwise,
        };
        let p = t.matrix(&LETTER).apply(PagePoint { x: 0.0, y: 0.0 });
        assert_eq!((p.x, p.y), (792.0, 0.0));
    }

    #[test]
    fn a_crop_box_off_the_origin_is_moved_to_it() {
        let cropped = PageRect {
            x0: 100.0,
            y0: 50.0,
            x1: 300.0,
            y1: 450.0,
        };
        assert_eq!(
            rect(Transform::default(), &cropped),
            (0, 0, 200, 400),
            "the crop box's corner lands on the origin"
        );
    }

    fn rotation() -> impl Strategy<Value = Rotation> {
        (0u8..4).prop_map(Rotation::from_quarter_turns)
    }

    proptest! {
        /// K.6 item 4 and R.4's "Done when": page → device → page comes back for every zoom, rotation and crop box,
        /// so annotations and selection land where they were drawn.
        #[test]
        fn page_to_device_and_back(
            zoom in 0.08f32..64.0,
            rotation in rotation(),
            (x0, y0) in (-500.0f32..500.0, -500.0f32..500.0),
            (w, h) in (10.0f32..2000.0, 10.0f32..2000.0),
            (fx, fy) in (0.0f32..1.0, 0.0f32..1.0),
        ) {
            let bounds = PageRect { x0, y0, x1: x0 + w, y1: y0 + h };
            let t = Transform { zoom, rotation };
            let p = PagePoint { x: x0 + fx * w, y: y0 + fy * h };
            let back = t.to_page(&bounds, t.matrix(&bounds).apply(p));
            let tolerance = 1e-3 * (w.max(h) + x0.abs() + y0.abs());
            prop_assert!((back.x - p.x).abs() < tolerance && (back.y - p.y).abs() < tolerance,
                "{:?} came back as {:?}", p, back);
            // And the page's points stay inside its shown size.
            let (sw, sh) = t.shown_size(&bounds);
            let d = t.matrix(&bounds).apply(p);
            prop_assert!(d.x >= -0.01 * zoom && d.x <= sw + 0.01 * sw && d.y >= -0.01 * zoom && d.y <= sh + 0.01 * sh,
                "{:?} outside {} x {}", d, sw, sh);
        }
    }

    #[test]
    fn four_quarter_turns_come_back_upright() {
        let mut r = Rotation::Upright;
        for _ in 0..4 {
            r = r.turned_clockwise();
        }
        assert_eq!(r, Rotation::Upright);
        assert_eq!(
            Rotation::Upright.turned_anticlockwise(),
            Rotation::Anticlockwise
        );
        assert!(Rotation::Clockwise.is_sideways() && !Rotation::UpsideDown.is_sideways());
    }
}
