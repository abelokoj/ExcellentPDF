//! Fill and gradients for closed strokes and shapes (H.16, plan §27.7): generates ISO 32000-1 compliant PDF
//! appearance streams (Form XObjects) with solid fills, axial (linear, ShadingType 2) gradients, and radial
//! (ShadingType 3) gradients so filled strokes render identically in Acrobat, PDF-XChange, pdf.js, and MuPDF.
//!
//! Coordinates passed in are in page points (`x` right, `y` down) and are converted to standard PDF bottom-left user
//! space using the page height when emitting PDF operators.

use std::fmt::{self, Write as _};

use crate::erase::Point;

/// Closure threshold: ends closer than this share of path length count as a closed stroke (matches H.10).
const CLOSED_RATIO: f32 = 0.18;

/// An error validating or building a fill appearance stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillError {
    /// Fewer than three distinct points were supplied for the closed path.
    TooFewPoints,
    /// A gradient has fewer than two color stops.
    TooFewStops,
    /// A gradient stop offset is not finite or outside `0.0..=1.0`, or stops are not monotonically increasing.
    InvalidStops,
    /// A radial gradient has a negative radius or degenerate coincident circles.
    InvalidRadialGeometry,
    /// A linear gradient has identical start and end points.
    DegenerateAxis,
    /// A non-positive or non-finite page height was supplied.
    InvalidPageHeight,
}

impl fmt::Display for FillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewPoints => f.write_str("a filled path requires at least three points"),
            Self::TooFewStops => f.write_str("a gradient requires at least two color stops"),
            Self::InvalidStops => f.write_str("gradient stops must be finite and ordered in 0..=1"),
            Self::InvalidRadialGeometry => {
                f.write_str("radial gradient radii must be non-negative and non-degenerate")
            }
            Self::DegenerateAxis => {
                f.write_str("linear gradient start and end points must be distinct")
            }
            Self::InvalidPageHeight => f.write_str("page height must be positive and finite"),
        }
    }
}

impl std::error::Error for FillError {}

/// Winding rule used to fill or clip the path (`f`/`W` vs `f*`/`W*` in PDF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FillRule {
    /// Non-zero winding number rule (PDF `f` / `W`).
    #[default]
    NonZero,
    /// Even-odd rule (PDF `f*` / `W*`).
    EvenOdd,
}

/// How a gradient extends beyond its start and end coordinates (PDF `/Extend [bool bool]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExtendMode {
    /// Do not paint outside `[0, 1]`.
    None,
    /// Extend only before the start point.
    StartOnly,
    /// Extend only past the end point.
    EndOnly,
    /// Extend beyond both ends (standard CSS / Acrobat default).
    #[default]
    Both,
}

impl ExtendMode {
    fn pdf_array(self) -> &'static str {
        match self {
            Self::None => "[false false]",
            Self::StartOnly => "[true false]",
            Self::EndOnly => "[false true]",
            Self::Both => "[true true]",
        }
    }
}

/// One color stop in a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    /// Position along the gradient axis, `0.0..=1.0`.
    pub offset: f32,
    /// DeviceRGB color, each channel in `0.0..=1.0`.
    pub color: [f32; 3],
}

impl ColorStop {
    /// Constructs a color stop, clamping RGB channels to `0.0..=1.0`.
    #[must_use]
    pub fn new(offset: f32, color: [f32; 3]) -> Self {
        Self {
            offset,
            color: clamp_rgb(color),
        }
    }
}

/// A solid color fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolidFill {
    /// DeviceRGB color, each channel in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Opacity, `0.0..=1.0`.
    pub opacity: f32,
}

impl SolidFill {
    /// Constructs a solid fill with clamped color and opacity.
    #[must_use]
    pub fn new(color: [f32; 3], opacity: f32) -> Self {
        Self {
            color: clamp_rgb(color),
            opacity: clamp_unit(opacity),
        }
    }
}

