//! Calibrated vector measurement tools: distance, perimeter, area, angle, `/VP`/`/Measure`
//! parsing & serialization, and vector endpoint/intersection snapping (plan A.23).

use std::fmt;

use excellent_core::annot::{Annotation, LineEnd, Path as AnnotPath, Shape, Style};
use excellent_mupdf::{PagePoint, PageRect};

/// Error returned by measurement calibration or geometry validation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum MeasureError {
    /// Two points are coincident or degenerate for calibration.
    DegenerateGeometry(String),
    /// A scale factor or known length was not positive and finite.
    InvalidScale(String),
}

impl fmt::Display for MeasureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DegenerateGeometry(msg) => write!(f, "degenerate measurement geometry: {msg}"),
            Self::InvalidScale(msg) => write!(f, "invalid measurement scale: {msg}"),
        }
    }
}

impl std::error::Error for MeasureError {}

/// Convenience result alias for measurement operations.
pub type Result<T> = std::result::Result<T, MeasureError>;

/// Measurement tool type matching ISO 32000 PDF annotation intents (plan A.23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MeasureTool {
    /// Linear distance (`PDF_ANNOT_IT_LINE_DIMENSION`).
    #[default]
    Distance,
    /// Open/closed perimeter (`PDF_ANNOT_IT_POLYLINE_DIMENSION`).
    Perimeter,
    /// Enclosed polygon area (`PDF_ANNOT_IT_POLYGON_DIMENSION`).
    Area,
    /// Angle between two rays sharing a vertex.
    Angle,
}

impl MeasureTool {
    /// PDF `/IT` (Intent) name.
    #[must_use]
    pub const fn pdf_intent(self) -> &'static str {
        match self {
            Self::Distance => "LineDimension",
            Self::Perimeter => "PolyLineDimension",
            Self::Area => "PolygonDimension",
            Self::Angle => "AngleDimension",
        }
    }
}

/// Linear measurement units (metric & imperial) (plan A.23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LengthUnit {
    /// PDF points (`1/72 in`).
    Points,
    /// Millimeters.
    #[default]
    Millimeters,
    /// Centimeters.
    Centimeters,
    /// Meters.
    Meters,
    /// Inches.
    Inches,
    /// Feet.
    Feet,
}

impl LengthUnit {
    /// Short unit suffix (`mm`, `cm`, `m`, `in`, `ft`, `pt`).
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Points => "pt",
            Self::Millimeters => "mm",
            Self::Centimeters => "cm",
            Self::Meters => "m",
            Self::Inches => "in",
            Self::Feet => "ft",
        }
    }

    /// PDF points per 1 unit at `1:1` drawing scale (`72 pt/in`).
    #[must_use]
    pub const fn points_per_unit_1_to_1(self) -> f64 {
        match self {
            Self::Points => 1.0,
            Self::Inches => 72.0,
            Self::Feet => 864.0,
            Self::Millimeters => 72.0 / 25.4,
            Self::Centimeters => 720.0 / 25.4,
            Self::Meters => 72_000.0 / 25.4,
        }
    }
}

/// Calibrated scale ratio and optional `/VP` viewport rectangle (plan A.23).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportScale {
    /// Optional viewport bounds in page points (`None` = whole page).
    pub viewport_rect: Option<PageRect>,
    /// Target unit for linear measurements (area uses `unit²`).
    pub unit: LengthUnit,
    /// Calibrated conversion factor: how many PDF points correspond to 1.0 `unit` in real-world
    /// drawing space (e.g., `1:100` in meters -> `points_per_unit = (72 / 0.0254) / 100.0`).
    pub points_per_unit: f64,
    /// Optional label (e.g. `"1:100"` or `"1 in = 10 ft"`).
    pub scale_label: String,
    /// Display decimal precision (`0..=6`).
    pub precision: u8,
}

