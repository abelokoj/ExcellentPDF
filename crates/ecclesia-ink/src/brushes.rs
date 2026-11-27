//! Pattern brushes (plan H.20): brushes that repeat vector geometry along an ink stroke and
//! emit standard ISO 32000-1 PDF appearance operators so pattern strokes render identically in
//! every PDF reader.

use std::fmt::{self, Write as _};

use crate::erase::Point;
use crate::stroke::InkPoint;

/// Built-in and custom vector pattern styles (plan H.20).
#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// Evenly spaced filled dots along the path.
    Dots,
    /// Short dashes aligned to the stroke tangent.
    Dashes,
    /// Diagonal cross-hatched ticks along the path.
    Crosshatch,
    /// Alternating chevrons forming a zigzag pattern.
    Zigzag,
    /// Interlocking oval links.
    Chain,
    /// Five-pointed vector stars.
    Stars,
    /// User-defined vector tile primitives.
    Custom(PatternTile),
}

/// A single geometric primitive inside a [`PatternTile`], in unit coordinates `[-0.5, 0.5]`.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternPrimitive {
    /// Filled circle centered at `(cx, cy)` with `radius`.
    Circle {
        /// Center X in normalized tile units.
        cx: f32,
        /// Center Y in normalized tile units.
        cy: f32,
        /// Radius in normalized tile units.
        radius: f32,
    },
    /// Stroked line segment from `a` to `b` with normalized stroke weight.
    Line {
        /// Start point.
        a: Point,
        /// End point.
        b: Point,
        /// Relative stroke weight multiplier.
        weight: f32,
    },
    /// Closed filled polygon through `vertices`.
    Polygon(Vec<Point>),
}

/// Reusable vector tile repeated along a stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternTile {
    /// Vector primitives forming one stamp.
    pub primitives: Vec<PatternPrimitive>,
}

/// How each repeated stamp is oriented along the stroke.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum StampOrientation {
    /// Rotate each stamp to follow the local stroke tangent.
    #[default]
    FollowTangent,
    /// Keep every stamp at a fixed angle (radians in page y-down space).
    FixedAngle(f32),
}

/// Configuration for a vector pattern brush (plan H.20).
#[derive(Debug, Clone, PartialEq)]
pub struct PatternBrush {
    /// Pattern geometry kind.
    pub kind: PatternKind,
    /// Center-to-center arc-length spacing between stamps, in page points.
    pub spacing_pt: f32,
    /// Scale multiplier applied on top of stroke point width.
    pub scale: f32,
    /// DeviceRGB color, each channel in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Opacity in `0.0..=1.0`.
    pub opacity: f32,
    /// Stamp orientation mode.
    pub orientation: StampOrientation,
}

impl Default for PatternBrush {
    fn default() -> Self {
        Self::new(PatternKind::Dots)
    }
}

impl PatternBrush {
    /// Creates a pattern brush with sensible defaults for `kind`.
    #[must_use]
    pub fn new(kind: PatternKind) -> Self {
        Self {
            kind,
            spacing_pt: 8.0,
            scale: 1.0,
            color: [0.10, 0.24, 0.56],
            opacity: 1.0,
            orientation: StampOrientation::FollowTangent,
        }
    }

    /// Returns the unit-space [`PatternTile`] for this brush's [`PatternKind`].
    #[must_use]
    pub fn tile(&self) -> PatternTile {
        match &self.kind {
            PatternKind::Dots => PatternTile {
                primitives: vec![PatternPrimitive::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    radius: 0.45,
                }],
            },
            PatternKind::Dashes => PatternTile {
                primitives: vec![PatternPrimitive::Line {
                    a: Point { x: -0.45, y: 0.0 },
                    b: Point { x: 0.45, y: 0.0 },
                    weight: 0.35,
                }],
            },
            PatternKind::Crosshatch => PatternTile {
                primitives: vec![
                    PatternPrimitive::Line {
                        a: Point { x: -0.4, y: -0.4 },
                        b: Point { x: 0.4, y: 0.4 },
                        weight: 0.2,
                    },
                    PatternPrimitive::Line {
                        a: Point { x: -0.4, y: 0.4 },
                        b: Point { x: 0.4, y: -0.4 },
                        weight: 0.2,
                    },
                ],
            },
            PatternKind::Zigzag => PatternTile {
                primitives: vec![
                    PatternPrimitive::Line {
                        a: Point { x: -0.45, y: 0.25 },
                        b: Point { x: 0.0, y: -0.25 },
                        weight: 0.22,
                    },
                    PatternPrimitive::Line {
                        a: Point { x: 0.0, y: -0.25 },
                        b: Point { x: 0.45, y: 0.25 },
                        weight: 0.22,
                    },
                ],
            },
            PatternKind::Chain => PatternTile {
                primitives: vec![
                    PatternPrimitive::Line {
                        a: Point { x: -0.4, y: -0.2 },
                        b: Point { x: 0.4, y: -0.2 },
                        weight: 0.16,
                    },
                    PatternPrimitive::Line {
                        a: Point { x: -0.4, y: 0.2 },
                        b: Point { x: 0.4, y: 0.2 },
                        weight: 0.16,
                    },
                    PatternPrimitive::Circle {
                        cx: -0.35,
                        cy: 0.0,
                        radius: 0.12,
                    },
                    PatternPrimitive::Circle {
                        cx: 0.35,
                        cy: 0.0,
                        radius: 0.12,
                    },
                ],
            },
            PatternKind::Stars => {
                let mut verts = Vec::with_capacity(10);
                for i in 0..10 {
                    let angle =
                        -std::f32::consts::FRAC_PI_2 + (i as f32) * std::f32::consts::PI / 5.0;
                    let r = if i % 2 == 0 { 0.48 } else { 0.20 };
                    verts.push(Point {
                        x: angle.cos() * r,
                        y: angle.sin() * r,
                    });
                }
                PatternTile {
                    primitives: vec![PatternPrimitive::Polygon(verts)],
                }
            }
            PatternKind::Custom(tile) => tile.clone(),
        }
    }
}

