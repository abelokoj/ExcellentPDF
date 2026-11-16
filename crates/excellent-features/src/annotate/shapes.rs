//! Shapes drawn on a page: lines and arrows (A.8), with an ending at each end (`/LE`); rectangles and ellipses (A.9),
//! with a fill (`/IC`); and polygons, polylines, clouds, regular polygons and stars (A.10). Lines and boxes are
//! dragged from where the button went down to where it came up; polygons, polylines and clouds are clicked corner by
//! corner and finished with a double-click. Each has its border's width and dash pattern (`/BS`); a closed one can
//! have a cloudy border (`/BE`). Shift snaps a line to steps of 15° and makes a box square, as Acrobat does. MuPDF
//! draws them into their appearances, so they look the same in every reader.

use std::fmt;

use excellent_core::annot::{Annotation, Boxed, LineEnd, Path, Shape, Style, now};
use excellent_core::edit::Change;
use excellent_mupdf::{PagePoint, PageRect};
use serde_json::Value;

use super::markup::{color, floats, opacity};
use super::polygons;

/// What the drawing tool draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain line.
    Line,
    /// A line with an open arrowhead at its end.
    Arrow,
    /// A rectangle (`/Square`).
    Rectangle,
    /// An ellipse (`/Circle`).
    Ellipse,
    /// A regular polygon of this many sides, dragged out like a box (a `/Polygon`).
    Regular(u8),
    /// A star of this many points, dragged out like a box (a `/Polygon`).
    Star(u8),
    /// A polygon, clicked corner by corner.
    Polygon,
    /// A polyline, clicked point by point (`/PolyLine`).
    PolyLine,
    /// A polygon with a cloudy border, clicked corner by corner.
    Cloud,
}

impl Kind {
    /// The kinds named by a word alone.
    const NAMED: [Self; 7] = [
        Self::Line,
        Self::Arrow,
        Self::Rectangle,
        Self::Ellipse,
        Self::Polygon,
        Self::PolyLine,
        Self::Cloud,
    ];

    fn word(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Arrow => "arrow",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Regular(_) => "regular",
            Self::Star(_) => "star",
            Self::Polygon => "polygon",
            Self::PolyLine => "polyline",
            Self::Cloud => "cloud",
        }
    }

    /// The kind named `name`, as [`Display`](fmt::Display) writes it: a word, and for a regular polygon or a star its
    /// count after a colon (`star:5`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if let Some((word, count)) = name.split_once(':') {
            let count = count.parse().ok().filter(|n| polygons::SIDES.contains(n))?;
            return match word {
                "regular" => Some(Self::Regular(count)),
                "star" => Some(Self::Star(count)),
                _ => None,
            };
        }
        Self::NAMED.into_iter().find(|kind| kind.word() == name)
    }

    /// How a new one starts and ends; only an arrow has a head.
    #[must_use]
    pub fn ends(self) -> [LineEnd; 2] {
        if self == Self::Arrow {
            [LineEnd::None, LineEnd::OpenArrow]
        } else {
            [LineEnd::None; 2]
        }
    }

    /// Whether it is dragged out as a box, from one corner to the other.
    #[must_use]
    pub fn is_box(self) -> bool {
        matches!(
            self,
            Self::Rectangle | Self::Ellipse | Self::Regular(_) | Self::Star(_)
        )
    }

    /// Whether it is clicked point by point rather than dragged.
    #[must_use]
    pub fn is_clicked(self) -> bool {
        matches!(self, Self::Polygon | Self::PolyLine | Self::Cloud)
    }

    /// Whether it encloses an area, so can be filled and given a cloudy border.
    #[must_use]
    pub fn is_closed(self) -> bool {
        !self.has_ends()
    }

    /// Whether it has two ends, each of which can take an ending.
    #[must_use]
    pub fn has_ends(self) -> bool {
        matches!(self, Self::Line | Self::Arrow | Self::PolyLine)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regular(count) | Self::Star(count) => write!(f, "{}:{count}", self.word()),
            _ => f.write_str(self.word()),
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

/// A new shape's width, in points.
pub const WIDTH: f32 = 2.0;

/// The widths the inspector offers, in points.
pub const WIDTHS: [f32; 3] = [1.0, 2.0, 4.0];

/// The shortest line a drag makes, and the narrowest box, in points: anything smaller is a click, not a shape.
pub const MIN_LENGTH: f32 = 3.0;

/// Shift snaps a line's angle to multiples of this, in degrees.
const SNAP_DEGREES: f32 = 15.0;

/// The straight sides an ellipse is drawn with while it is dragged out.
const ELLIPSE_SIDES: usize = 48;

/// A cloud's bulges (`/BE /I`), from 0 to 2: Acrobat's.
pub const CLOUD_INTENSITY: f32 = 1.0;

/// Points of a clicked shape closer than this, in points, are one point: a double-click puts its point down twice.
const SAME_POINT: f32 = 0.5;

/// How a border is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dash {
    /// Unbroken.
    Solid,
    /// Dashes of 6 points with gaps of 4.
    Dashed,
    /// Dots of 2 points with gaps of 2.
    Dotted,
}