impl Default for ViewportScale {
    fn default() -> Self {
        Self {
            viewport_rect: None,
            unit: LengthUnit::Millimeters,
            points_per_unit: LengthUnit::Millimeters.points_per_unit_1_to_1(),
            scale_label: "1:1".into(),
            precision: 2,
        }
    }
}

impl ViewportScale {
    /// Creates a scale from an engineering ratio (e.g. `ratio = 100.0` for `1:100`, `unit = Meters`).
    ///
    /// # Errors
    ///
    /// Returns [`MeasureError::InvalidScale`] if `drawing_to_real_ratio <= 0.0` or non-finite.
    pub fn from_ratio(unit: LengthUnit, drawing_to_real_ratio: f64) -> Result<Self> {
        if !drawing_to_real_ratio.is_finite() || drawing_to_real_ratio <= 0.0 {
            return Err(MeasureError::InvalidScale(
                "scale ratio must be finite and positive".into(),
            ));
        }
        Ok(Self {
            viewport_rect: None,
            unit,
            points_per_unit: unit.points_per_unit_1_to_1() / drawing_to_real_ratio,
            scale_label: format!("1:{drawing_to_real_ratio}"),
            precision: 2,
        })
    }

    /// Calibrates a scale from two clicked points on the page and their known real-world length
    /// (plan A.23).
    ///
    /// # Errors
    ///
    /// Returns [`MeasureError`] if the two points coincide or `known_length <= 0.0`.
    pub fn calibrate(
        p1: PagePoint,
        p2: PagePoint,
        known_length: f64,
        unit: LengthUnit,
    ) -> Result<Self> {
        if !known_length.is_finite() || known_length <= 0.0 {
            return Err(MeasureError::InvalidScale(
                "known calibration length must be finite and positive".into(),
            ));
        }
        let dist_pt = f64::from((p2.x - p1.x).hypot(p2.y - p1.y));
        if dist_pt < 0.01 {
            return Err(MeasureError::DegenerateGeometry(
                "calibration points must be distinct".into(),
            ));
        }
        Ok(Self {
            viewport_rect: None,
            unit,
            points_per_unit: dist_pt / known_length,
            scale_label: format!("Calibrated ({known_length} {})", unit.suffix()),
            precision: 2,
        })
    }

    /// Converts a PDF distance in points to calibrated real-world units.
    #[must_use]
    pub fn points_to_units(&self, distance_pt: f64) -> f64 {
        distance_pt / self.points_per_unit.max(1e-9)
    }

    /// Converts a PDF area in `pt²` to calibrated `unit²`.
    #[must_use]
    pub fn sq_points_to_sq_units(&self, area_pt2: f64) -> f64 {
        let ppu = self.points_per_unit.max(1e-9);
        area_pt2 / (ppu * ppu)
    }

    /// Serializes a PDF `/Measure` dictionary string (ISO 32000-1 §12.9) so Acrobat and other
    /// readers display the calibrated scale (plan A.23).
    #[must_use]
    pub fn to_pdf_measure_dict(&self) -> String {
        // Acrobat `/R` display string + `/X` conversion factor (units per 1 PDF point).
        let x_factor = 1.0 / self.points_per_unit.max(1e-9);
        let a_factor = x_factor * x_factor;
        let suffix = self.unit.suffix();
        format!(
            "<< /Type /Measure /Subtype /RL /R ({}) /X [<< /Type /NumberFormat /U ({suffix}) /C {x_factor:.8} /D 100 >>] /D [<< /Type /NumberFormat /U ({suffix}) /C {x_factor:.8} /D 100 >>] /A [<< /Type /NumberFormat /U (sq {suffix}) /C {a_factor:.10} /D 100 >>] >>",
            self.scale_label
        )
    }

