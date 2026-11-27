//! Stroke symmetry axes and radial mirrors (plan H.21): mirrors live or completed ink strokes
//! across vertical, horizontal, cross, angled, or N-fold radial axes, producing separate
//! strokes for each reflected/rotated copy.

use std::f32::consts::TAU;

use crate::erase::Point;
use crate::sample::Sample;
use crate::stroke::InkPoint;

/// Whether radial symmetry only rotates spokes or also mirrors inside each spoke (kaleidoscope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RadialMode {
    /// Pure N-fold rotational symmetry (`N - 1` copies).
    RotateOnly,
    /// Rotational plus reflection across each sector bisector (`2N - 1` copies).
    #[default]
    Kaleidoscope,
}

/// Symmetry guide configuration (plan H.21).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymmetryAxis {
    /// Mirror across the vertical line `x = x_pt`.
    Vertical {
        /// Horizontal coordinate of the vertical axis in page points.
        x: f32,
    },
    /// Mirror across the horizontal line `y = y_pt`.
    Horizontal {
        /// Vertical coordinate of the horizontal axis in page points.
        y: f32,
    },
    /// Four-quadrant symmetry across both `x` and `y` axes.
    Cross {
        /// Vertical axis coordinate.
        x: f32,
        /// Horizontal axis coordinate.
        y: f32,
    },
    /// Mirror across an arbitrary line passing through `origin` at `angle_radians`.
    Line {
        /// Anchor point on the mirror line.
        origin: Point,
        /// Line angle in radians in page y-down space.
        angle_radians: f32,
    },
    /// N-fold radial symmetry around `center`.
    Radial {
        /// Center point in page points.
        center: Point,
        /// Number of rotational sectors (clamped to `2..=32`).
        spokes: u8,
        /// Whether each sector is also mirrored.
        mode: RadialMode,
    },
}

/// A guide segment to render as a non-printing overlay on the page canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuideSegment {
    /// Start point in page points.
    pub a: Point,
    /// End point in page points.
    pub b: Point,
}

/// Output of mirroring a primary stroke: the original stroke plus each separate mirrored copy.
#[derive(Debug, Clone, PartialEq)]
pub struct MirroredInk {
    /// The primary stroke as drawn.
    pub primary: Vec<InkPoint>,
    /// Independent reflected/rotated strokes to save as separate ink annotations (plan H.21).
    pub copies: Vec<Vec<InkPoint>>,
}

impl MirroredInk {
    /// All strokes (`primary` followed by `copies`).
    #[must_use]
    pub fn into_all_strokes(mut self) -> Vec<Vec<InkPoint>> {
        let mut all = Vec::with_capacity(1 + self.copies.len());
        all.push(self.primary);
        all.append(&mut self.copies);
        all
    }
}

/// Reflects `point` across the line through `origin` at `angle_radians`.
#[must_use]
pub fn reflect_point(point: Point, origin: Point, angle_radians: f32) -> Point {
    let (sin, cos) = (2.0 * angle_radians).sin_cos();
    let dx = point.x - origin.x;
    let dy = point.y - origin.y;
    Point {
        x: origin.x + dx * cos + dy * sin,
        y: origin.y + dx * sin - dy * cos,
    }
}

/// Rotates `point` around `center` by `angle_radians`.
#[must_use]
pub fn rotate_point(point: Point, center: Point, angle_radians: f32) -> Point {
    let (sin, cos) = angle_radians.sin_cos();
    let dx = point.x - center.x;
    let dy = point.y - center.y;
    Point {
        x: center.x + dx * cos - dy * sin,
        y: center.y + dx * sin + dy * cos,
    }
}

/// Generates canvas overlay guide segments clipped to `[0, page_width] x [0, page_height]`.
#[must_use]
pub fn guide_segments(axis: SymmetryAxis, page_width: f32, page_height: f32) -> Vec<GuideSegment> {
    let diag = page_width.hypot(page_height).max(1.0);
    match axis {
        SymmetryAxis::Vertical { x } => vec![GuideSegment {
            a: Point { x, y: 0.0 },
            b: Point { x, y: page_height },
        }],
        SymmetryAxis::Horizontal { y } => vec![GuideSegment {
            a: Point { x: 0.0, y },
            b: Point { x: page_width, y },
        }],
        SymmetryAxis::Cross { x, y } => vec![
            GuideSegment {
                a: Point { x, y: 0.0 },
                b: Point { x, y: page_height },
            },
            GuideSegment {
                a: Point { x: 0.0, y },
                b: Point { x: page_width, y },
            },
        ],
        SymmetryAxis::Line {
            origin,
            angle_radians,
        } => {
            let (sin, cos) = angle_radians.sin_cos();
            vec![GuideSegment {
                a: Point {
                    x: origin.x - cos * diag,
                    y: origin.y - sin * diag,
                },
                b: Point {
                    x: origin.x + cos * diag,
                    y: origin.y + sin * diag,
                },
            }]
        }
        SymmetryAxis::Radial { center, spokes, .. } => {
            let count = spokes.clamp(2, 32);
            (0..count)
                .map(|i| {
                    let angle = TAU * f32::from(i) / f32::from(count);
                    let (sin, cos) = angle.sin_cos();
                    GuideSegment {
                        a: center,
                        b: Point {
                            x: center.x + cos * diag,
                            y: center.y + sin * diag,
                        },
                    }
                })
                .collect()
        }
    }
}