/// Concrete placement of a single pattern stamp along a stroke in page points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedStamp {
    /// Center position in page points (`x` right, `y` down).
    pub position: Point,
    /// Size in page points (derived from interpolated stroke width × brush scale).
    pub size: f32,
    /// Rotation angle in radians in page space.
    pub angle_radians: f32,
}

/// Errors building a PDF appearance stream for a pattern brush.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrushError {
    /// Stroke had zero points.
    EmptyStroke,
    /// Brush spacing or scale was non-positive or non-finite.
    InvalidBrushGeometry,
    /// Page height was non-positive or non-finite.
    InvalidPageHeight,
}

impl fmt::Display for BrushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyStroke => f.write_str("pattern brush stroke cannot be empty"),
            Self::InvalidBrushGeometry => {
                f.write_str("pattern brush spacing and scale must be positive and finite")
            }
            Self::InvalidPageHeight => f.write_str("page height must be positive and finite"),
        }
    }
}

impl std::error::Error for BrushError {}

/// Samples arc-length-spaced [`PlacedStamp`] instances along `points` for `brush`.
#[must_use]
pub fn place_stamps(points: &[InkPoint], brush: &PatternBrush) -> Vec<PlacedStamp> {
    if points.is_empty()
        || !brush.spacing_pt.is_finite()
        || brush.spacing_pt <= 0.0
        || !brush.scale.is_finite()
        || brush.scale <= 0.0
    {
        return Vec::new();
    }
    let scale = brush.scale;
    let spacing = brush.spacing_pt.max(0.5);
    let first = points[0];
    let angle_for = |dx: f32, dy: f32| match brush.orientation {
        StampOrientation::FollowTangent => dy.atan2(dx),
        StampOrientation::FixedAngle(a) => a,
    };

    if points.len() == 1 {
        return vec![PlacedStamp {
            position: Point {
                x: first.x,
                y: first.y,
            },
            size: (first.width * scale).max(0.5),
            angle_radians: angle_for(1.0, 0.0),
        }];
    }

    let initial_angle = angle_for(points[1].x - first.x, points[1].y - first.y);
    let mut stamps = vec![PlacedStamp {
        position: Point {
            x: first.x,
            y: first.y,
        },
        size: (first.width * scale).max(0.5),
        angle_radians: initial_angle,
    }];

    let mut next_sample_dist = spacing;
    let mut accum_dist = 0.0_f32;

    for seg in points.windows(2) {
        let dx = seg[1].x - seg[0].x;
        let dy = seg[1].y - seg[0].y;
        let seg_len = dx.hypot(dy);
        if seg_len <= 1e-5 {
            continue;
        }
        let seg_angle = angle_for(dx, dy);
        while accum_dist + seg_len >= next_sample_dist {
            let t = (next_sample_dist - accum_dist) / seg_len;
            let x = seg[0].x + dx * t;
            let y = seg[0].y + dy * t;
            let w = seg[0].width + (seg[1].width - seg[0].width) * t;
            stamps.push(PlacedStamp {
                position: Point { x, y },
                size: (w * scale).max(0.5),
                angle_radians: seg_angle,
            });
            next_sample_dist += spacing;
        }
        accum_dist += seg_len;
    }

    stamps
}