    /// Parses a PDF `/Measure` dictionary string back into a [`ViewportScale`] (plan A.23).
    #[must_use]
    pub fn from_pdf_measure_dict(dict_str: &str, viewport_rect: Option<PageRect>) -> Option<Self> {
        let unit = extract_paren_after(dict_str, "/U")
            .as_deref()
            .and_then(parse_unit_suffix)
            .unwrap_or(LengthUnit::Millimeters);
        let c_pos = dict_str.find("/C ")?;
        let c_val: f64 = dict_str[c_pos + 3..]
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        if !c_val.is_finite() || c_val <= 0.0 {
            return None;
        }
        let label = extract_paren_after(dict_str, "/R").unwrap_or_else(|| "Custom".into());
        Some(Self {
            viewport_rect,
            unit,
            points_per_unit: 1.0 / c_val,
            scale_label: label,
            precision: 2,
        })
    }
}

fn parse_unit_suffix(u: &str) -> Option<LengthUnit> {
    match u.trim() {
        "pt" => Some(LengthUnit::Points),
        "mm" => Some(LengthUnit::Millimeters),
        "cm" => Some(LengthUnit::Centimeters),
        "m" => Some(LengthUnit::Meters),
        "in" => Some(LengthUnit::Inches),
        "ft" => Some(LengthUnit::Feet),
        _ => None,
    }
}

fn extract_paren_after(s: &str, key: &str) -> Option<String> {
    let pos = s.find(key)?;
    let rest = s[pos + key.len()..].trim_start();
    let inner = rest.strip_prefix('(')?;
    let end = inner.find(')')?;
    Some(inner[..end].to_owned())
}

/// Computes Euclidean distance in calibrated units between `a` and `b` (plan A.23).
#[must_use]
pub fn measure_distance(a: PagePoint, b: PagePoint, scale: &ViewportScale) -> f64 {
    let pt = f64::from((b.x - a.x).hypot(b.y - a.y));
    scale.points_to_units(pt)
}

/// Computes polyline perimeter in calibrated units across `vertices` (plan A.23).
#[must_use]
pub fn measure_perimeter(vertices: &[PagePoint], scale: &ViewportScale) -> f64 {
    let pt: f64 = vertices
        .windows(2)
        .map(|w| f64::from((w[1].x - w[0].x).hypot(w[1].y - w[0].y)))
        .sum();
    scale.points_to_units(pt)
}

/// Computes polygon area in calibrated `unit²` using the shoelace formula (plan A.23).
#[must_use]
pub fn measure_area(vertices: &[PagePoint], scale: &ViewportScale) -> f64 {
    if vertices.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for i in 0..vertices.len() {
        let p = vertices[i];
        let q = vertices[(i + 1) % vertices.len()];
        sum += f64::from(p.x) * f64::from(q.y) - f64::from(q.x) * f64::from(p.y);
    }
    scale.sq_points_to_sq_units(0.5 * sum.abs())
}

/// Computes interior angle in degrees (`0.0..=180.0`) at `vertex` between rays `vertex -> a` and
/// `vertex -> b` (plan A.23).
#[must_use]
pub fn measure_angle_deg(a: PagePoint, vertex: PagePoint, b: PagePoint) -> f64 {
    let ax = f64::from(a.x - vertex.x);
    let ay = f64::from(a.y - vertex.y);
    let bx = f64::from(b.x - vertex.x);
    let by = f64::from(b.y - vertex.y);
    let la = ax.hypot(ay);
    let lb = bx.hypot(by);
    if la < 1e-6 || lb < 1e-6 {
        return 0.0;
    }
    let cos = ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0);
    cos.acos().to_degrees()
}

/// Formats a measurement caption string at `precision` decimal places (plan A.23).
#[must_use]
pub fn format_measurement(tool: MeasureTool, value: f64, scale: &ViewportScale) -> String {
    let p = usize::from(scale.precision.min(6));
    match tool {
        MeasureTool::Distance | MeasureTool::Perimeter => {
            format!("{value:.p$} {}", scale.unit.suffix())
        }
        MeasureTool::Area => {
            format!("{value:.p$} sq {}", scale.unit.suffix())
        }
        MeasureTool::Angle => {
            format!("{value:.p$}°")
        }
    }
}