impl Dash {
    /// Every style, solid first.
    pub const ALL: [Self; 3] = [Self::Solid, Self::Dashed, Self::Dotted];

    /// Its dash pattern, dash and gap lengths in points (`/BS /D`); empty for solid.
    #[must_use]
    pub fn pattern(self) -> &'static [f32] {
        match self {
            Self::Solid => &[],
            Self::Dashed => &[6.0, 4.0],
            Self::Dotted => &[2.0, 2.0],
        }
    }

    /// The style closest to `pattern`: another app's pattern is shown as dashed.
    #[must_use]
    pub fn of(pattern: &[f32]) -> Self {
        Self::ALL
            .into_iter()
            .find(|dash| dash.pattern() == pattern)
            .unwrap_or(Self::Dashed)
    }
}

/// The fills a closed shape offers, none first: white and three light tints that keep dark text on them readable.
pub const FILLS: [Option<[f32; 3]>; 5] = [
    None,
    Some([1.0, 1.0, 1.0]),
    Some([1.0, 0.96, 0.62]),
    Some([0.78, 0.89, 1.0]),
    Some([1.0, 0.84, 0.84]),
];

/// A shape's border and fill, as the inspector shows them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// The border's width, in points.
    pub width: f32,
    /// How the border is drawn.
    pub dash: Dash,
    /// A closed shape's fill; `None` for none, and for a line.
    pub fill: Option<[f32; 3]>,
    /// Whether a closed shape's border is cloudy.
    pub cloud: bool,
}

/// What `annotation` is, when it is a shape: a line with an arrowhead is an arrow, a polygon with a cloudy border a
/// cloud. A regular polygon or a star, once drawn, is a polygon.
#[must_use]
pub fn kind_of(annotation: &Annotation) -> Option<Kind> {
    match annotation.shape {
        Shape::Line(..) if is_arrow(annotation) => Some(Kind::Arrow),
        Shape::Line(..) => Some(Kind::Line),
        Shape::Boxed(Boxed::Square, _) => Some(Kind::Rectangle),
        Shape::Boxed(Boxed::Circle, _) => Some(Kind::Ellipse),
        Shape::Path(Path::Polygon, _) if annotation.cloud.is_some() => Some(Kind::Cloud),
        Shape::Path(Path::Polygon, _) => Some(Kind::Polygon),
        Shape::Path(Path::PolyLine, _) => Some(Kind::PolyLine),
        _ => None,
    }
}

/// The shape `annotation`'s border and fill; `None` when it isn't a shape.
#[must_use]
pub fn look_of(annotation: &Annotation) -> Option<Look> {
    kind_of(annotation).map(|_| Look {
        width: annotation.style.width,
        dash: Dash::of(&annotation.dash),
        fill: annotation.fill,
        cloud: annotation.cloud.is_some(),
    })
}

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

/// `to`, moved so the box from `from` to it is a square: its longer side wins, each side keeping its direction.
#[must_use]
pub fn square(from: PagePoint, to: PagePoint) -> PagePoint {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let side = dx.abs().max(dy.abs());
    PagePoint {
        x: from.x + side.copysign(dx),
        y: from.y + side.copysign(dy),
    }
}