/// An axial (linear) gradient fill (PDF ShadingType 2).
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    /// Start point of the gradient axis in page points.
    pub start: Point,
    /// End point of the gradient axis in page points.
    pub end: Point,
    /// Ordered color stops from `0.0` to `1.0` (at least two).
    pub stops: Vec<ColorStop>,
    /// Extension behavior beyond start/end.
    pub extend: ExtendMode,
    /// Overall fill opacity, `0.0..=1.0`.
    pub opacity: f32,
}

/// A radial gradient fill (PDF ShadingType 3).
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    /// Inner circle center in page points.
    pub start_center: Point,
    /// Inner circle radius in page points (`>= 0`).
    pub start_radius: f32,
    /// Outer circle center in page points.
    pub end_center: Point,
    /// Outer circle radius in page points (`>= 0`).
    pub end_radius: f32,
    /// Ordered color stops from `0.0` to `1.0` (at least two).
    pub stops: Vec<ColorStop>,
    /// Extension behavior beyond the inner/outer circles.
    pub extend: ExtendMode,
    /// Overall fill opacity, `0.0..=1.0`.
    pub opacity: f32,
}

/// Fill style applied to a closed stroke or shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// Flat solid color.
    Solid(SolidFill),
    /// Axial (linear) gradient.
    Linear(LinearGradient),
    /// Radial gradient.
    Radial(RadialGradient),
}

/// Optional stroked border drawn over the fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderStroke {
    /// DeviceRGB color, each channel in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Line width in points.
    pub width: f32,
    /// Stroke opacity, `0.0..=1.0`.
    pub opacity: f32,
}

/// A generated ISO 32000-1 PDF appearance stream (Form XObject) for a filled stroke or shape.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceStream {
    /// Bounding box `[x0, y0, x1, y1]` in PDF user space (y-up).
    pub bbox: [f32; 4],
    /// PDF content stream operators.
    pub content: String,
    /// Optional `/Sh0` shading dictionary (present for linear and radial gradients).
    pub shading_dict: Option<String>,
    /// Optional `/GS0` graphics state dictionary for fill opacity `< 1.0`.
    pub fill_gstate_dict: Option<String>,
    /// Optional `/GS1` graphics state dictionary for stroke opacity `< 1.0`.
    pub stroke_gstate_dict: Option<String>,
}

impl AppearanceStream {
    /// The complete `/Resources << ... >>` dictionary required by this stream.
    #[must_use]
    pub fn resources_dict(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(sh) = &self.shading_dict {
            parts.push(format!("/Shading << /Sh0 {sh} >>"));
        }
        let mut gs_entries: Vec<String> = Vec::new();
        if let Some(gs0) = &self.fill_gstate_dict {
            gs_entries.push(format!("/GS0 {gs0}"));
        }
        if let Some(gs1) = &self.stroke_gstate_dict {
            gs_entries.push(format!("/GS1 {gs1}"));
        }
        if !gs_entries.is_empty() {
            parts.push(format!("/ExtGState << {} >>", gs_entries.join(" ")));
        }
        if parts.is_empty() {
            "<< >>".to_owned()
        } else {
            format!("<< {} >>", parts.join(" "))
        }
    }

    /// Formats the complete PDF Form XObject dictionary and stream bytes.
    #[must_use]
    pub fn to_form_xobject(&self) -> String {
        let [x0, y0, x1, y1] = self.bbox;
        let resources = self.resources_dict();
        let len = self.content.len();
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{} {} {} {}] /Resources {resources} /Length {len} >>\nstream\n{}endstream",
            fmt_num(x0),
            fmt_num(y0),
            fmt_num(x1),
            fmt_num(y1),
            self.content
        )
    }
}

/// Returns whether a freehand stroke's ends are close enough to treat it as a closed shape suitable for filling.
#[must_use]
pub fn is_closed_stroke(points: &[Point]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let length: f32 = points
        .windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum();
    if length <= f32::EPSILON {
        return false;
    }
    let end_gap =
        (points[0].x - points[points.len() - 1].x).hypot(points[0].y - points[points.len() - 1].y);
    end_gap <= length * CLOSED_RATIO
}

