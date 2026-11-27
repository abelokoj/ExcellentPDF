//! A small software rasterizer for strokes: a stroke as a pen draws it, as coverage from 0 to 255 in a grey image.
//! The golden-image tests compare pens with it (H.2); it needs no GPU and gives the same pixels everywhere, up to
//! rounding.

use crate::pens::{Pen, Texture};
use crate::stroke::InkPoint;

/// A grey image of coverage, row by row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    /// Its width in pixels.
    pub width: usize,
    /// Its height in pixels.
    pub height: usize,
    /// Coverage, 0 (none) to 255 (full), `width` × `height`.
    pub pixels: Vec<u8>,
}

/// Where the raster lies over the page: the page point at its top-left, and pixels per page point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The page point across at the raster's left edge.
    pub left: f32,
    /// The page point down at its top edge.
    pub top: f32,
    /// Pixels per page point.
    pub scale: f32,
}

impl Raster {
    /// `points` (a finished stroke, page points) drawn as `pen` draws them into a `width` × `height` raster laid
    /// over the page as `frame` says.
    #[must_use]
    pub fn draw(points: &[InkPoint], pen: &Pen, width: usize, height: usize, frame: Frame) -> Self {
        let mut coverage = vec![0.0_f32; width * height];
        let to_pixels = |p: &InkPoint| InkPoint {
            x: (p.x - frame.left) * frame.scale,
            y: (p.y - frame.top) * frame.scale,
            width: p.width * frame.scale,
        };
        let pixels: Vec<InkPoint> = points.iter().map(to_pixels).collect();
        match pixels[..] {
            [] => {}
            [one] => capsule(&mut coverage, width, height, one, one),
            _ => {
                for pair in pixels.windows(2) {
                    capsule(&mut coverage, width, height, pair[0], pair[1]);
                }
            }
        }
        let opacity = pen.opacity.clamp(0.0, 1.0);
        let pixels = coverage
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let grain = match pen.texture {
                    Texture::Smooth => 1.0,
                    Texture::Grain { amount } => {
                        1.0 - amount.clamp(0.0, 1.0) * noise(i % width, i / width)
                    }
                };
                (c * opacity * grain * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self {
            width,
            height,
            pixels,
        }
    }

    /// As a binary PGM file.
    #[must_use]
    pub fn to_pgm(&self) -> Vec<u8> {
        let mut bytes = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        bytes.extend_from_slice(&self.pixels);
        bytes
    }

    /// The binary PGM file `bytes`, as [`Raster::to_pgm`] writes them; `None` for anything else.
    #[must_use]
    pub fn from_pgm(bytes: &[u8]) -> Option<Self> {
        let mut fields = Vec::with_capacity(4);
        let mut at = 0;
        while fields.len() < 4 {
            while bytes.get(at)?.is_ascii_whitespace() {
                at += 1;
            }
            let start = at;
            while !bytes.get(at)?.is_ascii_whitespace() {
                at += 1;
            }
            fields.push(std::str::from_utf8(&bytes[start..at]).ok()?);
        }
        let [magic, width, height, max] = fields[..] else {
            return None;
        };
        let (width, height): (usize, usize) = (width.parse().ok()?, height.parse().ok()?);
        let pixels = bytes.get(at + 1..)?;
        (magic == "P5" && max == "255" && pixels.len() == width * height).then(|| Self {
            width,
            height,
            pixels: pixels.to_vec(),
        })
    }

    /// How many pixels differ from `other`'s by more than `levels`; every pixel when the sizes differ.
    #[must_use]
    pub fn differing(&self, other: &Self, levels: u8) -> usize {
        if (self.width, self.height) != (other.width, other.height) {
            return self.pixels.len().max(other.pixels.len());
        }
        self.pixels
            .iter()
            .zip(&other.pixels)
            .filter(|(a, b)| a.abs_diff(**b) > levels)
            .count()
    }
}

/// Covers the round-ended band from `a` to `b` (pixels), its radius going from half `a`'s width to half `b`'s, keeping
/// the most coverage each pixel already has so overlapping pieces don't darken.
fn capsule(coverage: &mut [f32], width: usize, height: usize, a: InkPoint, b: InkPoint) {
    let (ra, rb) = (a.width / 2.0, b.width / 2.0);
    let reach = ra.max(rb) + 1.0;
    let span = |from: f32, to: f32, size: usize| {
        let low = (from.min(to) - reach).floor().max(0.0) as usize;
        let high = ((from.max(to) + reach).ceil().max(0.0) as usize).min(size);
        low..high
    };
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx * dx + dy * dy;
    for y in span(a.y, b.y, height) {
        for x in span(a.x, b.x, width) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if length == 0.0 {
                0.0
            } else {
                (((px - a.x) * dx + (py - a.y) * dy) / length).clamp(0.0, 1.0)
            };
            let distance = (px - (a.x + t * dx)).hypot(py - (a.y + t * dy));
            let radius = ra + (rb - ra) * t;
            let covered = (radius - distance + 0.5).clamp(0.0, 1.0);
            let pixel = &mut coverage[y * width + x];
            *pixel = pixel.max(covered);
        }
    }
}

/// Paper grain at pixel (`x`, `y`): the same everywhere for the same pixel, from 0 to 1.
fn noise(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pens::PenKind;

    const FRAME: Frame = Frame {
        left: 0.0,
        top: 0.0,
        scale: 1.0,
    };

    fn at(x: f32, y: f32, width: f32) -> InkPoint {
        InkPoint { x, y, width }
    }

    #[test]
    fn a_line_covers_its_band_and_nothing_else() {
        let pen = PenKind::Marker.pen(4.0);
        let raster = Raster::draw(
            &[at(5.0, 10.0, 4.0), at(25.0, 10.0, 4.0)],
            &pen,
            30,
            20,
            FRAME,
        );
        let at = |x: usize, y: usize| raster.pixels[y * 30 + x];
        assert_eq!(at(15, 10), 255, "the middle of the line");
        assert_eq!(at(15, 15), 0, "well off it");
        assert_eq!(at(1, 10), 0, "past its round end");
    }

    #[test]
    fn grain_and_opacity_lighten_a_pencil() {
        let line = [at(5.0, 10.0, 6.0), at(25.0, 10.0, 6.0)];
        let ink = Raster::draw(&line, &PenKind::Ballpoint.pen(6.0), 30, 20, FRAME);
        let pencil = Raster::draw(&line, &PenKind::Pencil.pen(6.0), 30, 20, FRAME);
        let sum = |r: &Raster| r.pixels.iter().map(|&p| u32::from(p)).sum::<u32>();
        assert!(sum(&pencil) < sum(&ink));
        assert!(
            pencil.pixels.iter().any(|&p| p > 0 && p < 200),
            "grain shows"
        );
    }

    #[test]
    fn pgm_goes_out_and_back() {
        let raster = Raster::draw(&[at(3.0, 3.0, 4.0)], &Pen::default(), 8, 6, FRAME);
        let back = Raster::from_pgm(&raster.to_pgm()).expect("reads");
        assert_eq!(back, raster);
        assert_eq!(back.differing(&raster, 0), 0);
        assert_eq!(Raster::from_pgm(b"P6\n1 1\n255\n\0"), None);
    }
}