/// Where a drag of `kind` from `from` ends with the mouse at `to` while Shift is down: a line snapped to 15°, a box
/// made square.
#[must_use]
pub fn constrain(kind: Kind, from: PagePoint, to: PagePoint) -> PagePoint {
    if kind.is_box() {
        square(from, to)
    } else {
        snap(from, to)
    }
}

/// Whether a drag of `kind` from `from` to `to` draws one: a line at least [`MIN_LENGTH`] long, a box at least that
/// wide and high. A smaller drag is a slip of the hand.
#[must_use]
pub fn big_enough(kind: Kind, from: PagePoint, to: PagePoint) -> bool {
    if kind.is_box() {
        (to.x - from.x).abs() >= MIN_LENGTH && (to.y - from.y).abs() >= MIN_LENGTH
    } else {
        length(from, to) >= MIN_LENGTH
    }
}

/// The box with corners `a` and `b`.
#[must_use]
pub fn rect_between(a: PagePoint, b: PagePoint) -> PageRect {
    PageRect {
        x0: a.x.min(b.x),
        y0: a.y.min(b.y),
        x1: a.x.max(b.x),
        y1: a.y.max(b.y),
    }
}

/// The outline of a `kind` dragged from `from` to `to`, in page points, for the preview: a line's two points, or a
/// box's, ellipse's, polygon's or star's points, closed. A clicked kind's is the line between the two.
#[must_use]
pub fn outline(kind: Kind, from: PagePoint, to: PagePoint) -> Vec<PagePoint> {
    let rect = rect_between(from, to);
    let point = |x, y| PagePoint { x, y };
    let closed = |mut points: Vec<PagePoint>| {
        points.extend(points.first().copied());
        points
    };
    match kind {
        Kind::Line | Kind::Arrow | Kind::Polygon | Kind::PolyLine | Kind::Cloud => vec![from, to],
        Kind::Regular(sides) => closed(polygons::regular(rect, sides)),
        Kind::Star(points) => closed(polygons::star(rect, points)),
        Kind::Rectangle => vec![
            point(rect.x0, rect.y0),
            point(rect.x1, rect.y0),
            point(rect.x1, rect.y1),
            point(rect.x0, rect.y1),
            point(rect.x0, rect.y0),
        ],
        Kind::Ellipse => {
            let (cx, cy) = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
            let (rx, ry) = ((rect.x1 - rect.x0) / 2.0, (rect.y1 - rect.y0) / 2.0);
            (0..=ELLIPSE_SIDES)
                .map(|i| {
                    let angle = std::f32::consts::TAU * i as f32 / ELLIPSE_SIDES as f32;
                    point(cx + rx * angle.cos(), cy + ry * angle.sin())
                })
                .collect()
        }
    }
}

/// A new `kind` on page `page` dragged from `from` to `to`, `WIDTH` wide in `style`'s colour and opacity; `None` when
/// a line is shorter than [`MIN_LENGTH`] or a box narrower or lower, and for a kind that is clicked, not dragged.
#[must_use]
pub fn draw(
    page: usize,
    from: PagePoint,
    to: PagePoint,
    kind: Kind,
    style: Style,
    author: &str,
) -> Option<Annotation> {
    if !big_enough(kind, from, to) {
        return None;
    }
    let rect = rect_between(from, to);
    let shape = match kind {
        Kind::Line | Kind::Arrow => Shape::Line(from, to),
        Kind::Rectangle => Shape::Boxed(Boxed::Square, rect),
        Kind::Ellipse => Shape::Boxed(Boxed::Circle, rect),
        Kind::Regular(sides) => Shape::Path(Path::Polygon, polygons::regular(rect, sides)),
        Kind::Star(points) => Shape::Path(Path::Polygon, polygons::star(rect, points)),
        Kind::Polygon | Kind::PolyLine | Kind::Cloud => return None,
    };
    Some(made(page, shape, kind, style, author))
}