/// Evaluates `fill` at `point` (in page points), returning its DeviceRGB color if the point lies inside the gradient
/// domain (or always for a solid fill or `ExtendMode::Both`).
#[must_use]
pub fn sample_fill(fill: &Fill, point: Point) -> Option<[f32; 3]> {
    match fill {
        Fill::Solid(solid) => Some(clamp_rgb(solid.color)),
        Fill::Linear(grad) => {
            let dx = grad.end.x - grad.start.x;
            let dy = grad.end.y - grad.start.y;
            let len_sq = dx * dx + dy * dy;
            if len_sq <= f32::EPSILON {
                return None;
            }
            let raw_t = ((point.x - grad.start.x) * dx + (point.y - grad.start.y) * dy) / len_sq;
            let t = apply_extend(raw_t, grad.extend)?;
            Some(eval_stops(&grad.stops, t))
        }
        Fill::Radial(grad) => {
            let t = radial_parameter(grad, point)?;
            let t = apply_extend(t, grad.extend)?;
            Some(eval_stops(&grad.stops, t))
        }
    }
}

/// Builds an ISO 32000-1 PDF appearance stream for `path` (page points, y-down) filled with `fill`, optionally
/// stroked with `border`.
///
/// # Errors
///
/// Returns a [`FillError`] if the path has fewer than 3 points, `page_height` is non-positive, or the gradient stops
/// or geometry are invalid.
pub fn build_appearance(
    path: &[Point],
    fill: &Fill,
    rule: FillRule,
    border: Option<BorderStroke>,
    page_height: f32,
) -> Result<AppearanceStream, FillError> {
    if path.len() < 3 {
        return Err(FillError::TooFewPoints);
    }
    if !page_height.is_finite() || page_height <= 0.0 {
        return Err(FillError::InvalidPageHeight);
    }
    validate_fill(fill)?;

    let to_pdf = |p: Point| (p.x, page_height - p.y);
    let pad = border.map_or(0.0, |b| b.width.max(0.0) / 2.0);
    let (mut x0, mut y0, mut x1, mut y1) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for &pt in path {
        let (px, py) = to_pdf(pt);
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    let bbox = [x0 - pad, y0 - pad, x1 + pad, y1 + pad];

    let fill_opacity = match fill {
        Fill::Solid(s) => clamp_unit(s.opacity),
        Fill::Linear(g) => clamp_unit(g.opacity),
        Fill::Radial(g) => clamp_unit(g.opacity),
    };
    let fill_gstate_dict = (fill_opacity < 0.999)
        .then(|| format!("<< /Type /ExtGState /ca {} >>", fmt_num(fill_opacity)));

    let stroke_gstate_dict = border.and_then(|b| {
        let op = clamp_unit(b.opacity);
        (op < 0.999).then(|| format!("<< /Type /ExtGState /CA {} >>", fmt_num(op)))
    });

    let mut content = String::new();
    content.push_str("q\n");
    if fill_gstate_dict.is_some() {
        content.push_str("/GS0 gs\n");
    }

    let shading_dict = match fill {
        Fill::Solid(solid) => {
            let [r, g, b] = clamp_rgb(solid.color);
            let _ = writeln!(content, "{} {} {} rg", fmt_num(r), fmt_num(g), fmt_num(b));
            write_closed_path(&mut content, path, to_pdf);
            match rule {
                FillRule::NonZero => content.push_str("f\n"),
                FillRule::EvenOdd => content.push_str("f*\n"),
            }
            None
        }
        Fill::Linear(grad) => {
            let (sx, sy) = to_pdf(grad.start);
            let (ex, ey) = to_pdf(grad.end);
            let func = build_pdf_function(&grad.stops);
            let sh = format!(
                "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{} {} {} {}] /Extend {} /Function {func} >>",
                fmt_num(sx),
                fmt_num(sy),
                fmt_num(ex),
                fmt_num(ey),
                grad.extend.pdf_array()
            );
            write_closed_path(&mut content, path, to_pdf);
            match rule {
                FillRule::NonZero => content.push_str("W n\n"),
                FillRule::EvenOdd => content.push_str("W* n\n"),
            }
            content.push_str("/Sh0 sh\n");
            Some(sh)
        }
        Fill::Radial(grad) => {
            let (sx, sy) = to_pdf(grad.start_center);
            let (ex, ey) = to_pdf(grad.end_center);
            let func = build_pdf_function(&grad.stops);
            let sh = format!(
                "<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [{} {} {} {} {} {}] /Extend {} /Function {func} >>",
                fmt_num(sx),
                fmt_num(sy),
                fmt_num(grad.start_radius),
                fmt_num(ex),
                fmt_num(ey),
                fmt_num(grad.end_radius),
                grad.extend.pdf_array()
            );
            write_closed_path(&mut content, path, to_pdf);
            match rule {
                FillRule::NonZero => content.push_str("W n\n"),
                FillRule::EvenOdd => content.push_str("W* n\n"),
            }
            content.push_str("/Sh0 sh\n");
            Some(sh)
        }
    };
    content.push_str("Q\n");

    if let Some(stroke) = border
        && stroke.width > 0.0
    {
        content.push_str("q\n");
        if stroke_gstate_dict.is_some() {
            content.push_str("/GS1 gs\n");
        }
        let [r, g, b] = clamp_rgb(stroke.color);
        let _ = writeln!(content, "{} w 1 J 1 j", fmt_num(stroke.width));
        let _ = writeln!(content, "{} {} {} RG", fmt_num(r), fmt_num(g), fmt_num(b));
        write_closed_path(&mut content, path, to_pdf);
        content.push_str("S\nQ\n");
    }

    Ok(AppearanceStream {
        bbox,
        content,
        shading_dict,
        fill_gstate_dict,
        stroke_gstate_dict,
    })
}

fn validate_fill(fill: &Fill) -> Result<(), FillError> {
    match fill {
        Fill::Solid(_) => Ok(()),
        Fill::Linear(g) => {
            if (g.end.x - g.start.x).hypot(g.end.y - g.start.y) <= f32::EPSILON {
                return Err(FillError::DegenerateAxis);
            }
            validate_stops(&g.stops)
        }
        Fill::Radial(g) => {
            if !g.start_radius.is_finite()
                || !g.end_radius.is_finite()
                || g.start_radius < 0.0
                || g.end_radius < 0.0
            {
                return Err(FillError::InvalidRadialGeometry);
            }
            let center_dist =
                (g.end_center.x - g.start_center.x).hypot(g.end_center.y - g.start_center.y);
            if center_dist <= f32::EPSILON && (g.end_radius - g.start_radius).abs() <= f32::EPSILON
            {
                return Err(FillError::InvalidRadialGeometry);
            }
            validate_stops(&g.stops)
        }
    }
}

fn validate_stops(stops: &[ColorStop]) -> Result<(), FillError> {
    if stops.len() < 2 {
        return Err(FillError::TooFewStops);
    }
    let mut prev = -1.0_f32;
    for stop in stops {
        if !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset) || stop.offset < prev {
            return Err(FillError::InvalidStops);
        }
        prev = stop.offset;
    }
    Ok(())
}