/// Converts a completed measurement into a PDF [`Annotation`] with caption text (plan A.23).
#[must_use]
pub fn to_annotation(
    page: usize,
    tool: MeasureTool,
    points: &[PagePoint],
    scale: &ViewportScale,
    author: &str,
) -> Option<Annotation> {
    let style = Style {
        color: Some([0.85, 0.2, 0.15]),
        opacity: 1.0,
        width: 1.5,
    };
    let (shape, value, ends) = match tool {
        MeasureTool::Distance => {
            let [a, b] = points[..] else {
                return None;
            };
            let v = measure_distance(a, b, scale);
            (
                Shape::Line(a, b),
                v,
                [LineEnd::OpenArrow, LineEnd::OpenArrow],
            )
        }
        MeasureTool::Perimeter => {
            if points.len() < 2 {
                return None;
            }
            let v = measure_perimeter(points, scale);
            (
                Shape::Path(AnnotPath::PolyLine, points.to_vec()),
                v,
                [LineEnd::None, LineEnd::None],
            )
        }
        MeasureTool::Area => {
            if points.len() < 3 {
                return None;
            }
            let v = measure_area(points, scale);
            (
                Shape::Path(AnnotPath::Polygon, points.to_vec()),
                v,
                [LineEnd::None, LineEnd::None],
            )
        }
        MeasureTool::Angle => {
            let [a, vertex, b] = points[..] else {
                return None;
            };
            let v = measure_angle_deg(a, vertex, b);
            (
                Shape::Path(AnnotPath::PolyLine, vec![a, vertex, b]),
                v,
                [LineEnd::OpenArrow, LineEnd::OpenArrow],
            )
        }
    };
    let mut annot = Annotation::new(page, shape, style, author);
    annot.ends = ends;
    annot.contents = format_measurement(tool, value, scale);
    annot.tags.push(format!("measure:{}", tool.pdf_intent()));
    Some(annot)
}

// ---------------------------------------------------------------------------
// Vector snapping (endpoints, midpoints, intersections)
// ---------------------------------------------------------------------------

/// A vector line segment extracted from PDF paths for snapping (plan A.23).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorSegment {
    /// Segment start point in page points.
    pub start: PagePoint,
    /// Segment end point in page points.
    pub end: PagePoint,
}

/// Kind of vector snap target matched near the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SnapKind {
    /// Segment endpoint.
    Endpoint,
    /// Segment midpoint.
    Midpoint,
    /// Intersection of two segments.
    Intersection,
}

/// Result of [`snap_to_vector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapHit {
    /// Snapped coordinates in page points.
    pub point: PagePoint,
    /// Type of geometric feature snapped to.
    pub kind: SnapKind,
    /// Distance in points from the cursor to the snapped point.
    pub distance_pt: f32,
}

/// Snaps `cursor` to the nearest vector endpoint, midpoint, or segment intersection within
/// `threshold_pt` (plan A.23).
#[must_use]
pub fn snap_to_vector(
    cursor: PagePoint,
    segments: &[VectorSegment],
    threshold_pt: f32,
) -> Option<SnapHit> {
    let mut best: Option<SnapHit> = None;
    let mut consider = |pt: PagePoint, kind: SnapKind| {
        let d = (pt.x - cursor.x).hypot(pt.y - cursor.y);
        if d <= threshold_pt && best.is_none_or(|b| d < b.distance_pt) {
            best = Some(SnapHit {
                point: pt,
                kind,
                distance_pt: d,
            });
        }
    };

    for seg in segments {
        consider(seg.start, SnapKind::Endpoint);
        consider(seg.end, SnapKind::Endpoint);
        consider(
            PagePoint {
                x: (seg.start.x + seg.end.x) * 0.5,
                y: (seg.start.y + seg.end.y) * 0.5,
            },
            SnapKind::Midpoint,
        );
    }

    // Check pairwise intersections (bounded for interactive latency).
    let limit = segments.len().min(256);
    for i in 0..limit {
        for j in (i + 1)..limit {
            if let Some(ix) = segment_intersection(segments[i], segments[j]) {
                consider(ix, SnapKind::Intersection);
            }
        }
    }

    best
}

