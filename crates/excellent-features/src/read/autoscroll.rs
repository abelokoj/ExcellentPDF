//! Auto-scroll (plan R.21), the model. A port of SumatraPDF Enhanced's `src/ReadingAutoScroll.cpp`: hands-free
//! scrolling at a stored speed, paused and resumed, faster and slower in the same steps; and of its middle-click
//! scrolling (`Canvas.cpp`), which scrolls toward the pointer, faster the farther the pointer is from where the button
//! went down.
//!
//! Both turn the time since the last frame into whole pixels and carry the fraction, so the pages move by whole
//! pixels (sharp tiles) at an even speed whatever the display's refresh rate.

/// The speeds a step goes through, in pixels a second at 96 dpi (Enhanced's `kSpeeds`).
pub const SPEEDS: [f32; 17] = [
    8.0, 12.0, 16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 56.0, 64.0, 80.0, 96.0, 120.0, 160.0, 200.0,
    260.0, 320.0,
];
/// The speed until the reader picks one (Enhanced's `ReadingAutoScrollSpeed`).
pub const DEFAULT_SPEED: f32 = 40.0;
/// The longest frame counted, in seconds: after a stall the pages don't jump.
const MAX_FRAME: f32 = 0.1;
/// Middle-click scrolling's speed for each pixel the pointer is from where it started, in pixels a second (Enhanced
/// moves a tenth of the distance every 20 ms).
const PAN_RATE: f32 = 5.0;

/// A speed in range: a hand-edited setting may hold anything.
pub fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(SPEEDS[0], SPEEDS[SPEEDS.len() - 1])
    } else {
        DEFAULT_SPEED
    }
}

/// Which way to change the speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The next speed up.
    Faster,
    /// The next speed down.
    Slower,
}

/// The speed one step from `speed`, from the step nearest it.
pub fn step_speed(speed: f32, change: Change) -> f32 {
    let speed = clamp_speed(speed);
    let nearest = (0..SPEEDS.len())
        .min_by(|&a, &b| {
            (SPEEDS[a] - speed)
                .abs()
                .total_cmp(&(SPEEDS[b] - speed).abs())
        })
        .unwrap_or(0);
    let next = match change {
        Change::Faster => (nearest + 1).min(SPEEDS.len() - 1),
        Change::Slower => nearest.saturating_sub(1),
    };
    SPEEDS[next]
}

/// Whole pixels to move now, from `carry` plus `pixels`; the fraction left stays in `carry`.
fn whole(carry: &mut f32, pixels: f32) -> f32 {
    *carry += pixels;
    let moved = carry.trunc();
    *carry -= moved;
    moved
}

/// Seconds since the last frame, as counted: never negative, never more than [`MAX_FRAME`].
fn frame(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.clamp(0.0, MAX_FRAME)
    } else {
        0.0
    }
}

/// Scrolling toward the later pages at a steady speed.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoScroll {
    speed: f32,
    paused: bool,
    carry: f32,
}

impl AutoScroll {
    /// Scrolling at `speed` pixels a second (at 96 dpi), brought into range.
    pub fn new(speed: f32) -> Self {
        Self {
            speed: clamp_speed(speed),
            paused: false,
            carry: 0.0,
        }
    }

    /// Its speed, in pixels a second at 96 dpi.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Goes one speed step faster or slower; returns the new speed.
    pub fn change_speed(&mut self, change: Change) -> f32 {
        self.speed = step_speed(self.speed, change);
        self.speed
    }

    /// Whether it is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Pauses it, or goes on.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.carry = 0.0;
    }

    /// Stops it where it is: it reached the end.
    pub fn pause(&mut self) {
        self.paused = true;
        self.carry = 0.0;
    }

    /// The whole device pixels to scroll for a frame `seconds` after the last, at `scale` device pixels to a 96-dpi
    /// pixel; nothing while paused.
    pub fn advance(&mut self, seconds: f32, scale: f32) -> f32 {
        if self.paused {
            return 0.0;
        }
        whole(&mut self.carry, self.speed * scale * frame(seconds))
    }
}

/// Middle-click scrolling: toward the pointer, faster the farther it is from where the button went down.
#[derive(Debug, Clone, PartialEq)]
pub struct Pan {
    origin: (f32, f32),
    carry: (f32, f32),
}

impl Pan {
    /// Scrolling from client pixel (`x`, `y`), where the middle button went down.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            origin: (x, y),
            carry: (0.0, 0.0),
        }
    }

    /// Where it started.
    pub fn origin(&self) -> (f32, f32) {
        self.origin
    }

    /// The whole device pixels to scroll, across and down, for a frame `seconds` after the last with the pointer at
    /// (`x`, `y`).
    pub fn advance(&mut self, x: f32, y: f32, seconds: f32) -> (f32, f32) {
        let seconds = frame(seconds);
        let dx = (x - self.origin.0) * PAN_RATE * seconds;
        let dy = (y - self.origin.1) * PAN_RATE * seconds;
        (whole(&mut self.carry.0, dx), whole(&mut self.carry.1, dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_of_any_rate_cover_the_same_distance() {
        // One second at 40 px/s and 1.5x scale is 60 device pixels, at 60 Hz and at 120 Hz alike.
        for hz in [60.0_f32, 120.0, 144.0] {
            let mut scroll = AutoScroll::new(40.0);
            let total: f32 = (0..hz as usize)
                .map(|_| scroll.advance(1.0 / hz, 1.5))
                .sum();
            assert!((total - 60.0).abs() <= 1.0, "{hz} Hz: {total}");
        }
    }

    #[test]
    fn it_moves_by_whole_pixels() {
        // An eighth of a pixel a frame (exact in binary): a pixel every eighth frame.
        let mut scroll = AutoScroll::new(8.0);
        let steps: Vec<f32> = (0..32).map(|_| scroll.advance(1.0 / 64.0, 1.0)).collect();
        assert!(steps.iter().all(|&s| s == 0.0 || s == 1.0), "{steps:?}");
        assert_eq!(steps.iter().sum::<f32>(), 4.0);
    }

    #[test]
    fn paused_it_moves_nothing_and_a_stall_does_not_jump() {
        let mut scroll = AutoScroll::new(320.0);
        scroll.toggle_pause();
        assert_eq!(scroll.advance(0.5, 1.0), 0.0);
        scroll.toggle_pause();
        assert_eq!(
            scroll.advance(5.0, 1.0),
            32.0,
            "counted as a tenth of a second"
        );
    }

    #[test]
    fn speeds_step_through_the_table_and_stop_at_its_ends() {
        assert_eq!(step_speed(40.0, Change::Faster), 48.0);
        assert_eq!(step_speed(40.0, Change::Slower), 32.0);
        assert_eq!(
            step_speed(41.0, Change::Faster),
            48.0,
            "from the nearest step"
        );
        assert_eq!(step_speed(320.0, Change::Faster), 320.0);
        assert_eq!(step_speed(8.0, Change::Slower), 8.0);
        assert_eq!(clamp_speed(f32::NAN), DEFAULT_SPEED);
        assert_eq!(clamp_speed(1000.0), 320.0);
    }

    #[test]
    fn middle_click_scrolls_toward_the_pointer_faster_the_farther_it_is() {
        let mut pan = Pan::new(100.0, 100.0);
        assert_eq!(pan.advance(100.0, 100.0, 0.1), (0.0, 0.0));
        assert_eq!(pan.advance(100.0, 140.0, 0.1), (0.0, 20.0));
        assert_eq!(pan.advance(60.0, 180.0, 0.1), (-20.0, 40.0));
    }
}