/// Whether the clicked `points` make a `kind`: a polyline needs two points, a polygon or a cloud three, after a
/// double-click's repeated point is dropped, spread at least [`MIN_LENGTH`] across.
#[must_use]
pub fn enough(kind: Kind, points: &[PagePoint]) -> bool {
    let points = polygons::deduped(points, SAME_POINT);
    let needed = if kind == Kind::PolyLine { 2 } else { 3 };
    kind.is_clicked()
        && points.len() >= needed
        && polygons::bounds(&points).is_some_and(|b| (b.x1 - b.x0).max(b.y1 - b.y0) >= MIN_LENGTH)
}

/// A new `kind` on page `page` through the clicked `points`, as [`draw`] makes one; `None` when they are not
/// [`enough`]. A cloud is a polygon with a cloudy border.
#[must_use]
pub fn trace(
    page: usize,
    points: &[PagePoint],
    kind: Kind,
    style: Style,
    author: &str,
) -> Option<Annotation> {
    if !enough(kind, points) {
        return None;
    }
    let points = polygons::deduped(points, SAME_POINT);
    let path = if kind == Kind::PolyLine {
        Path::PolyLine
    } else {
        Path::Polygon
    };
    let mut traced = made(page, Shape::Path(path, points), kind, style, author);
    if kind == Kind::Cloud {
        traced.cloud = Some(CLOUD_INTENSITY);
    }
    Some(traced)
}

