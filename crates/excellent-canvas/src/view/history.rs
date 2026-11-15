//! Back and forward (plan R.8): where the reader was before each jump (a link, go to page, the outline, a search
//! result). Scrolling is not a jump and adds nothing.

use crate::viewport::{Anchor, ZoomMode};

/// Places kept each way; the oldest goes first.
const LIMIT: usize = 256;

/// A place in a document: the page and the offset on it, and the zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    /// The page and where on it.
    pub anchor: Anchor,
    /// How the zoom was chosen.
    pub mode: ZoomMode,
    /// The zoom, used when `mode` is custom.
    pub zoom: f32,
}

/// One view's back and forward lists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl History {
    /// The reader jumps away from `here`: back returns to it, and the places ahead are forgotten.
    pub fn jumped_from(&mut self, here: Place) {
        if self.back.last() != Some(&here) {
            push(&mut self.back, here);
        }
        self.forward.clear();
    }

    /// Where back goes from `here`, which forward then returns to; `None` at the start.
    pub fn back(&mut self, here: Place) -> Option<Place> {
        let to = self.back.pop()?;
        push(&mut self.forward, here);
        Some(to)
    }

    /// Where forward goes from `here`, which back then returns to; `None` at the end.
    pub fn forward(&mut self, here: Place) -> Option<Place> {
        let to = self.forward.pop()?;
        push(&mut self.back, here);
        Some(to)
    }

    /// Whether back has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether forward has somewhere to go.
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

fn push(places: &mut Vec<Place>, place: Place) {
    if places.len() == LIMIT {
        places.remove(0);
    }
    places.push(place);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(page: usize) -> Place {
        Place {
            anchor: Anchor {
                page,
                across: 0.0,
                down: 0.25,
            },
            mode: ZoomMode::FitWidth,
            zoom: 1.0,
        }
    }

    #[test]
    fn three_jumps_and_three_backs_return_to_the_start() {
        let mut history = History::default();
        let mut here = page(0);
        for target in [10, 20, 30] {
            history.jumped_from(here);
            here = page(target);
        }
        for expected in [20, 10, 0] {
            here = history.back(here).expect("back");
            assert_eq!(here.anchor.page, expected);
        }
        assert_eq!(history.back(here), None, "at the start");
        here = history.forward(here).expect("forward");
        assert_eq!(here.anchor.page, 10);
    }

    #[test]
    fn a_new_jump_forgets_the_places_ahead() {
        let mut history = History::default();
        history.jumped_from(page(0));
        let here = history.back(page(5)).expect("back");
        assert!(history.can_go_forward());
        history.jumped_from(here);
        assert!(!history.can_go_forward());
        assert!(history.can_go_back());
    }

    #[test]
    fn history_is_bounded() {
        let mut history = History::default();
        for target in 0..LIMIT + 10 {
            history.jumped_from(page(target));
        }
        assert_eq!(history.back.len(), LIMIT);
        assert_eq!(history.back[0].anchor.page, 10);
    }
}
