//! Where a page is in layout space, and the mapping between its page points and layout points under the view
//! rotation, so input lands on the page and what is drawn on the page stays on it through any layout, zoom,
//! rotation or box (plan R.4). The rotation itself is `excellent-core::transform`'s.

use crate::Rect;
use crate::layout::Layout;
use excellent_core::transform::{Rotation, Transform};
use excellent_mupdf::{DevicePoint, Matrix, PagePoint, PageRect};

/// One page's place in the layout.
#[derive(Debug, Clone)]
pub(crate) struct Placement {
    /// The page's rectangle in layout points.
    rect: Rect,
    bounds: PageRect,
    transform: Transform,
    /// Page points to points from the page's shown top-left corner.
    matrix: Matrix,
}

impl Placement {
    /// The page with these `bounds` (page points), shown at `rect` in the layout with `rotation`.
    pub(crate) fn new(rect: Rect, bounds: PageRect, rotation: Rotation) -> Self {
        let transform = Transform {
            zoom: 1.0,
            rotation,
        };
        Self {
            rect,
            bounds,
            transform,
            matrix: transform.matrix(&bounds),
        }
    }

    /// Page `index` of `layout`, whose unrotated boxes are `bounds`; `None` past the last page.
    pub(crate) fn of_page(
        layout: &Layout,
        bounds: &[PageRect],
        rotation: Rotation,
        index: usize,
    ) -> Option<Self> {
        Some(Self::new(
            layout.page(index)?,
            *bounds.get(index)?,
            rotation,
        ))
    }

    /// Layout point of page point `p`.
    pub(crate) fn to_layout(&self, p: PagePoint) -> (f32, f32) {
        let shown = self.matrix.apply(p);
        (self.rect.x0 + shown.x, self.rect.y0 + shown.y)
    }

    /// Page point of layout point (`x`, `y`).
    pub(crate) fn to_page(&self, x: f32, y: f32) -> PagePoint {
        let shown = DevicePoint {
            x: x - self.rect.x0,
            y: y - self.rect.y0,
        };
        self.transform.to_page(&self.bounds, shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_comes_back_under_every_rotation() {
        let bounds = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        let mut rotation = Rotation::Upright;
        for _ in 0..4 {
            let (w, h) = Transform {
                zoom: 1.0,
                rotation,
            }
            .shown_size(&bounds);
            let placement = Placement::new(Rect::new(100.0, 2000.0, w, h), bounds, rotation);
            let p = PagePoint { x: 72.0, y: 144.0 };
            let (x, y) = placement.to_layout(p);
            assert!(x >= 100.0 && x <= 100.0 + w && y >= 2000.0 && y <= 2000.0 + h);
            let back = placement.to_page(x, y);
            assert!(
                (back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3,
                "{rotation:?}"
            );
            rotation = rotation.turned_clockwise();
        }
    }
}
