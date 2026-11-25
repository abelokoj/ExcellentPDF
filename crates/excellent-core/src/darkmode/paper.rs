//! Paper texture (R.23): a faint grain laid over the pages as the canvas draws them, never baked into the tiles, so
//! the document's own pixels, search, copy and print stay as they are. The grain is one square of soft fibre noise,
//! the same on every run, stretched over each page so it moves and scales with the paper.

use excellent_mupdf::Pixmap;

/// The grain's side, in pixels.
pub const SIZE: u32 = 512;

/// How strongly the canvas lays the grain over the page, from 0 (none) to 1: barely there, so text keeps its contrast.
pub const DEFAULT_STRENGTH: f32 = 0.06;

/// The grain, as RGB pixels `SIZE` by `SIZE`: light (from about 200 to 255), with soft fibres.
#[must_use]
pub fn grain() -> Vec<u8> {
    let side = SIZE as usize;
    // White noise, then one pass of a 3 by 3 box blur, wrapping at the edges so the square tiles without a seam.
    let noise: Vec<f32> = (0..side * side).map(|i| hash(i as u32)).collect();
    let at = |x: isize, y: isize| {
        let wrap = |v: isize| v.rem_euclid(side as isize) as usize;
        noise[wrap(y) * side + wrap(x)]
    };
    let mut out = Vec::with_capacity(side * side * 3);
    for y in 0..side as isize {
        for x in 0..side as isize {
            let mut sum = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    sum += at(x + dx, y + dy);
                }
            }
            // Mostly light, with the odd darker fibre.
            let soft = (sum / 9.0).powf(0.6);
            let level = (200.0 + soft * 55.0).round() as u8;
            out.extend_from_slice(&[level, level, level.saturating_sub(3)]);
        }
    }
    out
}

/// The grain in a pixmap, for the canvas to upload.
///
/// # Errors
///
/// MuPDF can't allocate the pixmap.
pub fn grain_pixmap() -> excellent_mupdf::Result<Pixmap> {
    let mut pixmap = Pixmap::new_rgb(SIZE, SIZE)?;
    let rgb = grain();
    let (stride, n) = (pixmap.stride(), pixmap.components());
    let width = SIZE as usize;
    for (row, line) in pixmap
        .samples_mut()
        .chunks_exact_mut(stride)
        .enumerate()
        .take(width)
    {
        for (x, px) in line[..width * n].chunks_exact_mut(n).enumerate() {
            px[..3].copy_from_slice(&rgb[(row * width + x) * 3..][..3]);
        }
    }
    Ok(pixmap)
}

/// A number from 0 to 1 for `i`, the same on every run.
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grain_is_light_and_varied_and_the_same_every_time() {
        let grain = grain();
        assert_eq!(grain.len(), (SIZE * SIZE * 3) as usize);
        let lightest = *grain.iter().max().expect("pixels");
        let darkest = *grain.iter().min().expect("pixels");
        assert!(darkest >= 190, "never dark: {darkest}");
        assert!(lightest - darkest >= 20, "a grain, not a flat colour");
        assert_eq!(grain, super::grain(), "the same on every run");
    }

    #[test]
    fn the_grain_tiles_without_a_seam() {
        let grain = grain();
        let side = SIZE as usize;
        let level = |x: usize, y: usize| f32::from(grain[(y * side + x) * 3]);
        // Neighbours across the seam differ no more than neighbours inside.
        let across: f32 = (0..side)
            .map(|y| (level(side - 1, y) - level(0, y)).abs())
            .sum();
        let inside: f32 = (0..side)
            .map(|y| (level(100, y) - level(101, y)).abs())
            .sum();
        assert!(
            across < inside * 1.5,
            "{across} across the seam, {inside} inside"
        );
    }
}
