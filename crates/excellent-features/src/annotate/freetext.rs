//! Text boxes and typewriter text (A.6): a free-text annotation, its text drawn on the page in one of the standard
//! fonts, so every reader shows it alike. A text box keeps the width it was given, has a border in its text's colour
//! and grows down as its text wraps; typewriter text (`/IT /FreeTextTypeWriter`) has no border and its box fits the
//! text, as text typed onto a form that has no fields. A callout (A.7) is a text box with a leader line from the
//! point it is about, its tip, to the box's edge; the tip stays where it is when the box moves or grows. MuPDF draws
//! the text and clips what its box doesn't hold, so a box is fitted to its text whenever the text or its style
//! changes.

use excellent_core::annot::{Annotation, Boxed, Font, LineEnd, Shape, Style, TextStyle, now};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect};

use super::notes;

/// Which of the two a new one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A box of text with a border.
    Box,
    /// Text typed onto the page.
    Typewriter,
    /// A box with a leader line to the point clicked (A.7).
    Callout,
}

/// A new text box's width in points: about 40 characters of 12-point text.
pub const WIDTH: f32 = 200.0;

/// A text box's border width in points.
pub const BORDER: f32 = 1.0;

/// How far a new callout's box sits from its tip, across and up, in points.
const CALLOUT_OFFSET: (f32, f32) = (36.0, 36.0);

/// The leader-line endings a callout offers at its tip, the first its default (A.7).
pub const CALLOUT_ENDS: [LineEnd; 5] = [
    LineEnd::OpenArrow,
    LineEnd::ClosedArrow,
    LineEnd::Circle,
    LineEnd::Square,
    LineEnd::None,
];

/// Line height as MuPDF sets a text box's lines (`pdf-appearance.c`, `write_variable_text`), in ems.
const LINE: f32 = 1.2;

/// A new one on page `page` with its top left at `at`, kept inside the page (`bounds`); empty, fitted to one line. A
/// callout's tip is at `at` and its box above and to the right, or wherever the page has room.
#[must_use]
pub fn place(
    page: usize,
    at: PagePoint,
    bounds: PageRect,
    kind: Kind,
    text: TextStyle,
    author: &str,
) -> Annotation {
    let (boxed, style) = match kind {
        Kind::Box | Kind::Callout => (
            Boxed::FreeText,
            Style {
                color: None,
                opacity: 1.0,
                width: BORDER,
            },
        ),
        Kind::Typewriter => (
            Boxed::Typewriter,
            Style {
                color: None,
                opacity: 1.0,
                width: 0.0,
            },
        ),
    };
    let mut made = Annotation::new(
        page,
        Shape::Boxed(boxed, PageRect::default()),
        style,
        author,
    );
    made.text = text;
    let size = fitted(&made);
    let fit = |start: f32, length: f32, low: f32, high: f32| start.min(high - length).max(low);
    let corner = match kind {
        Kind::Callout => {
            let (across, up) = CALLOUT_OFFSET;
            // To the left of the tip when the right has no room, below it when the top has none.
            let x = if at.x + across + size.0 <= bounds.x1 {
                at.x + across
            } else {
                at.x - across - size.0
            };
            let y = if at.y - up - size.1 >= bounds.y0 {
                at.y - up - size.1
            } else {
                at.y + up
            };
            PagePoint { x, y }
        }
        Kind::Box | Kind::Typewriter => at,
    };
    let (x0, y0) = (
        fit(corner.x, size.0, bounds.x0, bounds.x1),
        fit(corner.y, size.1, bounds.y0, bounds.y1),
    );
    let rect = PageRect {
        x0,
        y0,
        x1: x0 + size.0,
        y1: y0 + size.1,
    };
    made.shape = match kind {
        Kind::Callout => {
            made.ends = [CALLOUT_ENDS[0], LineEnd::None];
            Shape::Callout(rect, at)
        }
        Kind::Box | Kind::Typewriter => Shape::Boxed(boxed, rect),
    };
    made
}

/// The kind of text box `annotation` is, if it is one.
#[must_use]
pub fn kind_of(annotation: &Annotation) -> Option<Kind> {
    match annotation.shape {
        Shape::Boxed(Boxed::FreeText, _) => Some(Kind::Box),
        Shape::Boxed(Boxed::Typewriter, _) => Some(Kind::Typewriter),
        Shape::Callout(..) => Some(Kind::Callout),
        _ => None,
    }
}

/// The change that ends the callout `annotation`'s leader line at its tip with `end`; none when it already does, or
/// for the other kinds.
#[must_use]
pub fn set_end(annotation: &Annotation, end: LineEnd) -> Option<Change> {
    (matches!(annotation.shape, Shape::Callout(..)) && annotation.ends[0] != end).then(|| {
        Change::UpdateAnnotation(Annotation {
            ends: [end, annotation.ends[1]],
            modified: now(),
            ..annotation.clone()
        })
    })
}

