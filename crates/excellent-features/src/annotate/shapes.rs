//! Lines and arrows (A.8): a line drawn by dragging on a page, from where the button went down to where it came up,
//! with an ending at each end (`/LE`); an arrow is a line with an open arrowhead at its end. Shift snaps the line to
//! steps of 15°, as Acrobat does. MuPDF draws the endings into the line's appearance, so they look the same in
//! every reader.

use excellent_core::annot::{Annotation, LineEnd, Shape, Style, now};
use excellent_core::edit::Change;
use excellent_mupdf::PagePoint;
use serde_json::Value;

use super::markup::{color, floats, opacity};

/// What a drag on the page draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain line.
    Line,
    /// A line with an open arrowhead at its end.
    Arrow,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::Line, Self::Arrow];

    /// Its name in events and props.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Arrow => "arrow",
        }
    }

    /// The kind named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// How a new one starts and ends.
    #[must_use]
    pub fn ends(self) -> [LineEnd; 2] {
        match self {
            Self::Line => [LineEnd::None; 2],
            Self::Arrow => [LineEnd::None, LineEnd::OpenArrow],
        }
    }
}

/// The endings a line offers at each end, none first: the plan's six and none.
pub const ENDS: [LineEnd; 7] = [
    LineEnd::None,
    LineEnd::OpenArrow,
    LineEnd::ClosedArrow,
    LineEnd::Circle,
    LineEnd::Square,
    LineEnd::Diamond,
    LineEnd::Slash,
];

/// A new line's width, in points.
pub const WIDTH: f32 = 2.0;

/// The shortest line a drag makes, in points: a shorter one is a click, not a line.
pub const MIN_LENGTH: f32 = 3.0;

/// Shift snaps a line's angle to multiples of this, in degrees.
const SNAP_DEGREES: f32 = 15.0;

/// `to`, moved round `from` to the nearest multiple of 15°, at the same distance from it.
#[must_use]
pub fn snap(from: PagePoint, to: PagePoint) -> PagePoint {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return to;
    }
    let step = SNAP_DEGREES.to_radians();
    let angle = (dy.atan2(dx) / step).round() * step;
    PagePoint {
        x: from.x + length * angle.cos(),
        y: from.y + length * angle.sin(),
    }
}

/// A new `kind` on page `page` from `from` to `to`, `WIDTH` wide in `style`'s colour and opacity; `None` when it is
/// shorter than [`MIN_LENGTH`].
#[must_use]
pub fn draw(
    page: usize,
    from: PagePoint,
    to: PagePoint,
    kind: Kind,
    style: Style,
    author: &str,
) -> Option<Annotation> {
    if length(from, to) < MIN_LENGTH {
        return None;
    }
    let mut line = Annotation::new(
        page,
        Shape::Line(from, to),
        Style {
            width: WIDTH,
            ..style
        },
        author,
    );
    line.ends = kind.ends();
    Some(line)
}

/// Whether `annotation` is a line.
#[must_use]
pub fn is_line(annotation: &Annotation) -> bool {
    matches!(annotation.shape, Shape::Line(..))
}

/// Whether the line `annotation` has an arrowhead at either end, so is called an arrow.
#[must_use]
pub fn is_arrow(annotation: &Annotation) -> bool {
    is_line(annotation)
        && annotation.ends.iter().any(|end| {
            matches!(
                end,
                LineEnd::OpenArrow
                    | LineEnd::ClosedArrow
                    | LineEnd::ROpenArrow
                    | LineEnd::RClosedArrow
            )
        })
}

/// The change that gives the line `annotation` the endings `ends`, start first; none when it has them already, or
/// isn't a line.
#[must_use]
pub fn set_ends(annotation: &Annotation, ends: [LineEnd; 2]) -> Option<Change> {
    (is_line(annotation) && annotation.ends != ends).then(|| {
        Change::UpdateAnnotation(Annotation {
            ends,
            modified: now(),
            ..annotation.clone()
        })
    })
}

/// How far `point` is from the line from `from` to `to`, in points.
#[must_use]
pub fn distance(from: PagePoint, to: PagePoint, point: PagePoint) -> f32 {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let squared = dx * dx + dy * dy;
    let along = if squared == 0.0 {
        0.0
    } else {
        (((point.x - from.x) * dx + (point.y - from.y) * dy) / squared).clamp(0.0, 1.0)
    };
    length(
        PagePoint {
            x: from.x + along * dx,
            y: from.y + along * dy,
        },
        point,
    )
}

fn length(a: PagePoint, b: PagePoint) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

/// A shape the page canvas drew (its `pageDraw` event).
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// The page, counting from zero.
    pub page: usize,
    /// What was drawn.
    pub kind: Kind,
    /// Its points, in page points: a line's start and end.
    pub points: Vec<PagePoint>,
    /// RGB from 0 to 1.
    pub color: [f32; 3],
    /// From 0 to 1.
    pub opacity: f32,
}

