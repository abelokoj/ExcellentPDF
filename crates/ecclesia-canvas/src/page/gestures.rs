//! Palm rejection and "finger draws" (H.8, plan §7.3): which touches on the page count. While the pen is near the
//! screen, touching it, or has just left it, every touch is a hand resting on the screen and is ignored; so is any
//! contact too large for a fingertip. A touch keeps its verdict until it lifts, so a palm that settles before the pen
//! comes in range doesn't start counting half way. A touch that counts draws ink when the reader turned "finger
//! draws" on, and otherwise pans and zooms as the platform does. Pure: the platform layer feeds it events and acts on
//! its answers.

use std::collections::HashMap;

/// How long after the pen was last near the screen touches are still taken for the hand that held it, in
/// milliseconds: a hand lifts after the pen does.
pub const GRACE_MS: f64 = 500.0;

/// The widest contact taken for a fingertip, in millimetres; wider is a palm or the side of a hand. A fingertip
/// presses 8 to 12 mm wide; the side of a hand resting on glass is 20 mm and more.
pub const FINGER_MM: f32 = 16.0;

/// What a touch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing: a resting hand.
    Ignore,
    /// What a finger does on the platform: pan, zoom, press.
    Pass,
    /// Draws ink, as the pen would.
    Draw,
}

/// The touches on the page and what the pen was doing when each came down.
#[derive(Debug, Default)]
pub struct Palms {
    /// When the pen was last near or on the screen, in milliseconds on the input clock; `None`: not yet.
    pen_seen: Option<f64>,
    /// The pen is near or on the screen now.
    pen_near: bool,
    /// Each touch down now, and its verdict.
    touches: HashMap<u32, Verdict>,
    /// Fingers draw ink.
    finger_draws: bool,
}

impl Palms {
    /// Whether a finger that counts draws ink (the reader's "finger draws"), or pans and zooms.
    pub fn set_finger_draws(&mut self, on: bool) {
        self.finger_draws = on;
    }

    /// The pen is near the screen or on it, at `time`.
    pub fn pen_near(&mut self, time: f64) {
        self.pen_near = true;
        self.pen_seen = Some(time);
    }

    /// The pen left the screen's range at `time`.
    pub fn pen_gone(&mut self, time: f64) {
        self.pen_near = false;
        self.pen_seen = Some(time);
    }

    /// Touch `id` came down at `time`, `width` millimetres across (`None`: the platform doesn't say). Its verdict,
    /// kept until it lifts.
    pub fn touch_down(&mut self, id: u32, time: f64, width: Option<f32>) -> Verdict {
        let pen_about = self.pen_near
            || self
                .pen_seen
                .is_some_and(|seen| time - seen >= 0.0 && time - seen < GRACE_MS);
        let palm = width.is_some_and(|w| w > FINGER_MM);
        let verdict = if pen_about || palm {
            Verdict::Ignore
        } else if self.finger_draws {
            Verdict::Draw
        } else {
            Verdict::Pass
        };
        self.touches.insert(id, verdict);
        verdict
    }

    /// Touch `id`'s verdict while it is down; a touch it never saw passes, as the platform would have it.
    #[must_use]
    pub fn verdict(&self, id: u32) -> Verdict {
        self.touches.get(&id).copied().unwrap_or(Verdict::Pass)
    }

    /// Touch `id` lifted.
    pub fn touch_up(&mut self, id: u32) {
        self.touches.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One event of a recorded session: the pen near or gone, a touch down or up.
    #[derive(Debug, Clone, Copy)]
    enum Event {
        PenNear(f64),
        PenGone(f64),
        Down(u32, f64, Option<f32>),
        Up(u32),
    }

    /// A note written with the hand resting on the screen, as a Surface reports it: the palm lands before the pen
    /// touches and lifts after it leaves, the side of the hand slides along, a knuckle brushes the glass between
    /// words, and a fingertip scrolls the page once the pen is away.
    fn writing_with_the_hand_resting() -> Vec<Event> {
        use Event::{Down, PenGone, PenNear, Up};
        vec![
            // The pen comes near; the palm lands (wide), then the side of the hand (narrow but during the pen).
            PenNear(0.0),
            Down(1, 40.0, Some(34.0)),
            Down(2, 90.0, Some(11.0)),
            PenNear(120.0),
            // A word, then the pen lifts between words but stays in range; a knuckle brushes the glass.
            PenNear(800.0),
            Down(3, 850.0, Some(9.0)),
            Up(3),
            PenNear(1600.0),
            // The pen leaves; the hand lifts a moment later and a finger touches while the hand lifts.
            PenGone(1700.0),
            Down(4, 1900.0, Some(10.0)),
            Up(4),
            Up(1),
            Up(2),
            // Well after: a fingertip scrolls the page.
            Down(5, 3000.0, Some(10.0)),
            Up(5),
        ]
    }

    fn replay(palms: &mut Palms, events: &[Event]) -> Vec<(u32, Verdict)> {
        let mut verdicts = Vec::new();
        for event in events {
            match *event {
                Event::PenNear(t) => palms.pen_near(t),
                Event::PenGone(t) => palms.pen_gone(t),
                Event::Down(id, t, width) => verdicts.push((id, palms.touch_down(id, t, width))),
                Event::Up(id) => palms.touch_up(id),
            }
        }
        verdicts
    }

    #[test]
    fn writing_with_the_hand_resting_leaves_no_stray_marks() {
        let mut palms = Palms::default();
        palms.set_finger_draws(true);
        let verdicts = replay(&mut palms, &writing_with_the_hand_resting());
        let drawn: Vec<u32> = verdicts
            .iter()
            .filter(|(_, verdict)| *verdict == Verdict::Draw)
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            drawn,
            [5],
            "only the fingertip long after the pen draws: {verdicts:?}"
        );
    }

    #[test]
    fn without_finger_draws_a_finger_pans() {
        let mut palms = Palms::default();
        let verdicts = replay(&mut palms, &writing_with_the_hand_resting());
        assert_eq!(verdicts.last(), Some(&(5, Verdict::Pass)));
        assert!(
            verdicts[..verdicts.len() - 1]
                .iter()
                .all(|(_, verdict)| *verdict == Verdict::Ignore)
        );
    }

    #[test]
    fn a_palm_is_ignored_with_no_pen_about_and_keeps_its_verdict() {
        let mut palms = Palms::default();
        palms.set_finger_draws(true);
        assert_eq!(
            palms.touch_down(1, 0.0, Some(30.0)),
            Verdict::Ignore,
            "too wide"
        );
        palms.pen_gone(10_000.0);
        assert_eq!(palms.verdict(1), Verdict::Ignore, "until it lifts");
        assert_eq!(
            palms.touch_down(2, 20_000.0, None),
            Verdict::Draw,
            "no size: a finger"
        );
        palms.touch_up(2);
        assert_eq!(palms.verdict(2), Verdict::Pass, "gone");
    }
}