/// The change that gives the text box `annotation` the text `text`, its box fitted to it; none when the text stays.
#[must_use]
pub fn set_text(annotation: &Annotation, text: &str) -> Option<Change> {
    let text = notes::normalize(text);
    (text != annotation.contents).then(|| {
        refit(Annotation {
            contents: text,
            ..annotation.clone()
        })
    })
}

/// The change that gives the text box `annotation` the text style `text`; none when it has it.
#[must_use]
pub fn set_style(annotation: &Annotation, text: TextStyle) -> Option<Change> {
    (annotation.text != text).then(|| {
        refit(Annotation {
            text,
            ..annotation.clone()
        })
    })
}

/// The change that draws the text box `annotation`'s border, or leaves it out; none when it already is, or for
/// typewriter text, which has none.
#[must_use]
pub fn set_border(annotation: &Annotation, on: bool) -> Option<Change> {
    let width = if on { BORDER } else { 0.0 };
    let boxed = matches!(
        annotation.shape,
        Shape::Boxed(Boxed::FreeText, _) | Shape::Callout(..)
    );
    (boxed && annotation.style.width != width).then(|| {
        refit(Annotation {
            style: Style {
                width,
                ..annotation.style
            },
            ..annotation.clone()
        })
    })
}

/// `annotation`, changed now, its box fitted to its text from the same top left; a callout's tip stays.
fn refit(mut annotation: Annotation) -> Change {
    let (width, height) = fitted(&annotation);
    let grown = |rect: PageRect| PageRect {
        x1: rect.x0 + width,
        y1: rect.y0 + height,
        ..rect
    };
    match annotation.shape {
        Shape::Boxed(kind, rect) => annotation.shape = Shape::Boxed(kind, grown(rect)),
        Shape::Callout(rect, tip) => annotation.shape = Shape::Callout(grown(rect), tip),
        _ => {}
    }
    annotation.modified = now();
    Change::UpdateAnnotation(annotation)
}

/// The width and height `annotation`'s box needs for its text: a text box keeps its width (or [`WIDTH`] when it has
/// none yet) and wraps; typewriter text is as wide as its longest line.
fn fitted(annotation: &Annotation) -> (f32, f32) {
    let text = annotation.text;
    // MuPDF insets the text by twice the border on every side.
    let inset = 2.0 * annotation.style.width;
    let lines: Vec<f32> = annotation
        .contents
        .split('\n')
        .map(|line| advance(line, text.font) * text.size)
        .collect();
    let height = |count: usize| count.max(1) as f32 * text.size * LINE + 2.0 * inset + 1.0;
    match annotation.shape {
        Shape::Boxed(Boxed::Typewriter, _) => {
            let widest = lines.iter().copied().fold(0.0, f32::max);
            (
                widest.max(text.size) + 2.0 * inset + 2.0,
                height(lines.len()),
            )
        }
        Shape::Boxed(_, rect) | Shape::Callout(rect, _) => {
            let width = if rect.x1 > rect.x0 {
                rect.x1 - rect.x0
            } else {
                WIDTH
            };
            let room = (width - 2.0 * inset).max(text.size);
            let count = lines
                .iter()
                .map(|line| (line / room).ceil().max(1.0) as usize)
                .sum();
            (width, height(count))
        }
        _ => (0.0, 0.0),
    }
}

/// A line's width in ems.
///
/// shortcut: average advances per font, a little generous so the text is not clipped, not the fonts' own metrics;
/// measure with MuPDF (`fz_advance_glyph`) if boxes are seen to clip or to leave too much room.
fn advance(line: &str, font: Font) -> f32 {
    let narrow = match font {
        Font::Sans => 0.58,
        Font::Serif => 0.55,
        Font::Mono => 0.6,
    };
    line.chars()
        .map(|c| if is_wide(c) { 1.0 } else { narrow })
        .sum()
}

