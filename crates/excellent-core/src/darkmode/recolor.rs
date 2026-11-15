//! Maps a rendered tile from black-on-white to the theme's ink and paper (Enhanced's `RecolorPixels`).

use excellent_mupdf::DeviceRect;

use super::PageColors;

/// Recolours `samples` (rows `stride` bytes apart, `components` bytes a pixel, red first; a fourth byte, alpha, is
/// left alone) in place: each channel goes from `ink` at 0 to `paper` at 255, and blue-ish link pixels go to
/// `link`. Pixels in `keep` (in the tile's own pixels) stay as they are.
///
/// # Panics
///
/// `components` is less than 3, or `samples` is shorter than `height` rows.
pub fn recolor(
    samples: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    components: usize,
    colors: &PageColors,
    keep: Option<DeviceRect>,
) {
    assert!(
        components >= 3,
        "recolour needs RGB pixels, not {components} bytes a pixel"
    );
    if colors.is_plain() && keep.is_none() {
        return;
    }
    let ink = colors.ink.map(i32::from);
    let diff = [0, 1, 2].map(|i| i32::from(colors.paper[i]) - ink[i]);
    let link = colors.link.map(|link| {
        let link = link.map(i32::from);
        let toward_paper = [0, 1, 2].map(|i| i32::from(colors.paper[i]) - link[i]);
        (link, toward_paper)
    });
    let kept = |x: usize, y: usize| {
        keep.is_some_and(|k| {
            let (x, y) = (x as i64, y as i64);
            x >= i64::from(k.x0)
                && x < i64::from(k.x1)
                && y >= i64::from(k.y0)
                && y < i64::from(k.y1)
        })
    };
    for y in 0..height {
        let row = &mut samples[y * stride..y * stride + width * components];
        for (x, pixel) in row.chunks_exact_mut(components).enumerate() {
            if kept(x, y) {
                continue;
            }
            let [r, g, b] = [0, 1, 2].map(|i| i32::from(pixel[i]));
            if let Some((link, toward_paper)) = link
                && b >= r.max(g) + 25
                && b >= 72
                && (r + g + b) / 3 <= 230
            {
                let rg = (r + g) / 2;
                for i in 0..3 {
                    pixel[i] = channel(link[i] + mul255(rg, toward_paper[i]));
                }
                continue;
            }
            for i in 0..3 {
                pixel[i] = channel(ink[i] + mul255(i32::from(pixel[i]), diff[i]));
            }
        }
    }
}

/// `a × b / 255`, rounded, as Enhanced computes it (`b` may be negative).
fn mul255(a: i32, b: i32) -> i32 {
    let n = a * b + 128;
    (n + (n >> 8)) >> 8
}

fn channel(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).expect("clamped to a byte")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::darkmode::Pictures;

    const DARK: PageColors = PageColors {
        ink: [230, 230, 225],
        paper: [18, 23, 36],
        link: Some([120, 170, 255]),
        pictures: Pictures::Keep,
    };

    fn one(pixel: [u8; 3], colors: &PageColors) -> [u8; 3] {
        let mut samples = pixel.to_vec();
        recolor(&mut samples, 1, 1, 3, 3, colors, None);
        [samples[0], samples[1], samples[2]]
    }

    #[test]
    fn black_becomes_ink_and_white_becomes_paper() {
        assert_eq!(one([0, 0, 0], &DARK), DARK.ink);
        assert_eq!(one([255, 255, 255], &DARK), DARK.paper);
        let grey = one([128, 128, 128], &DARK);
        assert!(grey[0] < DARK.ink[0] && grey[0] > DARK.paper[0], "{grey:?}");
    }

    #[test]
    fn link_blue_becomes_the_link_colour_and_red_does_not() {
        assert_eq!(one([0, 0, 238], &DARK), DARK.link.expect("a link colour"));
        let no_link = PageColors { link: None, ..DARK };
        assert_ne!(
            one([0, 0, 238], &no_link),
            DARK.link.expect("a link colour")
        );
        let red = one([200, 30, 30], &DARK);
        assert_ne!(red, DARK.link.expect("a link colour"));
    }

    #[test]
    fn kept_pixels_and_alpha_are_left_alone() {
        // Two RGBA pixels in a row with padding after them; the second is kept.
        let mut samples = vec![255, 255, 255, 7, 255, 255, 255, 9, 0xEE, 0xEE];
        let keep = DeviceRect {
            x0: 1,
            y0: 0,
            x1: 2,
            y1: 1,
        };
        recolor(&mut samples, 2, 1, 10, 4, &DARK, Some(keep));
        assert_eq!(&samples[..3], &DARK.paper);
        assert_eq!(samples[3], 7);
        assert_eq!(&samples[4..], &[255, 255, 255, 9, 0xEE, 0xEE]);
    }

    #[test]
    fn plain_colours_change_nothing() {
        let plain = PageColors {
            ink: [0, 0, 0],
            paper: [255, 255, 255],
            link: None,
            pictures: Pictures::Keep,
        };
        assert_eq!(one([12, 200, 99], &plain), [12, 200, 99]);
    }
}
