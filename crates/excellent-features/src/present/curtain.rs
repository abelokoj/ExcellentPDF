//! Presentation curtain reveal overlay (plan G.7).
//!
//! Covers a slide and slides down, up, left, or right as the presenter reveals content line by
//! line via keyboard steps or smooth pen/pointer dragging.

use excellent_mupdf::{PagePoint, PageRect};

/// Direction the curtain slides as content is revealed (G.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CurtainDirection {
    /// Reveals from top to bottom (curtain slides downward).
    #[default]
    TopToBottom,
    /// Reveals from bottom to top (curtain slides upward).
    BottomToTop,
    /// Reveals from left to right (curtain slides right).
    LeftToRight,
    /// Reveals from right to left (curtain slides left).
    RightToLeft,
}

/// Slide curtain overlay state controllable by keyboard and pen drag (plan G.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurtainReveal {
    /// Whether the curtain overlay is currently enabled.
    active: bool,
    /// Direction of reveal.
    pub direction: CurtainDirection,
    /// Fraction of the slide currently revealed (`0.0` = fully covered, `1.0` = fully revealed).
    revealed_fraction: f32,
    /// Fractional step size per keyboard line-reveal press (default `0.1` = 10% per step).
    pub step_fraction: f32,
}

impl Default for CurtainReveal {
    fn default() -> Self {
        Self {
            active: false,
            direction: CurtainDirection::TopToBottom,
            revealed_fraction: 0.0,
            step_fraction: 0.1,
        }
    }
}

impl CurtainReveal {
    /// Creates an enabled curtain starting fully covered (`0.0`) sliding in `direction`.
    #[must_use]
    pub fn new(direction: CurtainDirection) -> Self {
        Self {
            active: true,
            direction,
            revealed_fraction: 0.0,
            step_fraction: 0.1,
        }
    }

    /// Enables the curtain overlay.
    pub fn enable(&mut self) {
        self.active = true;
    }

    /// Disables the curtain overlay.
    pub fn disable(&mut self) {
        self.active = false;
    }

    /// Whether the curtain overlay is active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Current revealed fraction in `[0.0..=1.0]`.
    #[must_use]
    pub fn revealed_fraction(&self) -> f32 {
        self.revealed_fraction
    }

    /// Reveals one more line/step via keyboard (e.g., `Down` / `Right`).
    pub fn step_reveal(&mut self) {
        self.active = true;
        self.revealed_fraction = (self.revealed_fraction + self.step_fraction.max(0.01)).min(1.0);
    }

    /// Covers one step back via keyboard (e.g., `Up` / `Left`).
    pub fn step_hide(&mut self) {
        self.active = true;
        self.revealed_fraction = (self.revealed_fraction - self.step_fraction.max(0.01)).max(0.0);
    }

    /// Resets the curtain to fully covered (`0.0`), typically when advancing to a new slide.
    pub fn cover_all(&mut self) {
        self.revealed_fraction = 0.0;
    }

    /// Fully reveals the slide (`1.0`).
    pub fn reveal_all(&mut self) {
        self.revealed_fraction = 1.0;
    }

    /// Moves the curtain edge directly to a pen or pointer position `point` within `page_bounds`.
    pub fn drag_to(&mut self, point: PagePoint, page_bounds: PageRect) {
        self.active = true;
        let w = (page_bounds.x1 - page_bounds.x0).max(1.0);
        let h = (page_bounds.y1 - page_bounds.y0).max(1.0);
        let frac = match self.direction {
            CurtainDirection::TopToBottom => (point.y - page_bounds.y0) / h,
            CurtainDirection::BottomToTop => (page_bounds.y1 - point.y) / h,
            CurtainDirection::LeftToRight => (point.x - page_bounds.x0) / w,
            CurtainDirection::RightToLeft => (page_bounds.x1 - point.x) / w,
        };
        self.revealed_fraction = frac.clamp(0.0, 1.0);
    }

    /// Computes the opaque rectangle covering the hidden portion of `page_bounds`, or `None` when
    /// disabled or fully revealed.
    #[must_use]
    pub fn covered_rect(&self, page_bounds: PageRect) -> Option<PageRect> {
        if !self.active || self.revealed_fraction >= 1.0 - 1e-4 {
            return None;
        }
        let w = page_bounds.x1 - page_bounds.x0;
        let h = page_bounds.y1 - page_bounds.y0;
        let f = self.revealed_fraction.clamp(0.0, 1.0);
        Some(match self.direction {
            CurtainDirection::TopToBottom => PageRect {
                x0: page_bounds.x0,
                y0: page_bounds.y0 + h * f,
                x1: page_bounds.x1,
                y1: page_bounds.y1,
            },
            CurtainDirection::BottomToTop => PageRect {
                x0: page_bounds.x0,
                y0: page_bounds.y0,
                x1: page_bounds.x1,
                y1: page_bounds.y1 - h * f,
            },
            CurtainDirection::LeftToRight => PageRect {
                x0: page_bounds.x0 + w * f,
                y0: page_bounds.y0,
                x1: page_bounds.x1,
                y1: page_bounds.y1,
            },
            CurtainDirection::RightToLeft => PageRect {
                x0: page_bounds.x0,
                y0: page_bounds.y0,
                x1: page_bounds.x1 - w * f,
                y1: page_bounds.y1,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 800.0,
        y1: 450.0,
    };

    #[test]
    fn keyboard_and_pen_drag_both_move_curtain_accurately() {
        let mut curtain = CurtainReveal::new(CurtainDirection::TopToBottom);
        let full = curtain.covered_rect(SLIDE).expect("fully covered");
        assert_eq!(full, SLIDE);

        // Keyboard step reveal (2 steps = 20% revealed -> top 90 pt visible, bottom 360 pt covered).
        curtain.step_reveal();
        curtain.step_reveal();
        let after_keys = curtain.covered_rect(SLIDE).expect("partial");
        assert!((after_keys.y0 - 90.0).abs() < 0.01);
        assert_eq!(after_keys.y1, 450.0);

        // Pen drag directly to halfway down the slide.
        curtain.drag_to(PagePoint { x: 400.0, y: 225.0 }, SLIDE);
        let after_pen = curtain.covered_rect(SLIDE).expect("half");
        assert!((after_pen.y0 - 225.0).abs() < 0.01);

        // Horizontal reveal direction.
        curtain.direction = CurtainDirection::LeftToRight;
        curtain.drag_to(PagePoint { x: 200.0, y: 100.0 }, SLIDE);
        let horiz = curtain.covered_rect(SLIDE).expect("horiz");
        assert!((horiz.x0 - 200.0).abs() < 0.01);
        assert_eq!(horiz.x1, 800.0);

        curtain.reveal_all();
        assert!(curtain.covered_rect(SLIDE).is_none());
    }
}