/// Mirrors `stroke` across `axis`, returning the primary stroke and every generated copy as
/// independent strokes ready to be saved separately (plan H.21 **Done when**).
#[must_use]
pub fn mirror_stroke(stroke: &[InkPoint], axis: SymmetryAxis) -> MirroredInk {
    let map_all = |f: &dyn Fn(Point) -> Point| -> Vec<InkPoint> {
        stroke
            .iter()
            .map(|p| {
                let mapped = f(Point { x: p.x, y: p.y });
                InkPoint {
                    x: mapped.x,
                    y: mapped.y,
                    width: p.width,
                }
            })
            .collect()
    };

    let copies = match axis {
        SymmetryAxis::Vertical { x } => vec![map_all(&|p| Point {
            x: 2.0 * x - p.x,
            y: p.y,
        })],
        SymmetryAxis::Horizontal { y } => vec![map_all(&|p| Point {
            x: p.x,
            y: 2.0 * y - p.y,
        })],
        SymmetryAxis::Cross { x, y } => vec![
            map_all(&|p| Point {
                x: 2.0 * x - p.x,
                y: p.y,
            }),
            map_all(&|p| Point {
                x: p.x,
                y: 2.0 * y - p.y,
            }),
            map_all(&|p| Point {
                x: 2.0 * x - p.x,
                y: 2.0 * y - p.y,
            }),
        ],
        SymmetryAxis::Line {
            origin,
            angle_radians,
        } => vec![map_all(&|p| reflect_point(p, origin, angle_radians))],
        SymmetryAxis::Radial {
            center,
            spokes,
            mode,
        } => {
            let n = spokes.clamp(2, 32);
            let mut out = Vec::new();
            if mode == RadialMode::Kaleidoscope {
                out.push(map_all(&|p| reflect_point(p, center, 0.0)));
            }
            for k in 1..n {
                let theta = TAU * f32::from(k) / f32::from(n);
                out.push(map_all(&|p| rotate_point(p, center, theta)));
                if mode == RadialMode::Kaleidoscope {
                    out.push(map_all(&|p| reflect_point(p, center, theta * 0.5)));
                }
            }
            out
        }
    };

    MirroredInk {
        primary: stroke.to_vec(),
        copies,
    }
}

/// Mirrors raw pen [`Sample`]s across `axis`, preserving pressure, tilt, and sample timestamps.
#[must_use]
pub fn mirror_samples(samples: &[Sample], axis: SymmetryAxis) -> Vec<Vec<Sample>> {
    let as_ink: Vec<InkPoint> = samples
        .iter()
        .map(|s| InkPoint {
            x: s.x,
            y: s.y,
            width: 1.0,
        })
        .collect();
    mirror_stroke(&as_ink, axis)
        .copies
        .into_iter()
        .map(|copy| {
            copy.into_iter()
                .zip(samples)
                .map(|(pt, src)| Sample {
                    x: pt.x,
                    y: pt.y,
                    ..*src
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> InkPoint {
        InkPoint { x, y, width: 2.0 }
    }

    #[test]
    fn vertical_and_horizontal_reflections_are_exact_involutions() {
        let stroke = vec![pt(120.0, 50.0), pt(160.0, 90.0)];
        let vert = mirror_stroke(&stroke, SymmetryAxis::Vertical { x: 200.0 });
        assert_eq!(vert.copies.len(), 1);
        assert_eq!(vert.copies[0], vec![pt(280.0, 50.0), pt(240.0, 90.0)]);
        let back = mirror_stroke(&vert.copies[0], SymmetryAxis::Vertical { x: 200.0 });
        assert_eq!(back.copies[0], stroke);

        let horiz = mirror_stroke(&stroke, SymmetryAxis::Horizontal { y: 100.0 });
        assert_eq!(horiz.copies[0], vec![pt(120.0, 150.0), pt(160.0, 110.0)]);
    }

    #[test]
    fn cross_and_radial_generate_separate_strokes() {
        let stroke = vec![pt(110.0, 120.0), pt(130.0, 140.0)];
        let cross = mirror_stroke(&stroke, SymmetryAxis::Cross { x: 100.0, y: 100.0 });
        assert_eq!(cross.into_all_strokes().len(), 4);

        let radial_rot = mirror_stroke(
            &stroke,
            SymmetryAxis::Radial {
                center: Point { x: 100.0, y: 100.0 },
                spokes: 6,
                mode: RadialMode::RotateOnly,
            },
        );
        assert_eq!(radial_rot.copies.len(), 5);

        let radial_kal = mirror_stroke(
            &stroke,
            SymmetryAxis::Radial {
                center: Point { x: 100.0, y: 100.0 },
                spokes: 6,
                mode: RadialMode::Kaleidoscope,
            },
        );
        assert_eq!(radial_kal.copies.len(), 11);
    }
}