/// Builds a PDF Type 2 (exponential interpolation) function for 2 stops, or a Type 3 (stitching) function combining
/// Type 2 sub-functions for 3+ stops (ISO 32000-1 §7.10.3 & §7.10.4).
fn build_pdf_function(stops: &[ColorStop]) -> String {
    let mut normalized = Vec::with_capacity(stops.len() + 2);
    if stops[0].offset > 0.0 {
        normalized.push(ColorStop::new(0.0, stops[0].color));
    }
    normalized.extend(stops.iter().map(|s| ColorStop::new(s.offset, s.color)));
    if let Some(last) = stops.last()
        && last.offset < 1.0
    {
        normalized.push(ColorStop::new(1.0, last.color));
    }

    if normalized.len() == 2 {
        return type2_function(normalized[0].color, normalized[1].color);
    }

    let mut functions = Vec::with_capacity(normalized.len() - 1);
    let mut bounds = Vec::with_capacity(normalized.len().saturating_sub(2));
    let mut encode = Vec::with_capacity((normalized.len() - 1) * 2);
    for (idx, pair) in normalized.windows(2).enumerate() {
        functions.push(type2_function(pair[0].color, pair[1].color));
        if idx > 0 {
            bounds.push(fmt_num(pair[0].offset));
        }
        encode.push("0 1");
    }
    format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
        functions.join(" "),
        bounds.join(" "),
        encode.join(" ")
    )
}