impl Drawn {
    /// The shape in `value`, the event's data: `{page, kind, points: [[x, y]], color: [r, g, b], opacity}`. `None`
    /// when the page, kind or colour is missing or a point doesn't parse.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
        let kind = Kind::parse(value.get("kind")?.as_str()?)?;
        let points = value
            .get("points")?
            .as_array()?
            .iter()
            .map(|point| match floats(point)?[..] {
                [x, y] => Some(PagePoint { x, y }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            page,
            kind,
            points,
            color: color(value.get("color")?)?,
            opacity: opacity(value),
        })
    }

    /// The annotation it makes, by `author`; `None` when it has too few points or is too short.
    #[must_use]
    pub fn annotation(&self, author: &str) -> Option<Annotation> {
        let style = Style {
            color: Some(self.color),
            opacity: self.opacity,
            ..Style::default()
        };
        match self.points[..] {
            [from, to] => draw(self.page, from, to, self.kind, style, author),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: PagePoint = PagePoint { x: 100.0, y: 100.0 };

    fn close(a: PagePoint, b: PagePoint) -> bool {
        length(a, b) < 0.001
    }

    #[test]
    fn shift_snaps_to_fifteen_degree_steps_at_the_same_length() {
        let nearly_flat = PagePoint { x: 200.0, y: 104.0 };
        let snapped = snap(ORIGIN, nearly_flat);
        assert!((snapped.y - ORIGIN.y).abs() < 0.001, "flat: {snapped:?}");
        assert!((length(ORIGIN, snapped) - length(ORIGIN, nearly_flat)).abs() < 0.001);
        let near_thirty = PagePoint {
            x: 100.0 + 100.0 * 31f32.to_radians().cos(),
            y: 100.0 + 100.0 * 31f32.to_radians().sin(),
        };
        let thirty = PagePoint {
            x: 100.0 + 100.0 * 30f32.to_radians().cos(),
            y: 100.0 + 100.0 * 30f32.to_radians().sin(),
        };
        assert!(close(snap(ORIGIN, near_thirty), thirty));
        assert!(
            close(snap(ORIGIN, ORIGIN), ORIGIN),
            "no length, nothing to snap"
        );
    }

    #[test]
    fn a_drag_makes_a_line_or_an_arrow_and_a_click_makes_nothing() {
        let to = PagePoint { x: 160.0, y: 180.0 };
        let arrow = draw(0, ORIGIN, to, Kind::Arrow, Style::default(), "me").expect("long enough");
        assert_eq!(arrow.shape, Shape::Line(ORIGIN, to));
        assert_eq!(arrow.ends, [LineEnd::None, LineEnd::OpenArrow]);
        assert_eq!(arrow.style.width, WIDTH);
        assert!(is_arrow(&arrow));
        let line = draw(0, ORIGIN, to, Kind::Line, Style::default(), "me").expect("long enough");
        assert!(is_line(&line) && !is_arrow(&line));
        let click = PagePoint { x: 101.0, y: 101.0 };
        assert!(draw(0, ORIGIN, click, Kind::Line, Style::default(), "me").is_none());
    }

    #[test]
    fn changing_the_ends_is_one_change_only_when_they_change() {
        let to = PagePoint { x: 200.0, y: 100.0 };
        let line = draw(0, ORIGIN, to, Kind::Line, Style::default(), "me").expect("long enough");
        assert!(set_ends(&line, [LineEnd::None; 2]).is_none());
        let Some(Change::UpdateAnnotation(changed)) =
            set_ends(&line, [LineEnd::Circle, LineEnd::ClosedArrow])
        else {
            panic!("one update");
        };
        assert_eq!(changed.ends, [LineEnd::Circle, LineEnd::ClosedArrow]);
        assert!(is_arrow(&changed));
    }

    #[test]
    fn distance_is_measured_to_the_nearest_point_of_the_line() {
        let to = PagePoint { x: 200.0, y: 100.0 };
        assert!((distance(ORIGIN, to, PagePoint { x: 150.0, y: 103.0 }) - 3.0).abs() < 0.001);
        assert!((distance(ORIGIN, to, PagePoint { x: 204.0, y: 103.0 }) - 5.0).abs() < 0.001);
        assert!((distance(ORIGIN, ORIGIN, PagePoint { x: 103.0, y: 104.0 }) - 5.0).abs() < 0.001);
    }

    #[test]
    fn the_canvas_s_drawn_shape_parses_into_an_annotation() {
        let data = serde_json::json!({
            "page": 2,
            "kind": "arrow",
            "points": [[10.0, 20.0], [110.0, 20.0]],
            "color": [1.0, 0.0, 0.0],
            "opacity": 0.5,
        });
        let drawn = Drawn::parse(&data).expect("parses");
        assert_eq!(drawn.page, 2);
        assert_eq!(drawn.kind, Kind::Arrow);
        let arrow = drawn.annotation("me").expect("long enough");
        assert_eq!(arrow.style.color, Some([1.0, 0.0, 0.0]));
        assert!((arrow.style.opacity - 0.5).abs() < f32::EPSILON);
        assert!(
            Drawn::parse(
                &serde_json::json!({"page": 0, "kind": "blob", "points": [], "color": [0, 0, 0]})
            )
            .is_none()
        );
    }
}
