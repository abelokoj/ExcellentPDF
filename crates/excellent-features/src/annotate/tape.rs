//! Tape (H.4): an opaque strip laid over a definition or an answer to test yourself, tapped to reveal what is under
//! it and tapped again to hide it. Saved as a standard Square annotation, filled, with the private `tape` mark, so
//! other readers show a plain bar and this app knows to reveal it.

use excellent_core::annot::{Annotation, Boxed, Shape, Style};
use excellent_core::edit::Change;
use excellent_mupdf::PageRect;

use super::edit;

/// The tape's colour: a soft blue, clearly not part of the document.
pub const TAPE: [f32; 3] = [0.55, 0.71, 0.86];

/// How much of the tape shows once revealed: enough to see where it is, not enough to hide anything.
pub const REVEALED: f32 = 0.18;

/// A strip of tape over `rect` on page `page`, by `author`.
#[must_use]
pub fn tape(page: usize, rect: PageRect, author: &str) -> Annotation {
    let mut tape = Annotation::new(
        page,
        Shape::Boxed(Boxed::Square, rect),
        Style {
            color: Some(TAPE),
            opacity: 1.0,
            width: 0.0,
        },
        author,
    );
    tape.fill = Some(TAPE);
    tape.tape = true;
    tape
}

/// Whether `annotation` is tape hiding what is under it (not revealed).
#[must_use]
pub fn is_hiding(annotation: &Annotation) -> bool {
    annotation.tape && annotation.style.opacity > REVEALED
}

/// The change a tap on `annotation` makes: tape hiding its text is revealed, revealed tape hides again. `None` for
/// anything that isn't tape, or tape a lock keeps as it is.
#[must_use]
pub fn tapped(annotation: &Annotation) -> Option<Change> {
    if !annotation.tape {
        return None;
    }
    let mut after = annotation.clone();
    after.style.opacity = if is_hiding(annotation) { REVEALED } else { 1.0 };
    edit::change(annotation, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip() -> Annotation {
        tape(
            0,
            PageRect {
                x0: 72.0,
                y0: 100.0,
                x1: 300.0,
                y1: 116.0,
            },
            "Ada",
        )
    }

    #[test]
    fn tape_is_an_opaque_filled_square_marked_as_tape() {
        let t = strip();
        assert!(matches!(t.shape, Shape::Boxed(Boxed::Square, _)));
        assert_eq!(t.fill, Some(TAPE), "filled, so other readers show a bar");
        assert!(t.tape && is_hiding(&t));
        // The mark goes into the file's private key.
        let data = t.to_data();
        assert!(data.app_data.contains("\"tape\":true"), "{}", data.app_data);
    }

    #[test]
    fn a_tap_reveals_and_another_hides_again() {
        let t = strip();
        let Some(Change::UpdateAnnotation(revealed)) = tapped(&t) else {
            panic!("a change");
        };
        assert!(!is_hiding(&revealed));
        let Some(Change::UpdateAnnotation(hidden)) = tapped(&revealed) else {
            panic!("a change");
        };
        assert!(is_hiding(&hidden));
    }

    #[test]
    fn only_tape_and_unlocked_tape_answer_a_tap() {
        let mut plain = strip();
        plain.tape = false;
        assert_eq!(tapped(&plain), None);
        let mut locked = strip();
        locked.locked = true;
        assert_eq!(tapped(&locked), None, "a lock keeps it as it is");
    }
}