/// Whether `c` is drawn a full em wide: CJK and the other East Asian wide characters, which MuPDF draws in its CJK
/// font.
fn is_wide(c: char) -> bool {
    matches!(u32::from(c), 0x1100..=0x115f | 0x2e80..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe30..=0xfe4f | 0xff00..=0xff60 | 0xffe0..=0xffe6 | 0x2_0000..=0x3_fffd)
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::Align;

    use super::*;

    const PAGE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    fn rect(annotation: &Annotation) -> PageRect {
        match annotation.shape {
            Shape::Boxed(_, rect) => rect,
            _ => panic!("a text box"),
        }
    }

    fn changed(change: Option<Change>) -> Annotation {
        match change {
            Some(Change::UpdateAnnotation(annotation)) => annotation,
            other => panic!("an update, not {other:?}"),
        }
    }

    #[test]
    fn a_text_box_is_placed_at_the_click_with_a_border_and_typewriter_text_without() {
        let boxed = place(
            1,
            PagePoint { x: 72.0, y: 100.0 },
            PAGE,
            Kind::Box,
            TextStyle::default(),
            "me",
        );
        assert!(boxed.is_text_box());
        assert!(matches!(boxed.shape, Shape::Boxed(Boxed::FreeText, _)));
        let at = rect(&boxed);
        assert_eq!((at.x0, at.y0, at.x1 - at.x0), (72.0, 100.0, WIDTH));
        assert_eq!(boxed.style.width, BORDER);
        assert_eq!(boxed.style.color, None, "no background");

        let typed = place(
            0,
            PagePoint { x: 600.0, y: 790.0 },
            PAGE,
            Kind::Typewriter,
            TextStyle::default(),
            "me",
        );
        assert!(matches!(typed.shape, Shape::Boxed(Boxed::Typewriter, _)));
        assert_eq!(typed.style.width, 0.0);
        let at = rect(&typed);
        assert!(at.x1 <= PAGE.x1 && at.y1 <= PAGE.y1, "kept on the page");
    }

    #[test]
    fn typing_fits_the_box_to_the_text() {
        let typed = place(
            0,
            PagePoint { x: 10.0, y: 10.0 },
            PAGE,
            Kind::Typewriter,
            TextStyle::default(),
            "me",
        );
        let short = changed(set_text(&typed, "Name"));
        let long = changed(set_text(&short, "Name: Ada Lovelace\nSecond line"));
        let (a, b) = (rect(&short), rect(&long));
        assert!(b.x1 - b.x0 > a.x1 - a.x0, "wider for a longer line");
        assert!(b.y1 - b.y0 > a.y1 - a.y0, "taller for two lines");
        assert_eq!((b.x0, b.y0), (10.0, 10.0), "the top left stays");
        assert!(set_text(&long, "Name: Ada Lovelace\nSecond line\n").is_none());

        let boxed = place(
            0,
            PagePoint { x: 10.0, y: 10.0 },
            PAGE,
            Kind::Box,
            TextStyle::default(),
            "me",
        );
        let wrapped = changed(set_text(&boxed, &"word ".repeat(40)));
        let at = rect(&wrapped);
        assert_eq!(at.x1 - at.x0, WIDTH, "a box keeps its width");
        assert!(at.y1 - at.y0 > 3.0 * 12.0 * LINE, "and wraps");
    }

    #[test]
    fn a_callout_points_at_the_click_from_a_box_that_has_room() {
        let tip = PagePoint { x: 100.0, y: 400.0 };
        let callout = place(0, tip, PAGE, Kind::Callout, TextStyle::default(), "me");
        assert_eq!(kind_of(&callout), Some(Kind::Callout));
        let Shape::Callout(rect, at) = callout.shape else {
            panic!("a callout");
        };
        assert_eq!(at, tip);
        assert!(rect.x0 > tip.x && rect.y1 < tip.y, "above and to the right");
        assert_eq!(callout.ends[0], LineEnd::OpenArrow);
        assert_eq!(callout.style.width, BORDER);

        let corner = place(
            0,
            PagePoint { x: 600.0, y: 10.0 },
            PAGE,
            Kind::Callout,
            TextStyle::default(),
            "me",
        );
        let Shape::Callout(rect, _) = corner.shape else {
            panic!("a callout");
        };
        assert!(
            rect.x1 < 600.0 && rect.y0 > 10.0,
            "left and below near the top right corner"
        );

        let typed = changed(set_text(
            &callout,
            "A long line of text for the callout box to wrap",
        ));
        let Shape::Callout(_, still) = typed.shape else {
            panic!("a callout");
        };
        assert_eq!(still, tip, "the tip stays as the box grows");
        assert!(set_end(&typed, LineEnd::OpenArrow).is_none());
        assert_eq!(
            changed(set_end(&typed, LineEnd::Circle)).ends[0],
            LineEnd::Circle
        );
        let boxed = place(0, tip, PAGE, Kind::Box, TextStyle::default(), "me");
        assert!(
            set_end(&boxed, LineEnd::Circle).is_none(),
            "only a callout has a tip"
        );
    }

    #[test]
    fn chinese_text_is_measured_a_full_em_a_character() {
        assert_eq!(advance("中文输入", Font::Sans), 4.0);
        assert!(advance("abcd", Font::Sans) < 4.0);
    }

    #[test]
    fn a_style_or_border_change_is_one_change_only_when_it_changes() {
        let boxed = place(
            0,
            PagePoint { x: 10.0, y: 10.0 },
            PAGE,
            Kind::Box,
            TextStyle::default(),
            "me",
        );
        assert!(set_style(&boxed, TextStyle::default()).is_none());
        let bigger = changed(set_style(
            &boxed,
            TextStyle {
                size: 24.0,
                font: Font::Serif,
                align: Align::Center,
                color: [1.0, 0.0, 0.0],
            },
        ));
        assert!(rect(&bigger).y1 > rect(&boxed).y1, "taller for bigger text");
        assert!(set_border(&bigger, true).is_none());
        assert_eq!(changed(set_border(&bigger, false)).style.width, 0.0);
        let typed = place(
            0,
            PagePoint { x: 0.0, y: 0.0 },
            PAGE,
            Kind::Typewriter,
            TextStyle::default(),
            "me",
        );
        assert!(
            set_border(&typed, true).is_none(),
            "typewriter text has none"
        );
    }
}
