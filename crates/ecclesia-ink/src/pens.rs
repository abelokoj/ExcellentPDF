//! Pen types (H.2, plan §8.4): fountain pen, ballpoint, pencil, brush, calligraphy, marker and the highlighter
//! (H.3), each a preset of width curve, steadying, texture, blend, colour and, for calligraphy, a nib angle. A pen
//! turns a stroke's samples into its points ([`Pen::stroke`]); [`crate::raster`] draws them as that pen does.

use std::fmt;

use crate::sample::Sample;
use crate::stroke::{InkPoint, Stroke};
use crate::width::WidthCurve;

/// A pen's size, in points, until the reader picks another.
pub const DEFAULT_SIZE: f32 = 2.4;
/// The pens' ink, `0xRRGGBB`: a deep blue.
pub const INK: u32 = 0x1a3d8f;
/// The highlighter's: a warm yellow, light enough that black text multiplied by it stays black on bright paper.
pub const HIGHLIGHTER: u32 = 0xffd84d;
/// How much wider than its size a highlighter draws: about a line of text at the default size.
const HIGHLIGHTER_SPAN: f32 = 4.0;

/// The kinds of pen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PenKind {
    /// Wet ink that swells with pressure and thins with speed.
    #[default]
    Fountain,
    /// An even line that barely follows pressure.
    Ballpoint,
    /// Grainy graphite that shades wider and lighter as the pen leans.
    Pencil,
    /// A soft brush: a hair at a touch, broad when pressed.
    Brush,
    /// A flat nib held at an angle: broad across it, thin along it.
    Calligraphy,
    /// A felt marker: broad, even and opaque.
    Marker,
    /// A broad chisel of light ink that darkens rather than covers, so the text under it stays readable (H.3).
    Highlighter,
}

impl PenKind {
    /// Every kind, in the toolbar's order.
    pub const ALL: [Self; 7] = [
        Self::Fountain,
        Self::Ballpoint,
        Self::Pencil,
        Self::Brush,
        Self::Calligraphy,
        Self::Marker,
        Self::Highlighter,
    ];

    /// The kind named `name` (as [`fmt::Display`] writes it).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fountain => "fountain",
            Self::Ballpoint => "ballpoint",
            Self::Pencil => "pencil",
            Self::Brush => "brush",
            Self::Calligraphy => "calligraphy",
            Self::Marker => "marker",
            Self::Highlighter => "highlighter",
        }
    }

    /// The pen of this kind, `size` points at its widest under a firm hand.
    #[must_use]
    pub fn pen(self, size: f32) -> Pen {
        let size = size.max(0.1);
        let curve = |min: f32, exponent: f32, tilt: f32, speed: f32| WidthCurve {
            min: size * min,
            max: size,
            exponent,
            tilt,
            speed,
        };
        let pen = Pen {
            kind: self,
            curve: curve(0.35, 1.25, 0.0, 0.15),
            steadiness: 0.2,
            texture: Texture::Smooth,
            blend: Blend::Normal,
            opacity: 1.0,
            color: INK,
            nib: None,
        };
        match self {
            Self::Fountain => pen,
            Self::Ballpoint => Pen {
                curve: curve(0.8, 1.0, 0.0, 0.0),
                steadiness: 0.3,
                ..pen
            },
            Self::Pencil => Pen {
                curve: curve(0.5, 1.0, 1.5, 0.0),
                texture: Texture::Grain { amount: 0.45 },
                opacity: 0.85,
                ..pen
            },
            Self::Brush => Pen {
                curve: curve(0.05, 2.0, 0.0, 0.3),
                steadiness: 0.35,
                ..pen
            },
            Self::Calligraphy => Pen {
                curve: curve(0.6, 1.0, 0.0, 0.0),
                nib: Some(Nib {
                    angle: 45.0,
                    thinnest: 0.15,
                }),
                ..pen
            },
            Self::Marker => Pen {
                curve: curve(1.0, 1.0, 0.0, 0.0),
                steadiness: 0.1,
                ..pen
            },
            Self::Highlighter => Pen {
                curve: WidthCurve {
                    min: size * HIGHLIGHTER_SPAN,
                    max: size * HIGHLIGHTER_SPAN,
                    exponent: 1.0,
                    tilt: 0.0,
                    speed: 0.0,
                },
                steadiness: 0.3,
                blend: Blend::Multiply,
                color: HIGHLIGHTER,
                ..pen
            },
        }
    }
}

impl fmt::Display for PenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How a pen's line is filled.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Texture {
    /// Solid.
    #[default]
    Smooth,
    /// Paper grain shows through: `amount` (0 to 1) of the coverage the grain can take away.
    Grain {
        /// How much of the coverage the grain can take away, 0 to 1.
        amount: f32,
    },
}

/// How a pen's line mixes with what is under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    /// Covers it.
    #[default]
    Normal,
    /// Darkens it, so text under it stays readable (the highlighter, H.3).
    Multiply,
}

impl Blend {
    /// `ink` laid over `under` (RGB from 0 to 1) at `alpha`, as PDF composites them.
    #[must_use]
    pub fn over(self, under: [f32; 3], ink: [f32; 3], alpha: f32) -> [f32; 3] {
        std::array::from_fn(|i| {
            let mixed = match self {
                Self::Normal => ink[i],
                Self::Multiply => under[i] * ink[i],
            };
            under[i] + (mixed - under[i]) * alpha
        })
    }
}

/// A flat nib (calligraphy): the line is broadest moving across the nib and thinnest moving along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nib {
    /// The nib's angle, in degrees clockwise from the right.
    pub angle: f32,
    /// The width moving along the nib, as a share of the full width.
    pub thinnest: f32,
}