fn type2_function(c0: [f32; 3], c1: [f32; 3]) -> String {
    format!(
        "<< /FunctionType 2 /Domain [0 1] /C0 [{} {} {}] /C1 [{} {} {}] /N 1 >>",
        fmt_num(c0[0]),
        fmt_num(c0[1]),
        fmt_num(c0[2]),
        fmt_num(c1[0]),
        fmt_num(c1[1]),
        fmt_num(c1[2]),
    )
}

fn write_closed_path(out: &mut String, path: &[Point], to_pdf: impl Fn(Point) -> (f32, f32)) {
    let (x0, y0) = to_pdf(path[0]);
    let _ = writeln!(out, "{} {} m", fmt_num(x0), fmt_num(y0));
    for &pt in &path[1..] {
        let (x, y) = to_pdf(pt);
        let _ = writeln!(out, "{} {} l", fmt_num(x), fmt_num(y));
    }
    out.push_str("h\n");
}

fn apply_extend(t: f32, extend: ExtendMode) -> Option<f32> {
    if t < 0.0 {
        match extend {
            ExtendMode::StartOnly | ExtendMode::Both => Some(0.0),
            ExtendMode::None | ExtendMode::EndOnly => None,
        }
    } else if t > 1.0 {
        match extend {
            ExtendMode::EndOnly | ExtendMode::Both => Some(1.0),
            ExtendMode::None | ExtendMode::StartOnly => None,
        }
    } else {
        Some(t)
    }
}

fn eval_stops(stops: &[ColorStop], t: f32) -> [f32; 3] {
    if stops.is_empty() {
        return [0.0; 3];
    }
    if t <= stops[0].offset {
        return clamp_rgb(stops[0].color);
    }
    for pair in stops.windows(2) {
        if t <= pair[1].offset {
            let span = pair[1].offset - pair[0].offset;
            let frac = if span <= f32::EPSILON {
                1.0
            } else {
                ((t - pair[0].offset) / span).clamp(0.0, 1.0)
            };
            let a = clamp_rgb(pair[0].color);
            let b = clamp_rgb(pair[1].color);
            return [
                a[0] + (b[0] - a[0]) * frac,
                a[1] + (b[1] - a[1]) * frac,
                a[2] + (b[2] - a[2]) * frac,
            ];
        }
    }
    clamp_rgb(stops[stops.len() - 1].color)
}