/// Emits standard ISO 32000-1 PDF content stream operators repeating `brush`'s vector tile
/// along `points`, converting top-left page points to PDF bottom-left coordinates using
/// `page_height`.
///
/// # Errors
///
/// Returns [`BrushError`] when `points` is empty or brush/page dimensions are invalid.
pub fn to_pdf_appearance_stream(
    points: &[InkPoint],
    brush: &PatternBrush,
    page_height: f32,
) -> Result<String, BrushError> {
    if points.is_empty() {
        return Err(BrushError::EmptyStroke);
    }
    if !brush.spacing_pt.is_finite()
        || brush.spacing_pt <= 0.0
        || !brush.scale.is_finite()
        || brush.scale <= 0.0
    {
        return Err(BrushError::InvalidBrushGeometry);
    }
    if !page_height.is_finite() || page_height <= 0.0 {
        return Err(BrushError::InvalidPageHeight);
    }

    let stamps = place_stamps(points, brush);
    let tile = brush.tile();
    let [r, g, b] = [
        brush.color[0].clamp(0.0, 1.0),
        brush.color[1].clamp(0.0, 1.0),
        brush.color[2].clamp(0.0, 1.0),
    ];

    let mut out = String::new();
    let _ = writeln!(out, "q");
    let _ = writeln!(out, "{r:.4} {g:.4} {b:.4} rg");
    let _ = writeln!(out, "{r:.4} {g:.4} {b:.4} RG");
    let _ = writeln!(out, "1 J 1 j");

    for stamp in &stamps {
        let (sin, cos) = stamp.angle_radians.sin_cos();
        let xform = |lx: f32, ly: f32| -> (f32, f32) {
            let sx = lx * stamp.size;
            let sy = ly * stamp.size;
            let px = stamp.position.x + sx * cos - sy * sin;
            let py = stamp.position.y + sx * sin + sy * cos;
            (px, page_height - py)
        };

        for prim in &tile.primitives {
            match prim {
                PatternPrimitive::Circle { cx, cy, radius } => {
                    let r_pt = (*radius * stamp.size).max(0.25);
                    let (ux, uy) = xform(*cx, *cy);
                    // 4-segment cubic Bezier circle approximation (k = 0.55228475).
                    let k = 0.552_284_8 * r_pt;
                    let _ = writeln!(
                        out,
                        "{:.3} {:.3} m {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c f",
                        ux + r_pt,
                        uy,
                        ux + r_pt,
                        uy + k,
                        ux + k,
                        uy + r_pt,
                        ux,
                        uy + r_pt,
                        ux - k,
                        uy + r_pt,
                        ux - r_pt,
                        uy + k,
                        ux - r_pt,
                        uy,
                        ux - r_pt,
                        uy - k,
                        ux - k,
                        uy - r_pt,
                        ux,
                        uy - r_pt,
                        ux + k,
                        uy - r_pt,
                        ux + r_pt,
                        uy - k,
                        ux + r_pt,
                        uy
                    );
                }
                PatternPrimitive::Line { a, b, weight } => {
                    let (ax, ay) = xform(a.x, a.y);
                    let (bx, by) = xform(b.x, b.y);
                    let w = (stamp.size * *weight).max(0.2);
                    let _ = writeln!(out, "{w:.3} w {ax:.3} {ay:.3} m {bx:.3} {by:.3} l S");
                }
                PatternPrimitive::Polygon(verts) if verts.len() >= 3 => {
                    let (x0, y0) = xform(verts[0].x, verts[0].y);
                    let _ = write!(out, "{x0:.3} {y0:.3} m ");
                    for v in &verts[1..] {
                        let (vx, vy) = xform(v.x, v.y);
                        let _ = write!(out, "{vx:.3} {vy:.3} l ");
                    }
                    let _ = writeln!(out, "h f");
                }
                PatternPrimitive::Polygon(_) => {}
            }
        }
    }

    let _ = writeln!(out, "Q");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32, width: f32) -> InkPoint {
        InkPoint { x, y, width }
    }

    #[test]
    fn stamps_follow_arc_length_and_tangent() {
        let brush = PatternBrush {
            kind: PatternKind::Dashes,
            spacing_pt: 10.0,
            scale: 1.0,
            color: [0.0, 0.0, 0.0],
            opacity: 1.0,
            orientation: StampOrientation::FollowTangent,
        };
        let stroke = vec![pt(0.0, 0.0, 4.0), pt(30.0, 0.0, 4.0), pt(30.0, 20.0, 4.0)];
        let stamps = place_stamps(&stroke, &brush);
        // Distances 0, 10, 20, 30 (horizontal), then 40, 50 (vertical) => 6 stamps.
        assert_eq!(stamps.len(), 6);
        assert!((stamps[1].position.x - 10.0).abs() < 0.01);
        assert!(stamps[1].angle_radians.abs() < 0.01);
        assert!((stamps[4].position.y - 10.0).abs() < 0.01);
        assert!((stamps[4].angle_radians - std::f32::consts::FRAC_PI_2).abs() < 0.01);
    }

    #[test]
    fn emits_valid_pdf_operators_for_all_builtin_patterns() {
        let stroke = vec![pt(10.0, 10.0, 5.0), pt(50.0, 20.0, 6.0)];
        for kind in [
            PatternKind::Dots,
            PatternKind::Dashes,
            PatternKind::Crosshatch,
            PatternKind::Zigzag,
            PatternKind::Chain,
            PatternKind::Stars,
        ] {
            let brush = PatternBrush::new(kind);
            let pdf_ops = to_pdf_appearance_stream(&stroke, &brush, 792.0).expect("stream");
            assert!(pdf_ops.starts_with("q\n"));
            assert!(pdf_ops.ends_with("Q\n"));
            assert!(pdf_ops.contains(" m "));
        }
    }
}