impl Nib {
    /// The share of the full width a stroke moving by (`dx`, `dy`) gets.
    #[must_use]
    pub fn share(&self, dx: f32, dy: f32) -> f32 {
        if dx == 0.0 && dy == 0.0 {
            return 1.0;
        }
        let across = (dy.atan2(dx) - self.angle.to_radians()).sin().abs();
        self.thinnest + (1.0 - self.thinnest) * across
    }
}

/// A pen: its kind and everything that makes its line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pen {
    /// Its kind.
    pub kind: PenKind,
    /// How its width follows pressure, tilt and speed.
    pub curve: WidthCurve,
    /// How much a finished stroke is steadied, 0 none, towards 1 heavy.
    pub steadiness: f32,
    /// How the line is filled.
    pub texture: Texture,
    /// How the line mixes with the page.
    pub blend: Blend,
    /// 0 (clear) to 1.
    pub opacity: f32,
    /// Its colour, `0xRRGGBB`.
    // shortcut: each pen's own colour until H.7's favourites let the reader pick one.
    pub color: u32,
    /// A flat nib's angle, for calligraphy.
    pub nib: Option<Nib>,
}

impl Default for Pen {
    fn default() -> Self {
        PenKind::default().pen(DEFAULT_SIZE)
    }
}

impl Pen {
    /// The stroke `samples` (page points, oldest first) draw with this pen: widths from its curve, and a flat nib's
    /// share by the way the stroke moves at each point.
    #[must_use]
    pub fn stroke(&self, samples: &[Sample]) -> Stroke {
        let mut stroke = Stroke::new();
        for &sample in samples {
            self.push(&mut stroke, sample);
        }
        stroke
    }

    /// Adds `sample` (page points) to `stroke` as this pen draws it: its width from the curve, and a flat nib's share
    /// by the way the stroke moved to it (the first point takes the second's way).
    pub fn push(&self, stroke: &mut Stroke, sample: Sample) {
        let Some(nib) = self.nib else {
            stroke.push_sample(sample, &self.curve);
            return;
        };
        let count = stroke.len();
        let before = stroke.points().last().copied();
        stroke.push_sample(sample, &self.curve);
        let Some(before) = before else {
            return;
        };
        if stroke.len() == count {
            // The pen held still: the point keeps the width its nib gave it.
            stroke.set_width(count - 1, before.width);
            return;
        }
        let share = nib.share(sample.x - before.x, sample.y - before.y);
        let width = stroke.points()[count].width;
        stroke.set_width(count, width * share);
        if count == 1 {
            stroke.set_width(0, before.width * share);
        }
    }

    /// `stroke`'s points as they are saved and drawn: steadied as this pen is, smoothed within `tolerance` points.
    #[must_use]
    pub fn finish(&self, stroke: &Stroke, tolerance: f32) -> Vec<InkPoint> {
        stroke.smoothed(self.steadiness, tolerance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_name() {
        for kind in PenKind::ALL {
            assert_eq!(PenKind::parse(&kind.to_string()), Some(kind));
        }
        assert_eq!(PenKind::parse("crayon"), None);
    }

    #[test]
    fn a_nib_is_broad_across_and_thin_along() {
        let nib = Nib {
            angle: 45.0,
            thinnest: 0.2,
        };
        let along = nib.share(1.0, 1.0);
        let across = nib.share(1.0, -1.0);
        assert!((along - 0.2).abs() < 1e-5, "{along}");
        assert!((across - 1.0).abs() < 1e-5, "{across}");
        assert_eq!(nib.share(0.0, 0.0), 1.0);
    }

    #[test]
    fn calligraphy_changes_width_with_direction() {
        let pen = PenKind::Calligraphy.pen(4.0);
        // Down-right runs along a 45° nib, up-right across it.
        let along = pen.stroke(&[
            Sample::at(0.0, 0.0, 0.0),
            Sample::at(10.0, 10.0, 8.0),
            Sample::at(20.0, 20.0, 16.0),
        ]);
        let across = pen.stroke(&[
            Sample::at(0.0, 20.0, 0.0),
            Sample::at(10.0, 10.0, 8.0),
            Sample::at(20.0, 0.0, 16.0),
        ]);
        assert!(along.points()[1].width * 3.0 < across.points()[1].width);
    }

    #[test]
    fn multiply_keeps_dark_text_dark_where_normal_covers_it() {
        let yellow = [1.0, 0.85, 0.3];
        assert_eq!(Blend::Multiply.over([0.0; 3], yellow, 1.0), [0.0; 3]);
        assert_eq!(Blend::Multiply.over([1.0; 3], yellow, 1.0), yellow);
        assert_eq!(Blend::Normal.over([0.0; 3], yellow, 1.0), yellow);
        let half = Blend::Normal.over([0.0; 3], [1.0; 3], 0.5);
        assert_eq!(half, [0.5; 3]);
        let highlighter = PenKind::Highlighter.pen(3.0);
        assert_eq!(highlighter.blend, Blend::Multiply);
        assert!(highlighter.curve.width(Some(0.1), None, 0.0) >= 12.0);
    }

    #[test]
    fn a_marker_is_even_and_a_brush_follows_the_hand() {
        let marker = PenKind::Marker.pen(5.0);
        let light = marker.curve.width(Some(0.1), None, 0.0);
        let firm = marker.curve.width(Some(1.0), None, 0.0);
        assert_eq!(light, firm);
        let brush = PenKind::Brush.pen(5.0);
        assert!(
            brush.curve.width(Some(0.1), None, 0.0) * 10.0
                < brush.curve.width(Some(1.0), None, 0.0)
        );
    }
}