/// A new `kind` shaped `shape`, `WIDTH` wide in `style`'s colour and opacity, with the kind's ends.
fn made(page: usize, shape: Shape, kind: Kind, style: Style, author: &str) -> Annotation {
    let mut drawn = Annotation::new(
        page,
        shape,
        Style {
            width: WIDTH,
            ..style
        },
        author,
    );
    drawn.ends = kind.ends();
    drawn
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

/// `annotation` changed by `edit` and stamped now, as one change; none when `edit` changed nothing.
fn changed(annotation: &Annotation, edit: impl FnOnce(&mut Annotation)) -> Option<Change> {
    let mut new = annotation.clone();
    edit(&mut new);
    (new != *annotation).then(|| {
        new.modified = now();
        Change::UpdateAnnotation(new)
    })
}

/// The change that gives the line or polyline `annotation` the endings `ends`, start first; none when it has them
/// already, or has no ends.
#[must_use]
pub fn set_ends(annotation: &Annotation, ends: [LineEnd; 2]) -> Option<Change> {
    if !kind_of(annotation)?.has_ends() {
        return None;
    }
    changed(annotation, |line| line.ends = ends)
}

/// The change that draws the shape `annotation`'s border `width` points wide; none when it is already, or isn't a
/// shape.
#[must_use]
pub fn set_width(annotation: &Annotation, width: f32) -> Option<Change> {
    kind_of(annotation)?;
    changed(annotation, |shape| shape.style.width = width)
}

/// The change that draws the shape `annotation`'s border `dash`; none when it is already, or isn't a shape.
#[must_use]
pub fn set_dash(annotation: &Annotation, dash: Dash) -> Option<Change> {
    kind_of(annotation)?;
    changed(annotation, |shape| shape.dash = dash.pattern().to_vec())
}

/// The change that fills the closed shape `annotation` with `fill` (`None`: empties it); none when it already is,
/// or encloses nothing.
#[must_use]
pub fn set_fill(annotation: &Annotation, fill: Option<[f32; 3]>) -> Option<Change> {
    if !kind_of(annotation)?.is_closed() {
        return None;
    }
    changed(annotation, |boxed| boxed.fill = fill)
}

/// The change that makes the closed shape `annotation`'s border cloudy, or plain; none when it already is, or
/// encloses nothing. Another app's intensity is kept while it stays cloudy.
#[must_use]
pub fn set_cloud(annotation: &Annotation, on: bool) -> Option<Change> {
    if !kind_of(annotation)?.is_closed() {
        return None;
    }
    changed(annotation, |shape| {
        shape.cloud = on.then(|| annotation.cloud.unwrap_or(CLOUD_INTENSITY));
    })
}

/// Whether a click at `point` is on the shape `annotation`, within `slop` points: on a line, on a border, or
/// anywhere inside a filled shape. `false` for anything else.
#[must_use]
pub fn hits(annotation: &Annotation, point: PagePoint, slop: f32) -> bool {
    // The whole width, not half: a thin border is hard to hit.
    let reach = slop + annotation.style.width;
    match annotation.shape {
        Shape::Line(from, to) => distance(from, to, point) <= reach,
        Shape::Path(path, ref points) => {
            let closed = path == Path::Polygon;
            polygons::distance(points, closed, point) <= reach
                || (closed && annotation.fill.is_some() && polygons::inside(points, point))
        }
        Shape::Boxed(Boxed::Square, rect) => {
            let inside =
                (rect.x0..=rect.x1).contains(&point.x) && (rect.y0..=rect.y1).contains(&point.y);
            let near = (rect.x0 - reach..=rect.x1 + reach).contains(&point.x)
                && (rect.y0 - reach..=rect.y1 + reach).contains(&point.y);
            let edge = [
                point.x - rect.x0,
                rect.x1 - point.x,
                point.y - rect.y0,
                rect.y1 - point.y,
            ]
            .into_iter()
            .any(|gap| gap.abs() <= reach);
            near && ((inside && annotation.fill.is_some()) || edge)
        }
        Shape::Boxed(Boxed::Circle, rect) => {
            let (rx, ry) = ((rect.x1 - rect.x0) / 2.0, (rect.y1 - rect.y0) / 2.0);
            if rx <= 0.0 || ry <= 0.0 {
                return false;
            }
            let (dx, dy) = (
                point.x - (rect.x0 + rect.x1) / 2.0,
                point.y - (rect.y0 + rect.y1) / 2.0,
            );
            // How far out along its ray the point is, 1 on the outline; the gap scaled by the nearer radius.
            let along = (dx / rx).hypot(dy / ry);
            (along <= 1.0 && annotation.fill.is_some()) || (along - 1.0).abs() * rx.min(ry) <= reach
        }
        _ => false,
    }
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
    /// Its points, in page points: where a drag started and where it ended, or each point clicked.
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

    /// The annotation it makes, by `author`; `None` when it has too few points or is too small.
    #[must_use]
    pub fn annotation(&self, author: &str) -> Option<Annotation> {
        let style = Style {
            color: Some(self.color),
            opacity: self.opacity,
            ..Style::default()
        };
        if self.kind.is_clicked() {
            return trace(self.page, &self.points, self.kind, style, author);
        }
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

    fn at(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
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
    fn shift_makes_a_box_square_whichever_way_it_is_dragged() {
        assert!(close(square(ORIGIN, at(160.0, 120.0)), at(160.0, 160.0)));
        assert!(close(square(ORIGIN, at(80.0, 30.0)), at(30.0, 30.0)));
        assert!(close(
            constrain(Kind::Ellipse, ORIGIN, at(90.0, 150.0)),
            at(50.0, 150.0)
        ));
        let line_end = constrain(Kind::Line, ORIGIN, at(200.0, 104.0));
        assert!(
            (line_end.y - ORIGIN.y).abs() < 0.001,
            "a line snaps instead"
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
    fn a_drag_makes_a_box_from_corner_to_corner_and_a_thin_one_makes_nothing() {
        let ellipse = draw(
            1,
            at(160.0, 180.0),
            ORIGIN,
            Kind::Ellipse,
            Style::default(),
            "me",
        )
        .expect("big enough");
        assert_eq!(
            ellipse.shape,
            Shape::Boxed(Boxed::Circle, rect_between(ORIGIN, at(160.0, 180.0)))
        );
        assert_eq!(ellipse.ends, [LineEnd::None; 2]);
        assert_eq!(kind_of(&ellipse), Some(Kind::Ellipse));
        let rectangle = draw(
            1,
            ORIGIN,
            at(160.0, 180.0),
            Kind::Rectangle,
            Style::default(),
            "me",
        )
        .expect("big enough");
        assert_eq!(kind_of(&rectangle), Some(Kind::Rectangle));
        assert!(
            draw(
                1,
                ORIGIN,
                at(200.0, 101.0),
                Kind::Rectangle,
                Style::default(),
                "me"
            )
            .is_none(),
            "a box one point high is a slip"
        );
    }

    #[test]
    fn the_preview_outlines_a_closed_box_and_ellipse() {
        let rectangle = outline(Kind::Rectangle, ORIGIN, at(140.0, 120.0));
        assert_eq!(rectangle.len(), 5);
        assert_eq!(rectangle.first(), rectangle.last());
        let ellipse = outline(Kind::Ellipse, ORIGIN, at(140.0, 120.0));
        assert!(close(ellipse[0], at(140.0, 110.0)));
        assert!(close(ellipse[0], *ellipse.last().expect("points")));
        assert!(ellipse.iter().all(|point| {
            let (dx, dy) = ((point.x - 120.0) / 20.0, (point.y - 110.0) / 10.0);
            (dx.hypot(dy) - 1.0).abs() < 0.001
        }));
        assert_eq!(
            outline(Kind::Arrow, ORIGIN, at(1.0, 2.0)),
            vec![ORIGIN, at(1.0, 2.0)]
        );
    }

    #[test]
    fn a_kind_round_trips_through_its_name_with_its_count() {
        for kind in [
            Kind::Star(7),
            Kind::Regular(3),
            Kind::Cloud,
            Kind::PolyLine,
            Kind::Arrow,
        ] {
            assert_eq!(Kind::parse(&kind.to_string()), Some(kind));
        }
        assert_eq!(Kind::Star(7).to_string(), "star:7");
        assert_eq!(Kind::parse("star:2"), None, "fewer points than a star has");
        assert_eq!(Kind::parse("polygon:5"), None);
    }

    #[test]
    fn a_dragged_star_or_polygon_is_a_polygon_annotation() {
        let star = draw(
            0,
            ORIGIN,
            at(200.0, 200.0),
            Kind::Star(5),
            Style::default(),
            "me",
        )
        .expect("big enough");
        let Shape::Path(Path::Polygon, ref corners) = star.shape else {
            panic!("a polygon, not {:?}", star.shape);
        };
        assert_eq!(corners.len(), 10);
        assert_eq!(kind_of(&star), Some(Kind::Polygon));
        assert_eq!(
            outline(Kind::Regular(6), ORIGIN, at(200.0, 200.0)).len(),
            7,
            "six corners and back to the first"
        );
    }

    #[test]
    fn clicked_points_make_a_polygon_a_polyline_or_a_cloud() {
        let points = [at(0.0, 0.0), at(50.0, 0.0), at(50.0, 40.0), at(50.2, 40.1)];
        let cloud = trace(0, &points, Kind::Cloud, Style::default(), "me").expect("three corners");
        assert_eq!(
            cloud.shape,
            Shape::Path(Path::Polygon, points[..3].to_vec()),
            "the double-click's repeat dropped"
        );
        assert_eq!(cloud.cloud, Some(CLOUD_INTENSITY));
        assert_eq!(kind_of(&cloud), Some(Kind::Cloud));
        let polyline =
            trace(0, &points[..2], Kind::PolyLine, Style::default(), "me").expect("two points");
        assert_eq!(kind_of(&polyline), Some(Kind::PolyLine));
        assert!(trace(0, &points[..2], Kind::Polygon, Style::default(), "me").is_none());
        assert!(!enough(Kind::Line, &points), "a line is dragged");
        assert!(
            draw(
                0,
                ORIGIN,
                at(200.0, 200.0),
                Kind::Polygon,
                Style::default(),
                "me"
            )
            .is_none()
        );
        let Some(Change::UpdateAnnotation(plain)) = set_cloud(&cloud, false) else {
            panic!("one update");
        };
        assert_eq!(kind_of(&plain), Some(Kind::Polygon));
        assert!(
            set_cloud(&polyline, true).is_none(),
            "a polyline encloses nothing"
        );
        assert!(set_ends(&polyline, [LineEnd::None, LineEnd::OpenArrow]).is_some());
        assert!(set_ends(&cloud, [LineEnd::None, LineEnd::OpenArrow]).is_none());
    }

    #[test]
    fn a_click_hits_a_polygon_s_sides_and_a_filled_one_s_inside() {
        let corners = [at(0.0, 0.0), at(100.0, 0.0), at(0.0, 100.0)];
        let mut polygon =
            trace(0, &corners, Kind::Polygon, Style::default(), "me").expect("enough");
        assert!(hits(&polygon, at(-1.0, 50.0), 1.5), "on its closing side");
        assert!(!hits(&polygon, at(20.0, 20.0), 1.5));
        polygon.fill = FILLS[1];
        assert!(hits(&polygon, at(20.0, 20.0), 1.5));
        let polyline = trace(0, &corners, Kind::PolyLine, Style::default(), "me").expect("enough");
        assert!(
            !hits(&polyline, at(-4.0, 50.0), 1.5),
            "a polyline isn't closed"
        );
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
    fn width_dash_and_fill_are_one_change_each_and_a_line_takes_no_fill() {
        let boxed = draw(
            0,
            ORIGIN,
            at(200.0, 200.0),
            Kind::Rectangle,
            Style::default(),
            "me",
        )
        .expect("big enough");
        assert!(set_width(&boxed, WIDTH).is_none());
        let Some(Change::UpdateAnnotation(wide)) = set_width(&boxed, 4.0) else {
            panic!("one update");
        };
        assert_eq!(wide.style.width, 4.0);
        let Some(Change::UpdateAnnotation(dotted)) = set_dash(&boxed, Dash::Dotted) else {
            panic!("one update");
        };
        assert_eq!(dotted.dash, vec![2.0, 2.0]);
        assert_eq!(look_of(&dotted).map(|look| look.dash), Some(Dash::Dotted));
        assert!(set_dash(&boxed, Dash::Solid).is_none());
        let Some(Change::UpdateAnnotation(filled)) = set_fill(&boxed, FILLS[2]) else {
            panic!("one update");
        };
        assert_eq!(filled.fill, FILLS[2]);
        let line = draw(
            0,
            ORIGIN,
            at(200.0, 100.0),
            Kind::Line,
            Style::default(),
            "me",
        )
        .expect("long enough");
        assert!(set_fill(&line, FILLS[1]).is_none(), "a line has no inside");
        assert!(set_dash(&line, Dash::Dashed).is_some());
        assert_eq!(Dash::of(&[3.0]), Dash::Dashed, "another app's pattern");
    }

    #[test]
    fn a_click_hits_a_box_s_border_and_a_filled_box_s_inside() {
        let mut boxed = draw(
            0,
            ORIGIN,
            at(200.0, 200.0),
            Kind::Rectangle,
            Style::default(),
            "me",
        )
        .expect("big enough");
        assert!(hits(&boxed, at(100.5, 150.0), 1.5), "on the left side");
        assert!(!hits(&boxed, at(150.0, 150.0), 1.5), "inside an empty box");
        assert!(!hits(&boxed, at(90.0, 150.0), 1.5), "outside");
        boxed.fill = FILLS[1];
        assert!(hits(&boxed, at(150.0, 150.0), 1.5), "inside a filled box");
        let ellipse = draw(
            0,
            ORIGIN,
            at(200.0, 150.0),
            Kind::Ellipse,
            Style::default(),
            "me",
        )
        .expect("big enough");
        assert!(hits(&ellipse, at(150.0, 100.5), 1.5), "on its top");
        assert!(!hits(&ellipse, at(150.0, 125.0), 1.5), "its empty middle");
        assert!(!hits(&ellipse, at(101.0, 101.0), 1.5), "its box's corner");
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
        let ellipse = serde_json::json!({
            "page": 0,
            "kind": "ellipse",
            "points": [[10.0, 20.0], [110.0, 80.0]],
            "color": [0.0, 0.0, 1.0],
        });
        let drawn = Drawn::parse(&ellipse).expect("parses");
        assert_eq!(
            drawn.annotation("me").map(|e| e.shape),
            Some(Shape::Boxed(
                Boxed::Circle,
                rect_between(at(10.0, 20.0), at(110.0, 80.0))
            ))
        );
    }
}