fn segment_intersection(a: VectorSegment, b: VectorSegment) -> Option<PagePoint> {
    let x1 = f64::from(a.start.x);
    let y1 = f64::from(a.start.y);
    let x2 = f64::from(a.end.x);
    let y2 = f64::from(a.end.y);
    let x3 = f64::from(b.start.x);
    let y3 = f64::from(b.start.y);
    let x4 = f64::from(b.end.x);
    let y4 = f64::from(b.end.y);

    let denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / denom;
    let u = -((x1 - x2) * (y1 - y3) - (y1 - y2) * (x1 - x3)) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(PagePoint {
            x: (x1 + t * (x2 - x1)) as f32,
            y: (y1 + t * (y2 - y1)) as f32,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_distance_perimeter_area_angle_within_half_percent() {
        // 1:100 architectural drawing in meters: 72 pt = 1 in = 0.0254 m on paper = 2.54 m real.
        let scale = ViewportScale::from_ratio(LengthUnit::Meters, 100.0).expect("scale");
        let a = PagePoint { x: 0.0, y: 0.0 };
        let b = PagePoint { x: 72.0, y: 0.0 };
        let c = PagePoint { x: 72.0, y: 72.0 };
        let d = PagePoint { x: 0.0, y: 72.0 };

        let dist = measure_distance(a, b, &scale);
        assert!((dist - 2.54).abs() / 2.54 < 0.005, "dist={dist}");

        let perim = measure_perimeter(&[a, b, c, d, a], &scale);
        assert!((perim - 10.16).abs() / 10.16 < 0.005, "perim={perim}");

        let area = measure_area(&[a, b, c, d], &scale);
        assert!(
            (area - (2.54 * 2.54)).abs() / (2.54 * 2.54) < 0.005,
            "area={area}"
        );

        let angle = measure_angle_deg(a, b, c);
        assert!((angle - 90.0).abs() < 0.01, "angle={angle}");

        // Calibration by known length + /Measure PDF round-trip.
        let cal = ViewportScale::calibrate(a, b, 5.0, LengthUnit::Feet).expect("cal");
        assert!((measure_distance(a, b, &cal) - 5.0).abs() < 0.001);
        let dict = cal.to_pdf_measure_dict();
        let parsed = ViewportScale::from_pdf_measure_dict(&dict, None).expect("parse /Measure");
        assert!((measure_distance(a, b, &parsed) - 5.0).abs() < 0.001);

        let ann =
            to_annotation(0, MeasureTool::Distance, &[a, b], &cal, "Architect").expect("annot");
        assert!(ann.contents.contains("5.00 ft"));
    }

    #[test]
    fn vector_snap_finds_endpoints_midpoints_and_intersections() {
        let segs = [
            VectorSegment {
                start: PagePoint { x: 0.0, y: 0.0 },
                end: PagePoint { x: 100.0, y: 100.0 },
            },
            VectorSegment {
                start: PagePoint { x: 0.0, y: 100.0 },
                end: PagePoint { x: 100.0, y: 0.0 },
            },
        ];
        let hit = snap_to_vector(PagePoint { x: 51.0, y: 49.5 }, &segs, 6.0).expect("snap");
        assert!((hit.point.x - 50.0).abs() < 0.01 && (hit.point.y - 50.0).abs() < 0.01);

        let end_hit = snap_to_vector(PagePoint { x: 98.0, y: 99.0 }, &segs, 6.0).expect("end");
        assert_eq!(end_hit.kind, SnapKind::Endpoint);
    }
}