/// Solves PDF's Type 3 radial shading quadratic for `s` in `[0, 1]` (ISO 32000-1 §8.7.4.5.4).
fn radial_parameter(grad: &RadialGradient, point: Point) -> Option<f32> {
    let dx = f64::from(grad.end_center.x - grad.start_center.x);
    let dy = f64::from(grad.end_center.y - grad.start_center.y);
    let dr = f64::from(grad.end_radius - grad.start_radius);
    let px = f64::from(point.x - grad.start_center.x);
    let py = f64::from(point.y - grad.start_center.y);
    let r0 = f64::from(grad.start_radius);

    // Concentric fast-path.
    if dx.hypot(dy) <= 1e-6 {
        if dr.abs() <= 1e-6 {
            return None;
        }
        return Some(((px.hypot(py) - r0) / dr) as f32);
    }

    let a = dx * dx + dy * dy - dr * dr;
    let b = -2.0 * (px * dx + py * dy + r0 * dr);
    let c = px * px + py * py - r0 * r0;
    if a.abs() <= 1e-9 {
        if b.abs() <= 1e-9 {
            return None;
        }
        let s = -c / b;
        return (r0 + s * dr >= 0.0).then_some(s as f32);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let sqrt_d = disc.sqrt();
    let s1 = (-b + sqrt_d) / (2.0 * a);
    let s2 = (-b - sqrt_d) / (2.0 * a);
    // PDF spec picks the larger root whose circle radius r(s) >= 0.
    let (hi, lo) = if s1 >= s2 { (s1, s2) } else { (s2, s1) };
    if r0 + hi * dr >= 0.0 {
        Some(hi as f32)
    } else if r0 + lo * dr >= 0.0 {
        Some(lo as f32)
    } else {
        None
    }
}

fn clamp_unit(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn clamp_rgb(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(clamp_unit)
}

fn fmt_num(v: f32) -> String {
    let v = if v.abs() < 1e-6 { 0.0 } else { v };
    // 6 decimal places ensure every 8-bit RGB value (n / 255.0) survives PDF renderers that truncate `v * 255`.
    let text = format!("{v:.6}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_path() -> Vec<Point> {
        vec![
            Point { x: 10.0, y: 20.0 },
            Point { x: 110.0, y: 20.0 },
            Point { x: 110.0, y: 120.0 },
            Point { x: 10.0, y: 120.0 },
        ]
    }

    #[test]
    fn solid_fill_emits_standard_pdf_operators_and_extgstate() {
        let stream = build_appearance(
            &square_path(),
            &Fill::Solid(SolidFill::new([0.2, 0.5, 0.8], 0.6)),
            FillRule::NonZero,
            Some(BorderStroke {
                color: [0.0, 0.0, 0.0],
                width: 2.0,
                opacity: 1.0,
            }),
            800.0,
        )
        .expect("builds solid appearance");

        assert_eq!(stream.bbox, [9.0, 679.0, 111.0, 781.0]);
        assert!(stream.shading_dict.is_none());
        assert_eq!(
            stream.fill_gstate_dict.as_deref(),
            Some("<< /Type /ExtGState /ca 0.6 >>")
        );
        assert!(stream.content.contains("/GS0 gs\n0.2 0.5 0.8 rg\n"));
        assert!(stream.content.contains("10 780 m\n"));
        assert!(stream.content.contains("h\nf\nQ\n"));
        assert!(stream.content.contains("2 w 1 J 1 j\n0 0 0 RG\n"));
        let xobj = stream.to_form_xobject();
        assert!(xobj.contains("/Subtype /Form"));
        assert!(xobj.contains("/ExtGState << /GS0"));
    }

    #[test]
    fn linear_gradient_emits_type2_shading_and_stitching_for_multi_stop() {
        let grad = Fill::Linear(LinearGradient {
            start: Point { x: 10.0, y: 20.0 },
            end: Point { x: 110.0, y: 20.0 },
            stops: vec![
                ColorStop::new(0.0, [1.0, 0.0, 0.0]),
                ColorStop::new(0.5, [0.0, 1.0, 0.0]),
                ColorStop::new(1.0, [0.0, 0.0, 1.0]),
            ],
            extend: ExtendMode::Both,
            opacity: 1.0,
        });
        let stream = build_appearance(&square_path(), &grad, FillRule::EvenOdd, None, 800.0)
            .expect("builds linear gradient");
        let sh = stream.shading_dict.as_deref().expect("shading dict");
        assert!(sh.contains("/ShadingType 2"), "{sh}");
        assert!(sh.contains("/Coords [10 780 110 780]"), "{sh}");
        assert!(sh.contains("/FunctionType 3"), "3 stops stitch: {sh}");
        assert!(sh.contains("/Bounds [0.5]"), "{sh}");
        assert!(stream.content.contains("W* n\n/Sh0 sh\n"));

        // CPU sampler matches endpoints and midpoint.
        let mid = sample_fill(&grad, Point { x: 60.0, y: 70.0 }).expect("mid");
        assert!((mid[1] - 1.0).abs() < 1e-4 && mid[0].abs() < 1e-4);
    }

    #[test]
    fn radial_gradient_emits_type3_shading_and_samples_concentrically() {
        let grad = Fill::Radial(RadialGradient {
            start_center: Point { x: 60.0, y: 70.0 },
            start_radius: 0.0,
            end_center: Point { x: 60.0, y: 70.0 },
            end_radius: 50.0,
            stops: vec![
                ColorStop::new(0.0, [1.0, 1.0, 1.0]),
                ColorStop::new(1.0, [0.0, 0.0, 0.0]),
            ],
            extend: ExtendMode::Both,
            opacity: 0.9,
        });
        let stream = build_appearance(&square_path(), &grad, FillRule::NonZero, None, 800.0)
            .expect("builds radial gradient");
        let sh = stream.shading_dict.as_deref().expect("shading dict");
        assert!(sh.contains("/ShadingType 3"), "{sh}");
        assert!(sh.contains("/Coords [60 730 0 60 730 50]"), "{sh}");
        assert!(sh.contains("/FunctionType 2"), "2 stops use Type 2: {sh}");
        assert!(stream.content.contains("W n\n/Sh0 sh\n"));

        let center = sample_fill(&grad, Point { x: 60.0, y: 70.0 }).expect("center");
        assert!((center[0] - 1.0).abs() < 1e-4);
        let halfway = sample_fill(&grad, Point { x: 85.0, y: 70.0 }).expect("halfway");
        assert!((halfway[0] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn validation_rejects_broken_inputs_and_detects_closed_strokes() {
        let solid = Fill::Solid(SolidFill::new([1.0, 0.0, 0.0], 1.0));
        assert_eq!(
            build_appearance(
                &[Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 }],
                &solid,
                FillRule::NonZero,
                None,
                800.0
            ),
            Err(FillError::TooFewPoints)
        );
        assert_eq!(
            build_appearance(&square_path(), &solid, FillRule::NonZero, None, 0.0),
            Err(FillError::InvalidPageHeight)
        );
        let bad_axis = Fill::Linear(LinearGradient {
            start: Point { x: 5.0, y: 5.0 },
            end: Point { x: 5.0, y: 5.0 },
            stops: vec![ColorStop::new(0.0, [0.0; 3]), ColorStop::new(1.0, [1.0; 3])],
            extend: ExtendMode::Both,
            opacity: 1.0,
        });
        assert_eq!(
            build_appearance(&square_path(), &bad_axis, FillRule::NonZero, None, 800.0),
            Err(FillError::DegenerateAxis)
        );
        let bad_stops = Fill::Linear(LinearGradient {
            start: Point { x: 0.0, y: 0.0 },
            end: Point { x: 10.0, y: 0.0 },
            stops: vec![ColorStop::new(0.8, [0.0; 3]), ColorStop::new(0.2, [1.0; 3])],
            extend: ExtendMode::Both,
            opacity: 1.0,
        });
        assert_eq!(
            build_appearance(&square_path(), &bad_stops, FillRule::NonZero, None, 800.0),
            Err(FillError::InvalidStops)
        );

        let closed = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
            Point { x: 100.0, y: 100.0 },
            Point { x: 0.0, y: 100.0 },
            Point { x: 2.0, y: 2.0 },
        ];
        assert!(is_closed_stroke(&closed));
        let open = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 50.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
        ];
        assert!(!is_closed_stroke(&open));
    }
}
