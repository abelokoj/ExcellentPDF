//! The zoom writing pad (H.12, plan §8.4): a magnified strip at the bottom of the pages where the reader writes large,
//! and the writing lands small in a box on the page. The box moves itself: when the writing nears the pad's right
//! edge the box steps along the line, and at the end of the line it goes to the start of the next, so a paragraph
//! needs no moves by hand. Pure: pad points in, page points out.

use crate::erase::Point;

/// How much larger the pad shows the page than it is.
pub const MAGNIFY: f32 = 3.0;

/// Where in the pad, as a share of its width, the writing makes the box step on.
const ADVANCE_AT: f32 = 0.75;

/// How far the box steps, as a share of its width: the writing near the edge comes back to the pad's middle.
const STEP: f32 = 0.5;

/// The space between lines, as a share of the box's height.
const LINE_GAP: f32 = 0.25;

/// The pad and the box on the page it writes into.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoomPad {
    /// The page's writing area: left and right margins, and the bottom the box mustn't pass.
    margins: (f32, f32, f32),
    /// The box's top-left corner, page points.
    corner: Point,
    /// The box's size, page points: the pad's size over [`MAGNIFY`].
    size: (f32, f32),
}

impl ZoomPad {
    /// A pad `pad_size` pixels across and high, writing into a box at `corner` on a page whose writing area runs from
    /// `left` to `right` across and down to `bottom` (page points), `pixels_per_point` the view's scale.
    #[must_use]
    pub fn new(
        pad_size: (f32, f32),
        pixels_per_point: f32,
        corner: Point,
        (left, right, bottom): (f32, f32, f32),
    ) -> Self {
        let scale = (pixels_per_point * MAGNIFY).max(f32::EPSILON);
        Self {
            margins: (left, right, bottom),
            corner,
            size: (pad_size.0 / scale, pad_size.1 / scale),
        }
    }

    /// The box on the page, page points: left, top, right, bottom.
    #[must_use]
    pub fn target(&self) -> (f32, f32, f32, f32) {
        (
            self.corner.x,
            self.corner.y,
            self.corner.x + self.size.0,
            self.corner.y + self.size.1,
        )
    }

    /// `point` in the pad (pixels from its top-left corner) on the page.
    #[must_use]
    pub fn to_page(&self, point: Point, pad_size: (f32, f32)) -> Point {
        Point {
            x: self.corner.x + point.x / pad_size.0 * self.size.0,
            y: self.corner.y + point.y / pad_size.1 * self.size.1,
        }
    }

    /// A stroke written in the pad was finished, reaching `right_most` pixels across it: when that is near the pad's
    /// right edge, the box steps along the line, or to the next line at its end. Returns whether it moved.
    pub fn stroke_done(&mut self, right_most: f32, pad_width: f32) -> bool {
        if right_most < pad_width * ADVANCE_AT {
            return false;
        }
        let (left, right, bottom) = self.margins;
        let next_x = self.corner.x + self.size.0 * STEP;
        if next_x + self.size.0 * ADVANCE_AT <= right {
            self.corner.x = next_x;
        } else {
            self.next_line(left, bottom);
        }
        true
    }

    /// Goes to the start of the next line (the reader's "new line", or the end of a line).
    pub fn new_line(&mut self) {
        let (left, _, bottom) = self.margins;
        self.next_line(left, bottom);
    }

    fn next_line(&mut self, left: f32, bottom: f32) {
        let down = self.corner.y + self.size.1 * (1.0 + LINE_GAP);
        self.corner.x = left;
        // At the foot of the page the box stays on its last line: the reader turns the page.
        if down + self.size.1 <= bottom {
            self.corner.y = down;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_a_paragraph_needs_no_manual_box_moves() {
        // A Letter page with one-inch margins, a 900 by 240 pixel pad at 100 % (one pixel a point).
        let pad = (900.0, 240.0);
        let mut zoom = ZoomPad::new(pad, 1.0, Point { x: 72.0, y: 100.0 }, (72.0, 540.0, 720.0));
        let mut landed: Vec<(f32, f32, f32, f32)> = Vec::new();
        let mut lines = 1;
        // A paragraph: words written left to right in the pad, each about 140 pixels wide with a gap after it; the
        // writer always carries on from where the pad shows the writing ending.
        let mut pen_x = 40.0;
        for _word in 0..25 {
            let (start, end) = (pen_x, pen_x + 140.0);
            let a = zoom.to_page(Point { x: start, y: 120.0 }, pad);
            let b = zoom.to_page(Point { x: end, y: 180.0 }, pad);
            landed.push((a.x, a.y, b.x, b.y));
            let before = zoom.target();
            pen_x = end + 30.0;
            if zoom.stroke_done(end, pad.0) {
                let after = zoom.target();
                if after.1 > before.1 {
                    lines += 1;
                    pen_x = 40.0;
                } else {
                    // The box stepped half its width: the writing ends that much further left in the pad.
                    pen_x -= pad.0 * STEP;
                }
            }
        }
        assert!(
            lines >= 3,
            "the paragraph ran onto new lines by itself ({lines})"
        );
        for (x0, y0, x1, y1) in &landed {
            assert!(
                *x0 >= 72.0 - 0.01 && *x1 <= 540.0 + 0.01,
                "inside the margins: {x0}..{x1}"
            );
            assert!(*y1 <= 720.0, "above the foot of the page: {y1}");
            assert!(x1 > x0 && y1 > y0);
        }
        // Words on one line never overlap: each starts at or after the previous one's end.
        for pair in landed.windows(2) {
            let (prev, next) = (pair[0], pair[1]);
            if (next.1 - prev.1).abs() < 0.01 {
                assert!(next.0 >= prev.2 - 0.01, "{prev:?} then {next:?}");
            }
        }
    }

    #[test]
    fn the_pad_shows_the_page_magnified() {
        let pad = (900.0, 240.0);
        let zoom = ZoomPad::new(pad, 1.0, Point { x: 72.0, y: 100.0 }, (72.0, 540.0, 720.0));
        let (x0, y0, x1, y1) = zoom.target();
        assert!(((x1 - x0) - 300.0).abs() < 1e-3 && ((y1 - y0) - 80.0).abs() < 1e-3);
        let corner = zoom.to_page(Point { x: 900.0, y: 240.0 }, pad);
        assert!((corner.x - x1).abs() < 1e-3 && (corner.y - y1).abs() < 1e-3);
    }

    #[test]
    fn the_box_stays_on_the_page_at_its_foot() {
        let pad = (900.0, 240.0);
        let mut zoom = ZoomPad::new(pad, 1.0, Point { x: 72.0, y: 600.0 }, (72.0, 540.0, 720.0));
        zoom.new_line();
        let (_, _, _, bottom) = zoom.target();
        assert!(bottom <= 720.0, "{bottom}");
    }
}
