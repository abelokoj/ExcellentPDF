//! Counting clicks: a second press soon enough and near enough to the first is a double click, a third a triple. Win32
//! and the other platforms report double clicks only; selection wants triple and quadruple ones too (K.13).

/// How soon and how near a press must be to the last one to count as the next click (`GetDoubleClickTime`,
/// `SM_CXDOUBLECLK` and `SM_CYDOUBLECLK`).
#[derive(Debug, Clone, Copy)]
pub struct ClickLimits {
    /// Milliseconds.
    pub time: u32,
    /// Pixels across the area, centred on the last press.
    pub width: i32,
    /// Pixels down it.
    pub height: i32,
}

/// The presses so far.
#[derive(Debug, Default)]
pub(crate) struct Clicks {
    /// The last press: its message time and client pixel.
    last: Option<(u32, i32, i32)>,
    count: u32,
}

impl Clicks {
    /// A press at message time `time` (milliseconds, wrapping) and client pixel (`x`, `y`): which click it is.
    pub(crate) fn press(&mut self, time: u32, x: i32, y: i32, limits: ClickLimits) -> u32 {
        let again = self.last.is_some_and(|(t, lx, ly)| {
            time.wrapping_sub(t) <= limits.time
                && (x - lx).abs() * 2 <= limits.width
                && (y - ly).abs() * 2 <= limits.height
        });
        self.count = if again { self.count + 1 } else { 1 };
        self.last = Some((time, x, y));
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: ClickLimits = ClickLimits {
        time: 500,
        width: 4,
        height: 4,
    };

    #[test]
    fn quick_presses_in_one_place_count_up() {
        let mut clicks = Clicks::default();
        assert_eq!(clicks.press(1000, 10, 10, LIMITS), 1);
        assert_eq!(clicks.press(1200, 11, 9, LIMITS), 2);
        assert_eq!(clicks.press(1600, 11, 9, LIMITS), 3);
        assert_eq!(clicks.press(2200, 11, 9, LIMITS), 1, "too slow");
        assert_eq!(clicks.press(2300, 20, 9, LIMITS), 1, "too far");
    }

    #[test]
    fn the_message_clock_may_wrap() {
        let mut clicks = Clicks::default();
        clicks.press(u32::MAX - 100, 0, 0, LIMITS);
        assert_eq!(clicks.press(100, 0, 0, LIMITS), 2);
    }
}
