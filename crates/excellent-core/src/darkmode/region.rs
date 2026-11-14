//! Which part of a page keeps its colours (Enhanced's `BuildPageDarkLegacySkipRects` and its helpers): at most one
//! region, the largest picture that is artwork or a photo, so a page of small images is not left patchy (#5806).

use excellent_mupdf::PageRect;

use super::stats::{SampleStats, page_dominant_recolors};

/// Pictures smaller than this on screen, in pixels either way, follow the page.
const MIN_KEPT_PIXELS: f32 = 72.0;

/// One picture a page draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageOnPage {
    /// Where it lands, in page points.
    pub bounds: PageRect,
    /// Its own width in pixels (0 when unknown).
    pub width: u32,
    /// Its own height in pixels (0 when unknown).
    pub height: u32,
}

/// The region of `page` (its bounds, in points) whose pictures keep their colours at `scale` pixels a point, if
/// any. `stats(i)` samples picture `i` (`None` when it can't be decoded); it is asked only when a rule needs it.
pub fn kept_region(
    images: &[ImageOnPage],
    page: PageRect,
    scale: f32,
    mut stats: impl FnMut(usize) -> Option<SampleStats>,
) -> Option<PageRect> {
    let page_area = match area(&page) {
        a if a > 0.0 => a,
        _ => 1.0,
    };
    let rects: Vec<PageRect> = images
        .iter()
        .map(|image| {
            if image.width > 0 && image.height > 0 {
                clamp_to_image(image.bounds, image.width, image.height)
            } else {
                cap_unknown(image.bounds, page.height())
            }
        })
        .collect();
    let on_page: Vec<PageRect> = rects.iter().map(|r| intersect(r, &page)).collect();
    // Tiles of one sliced illustration are judged as the region they form.
    let tolerance = (page.width().min(page.height()) * 0.005).max(1.0);
    let (group_of, groups) = group_tiles(&on_page, tolerance);
    let mut taken = vec![false; groups.len()];
    let mut best: Option<PageRect> = None;
    for (i, rect) in rects.iter().enumerate() {
        let group = group_of[i];
        let region = groups[group];
        let coverage = area(&region) / page_area;
        let mut sampled: Option<Option<SampleStats>> = None;
        let mut stats_of = |i| *sampled.get_or_insert_with(|| stats(i));
        // Full-bleed backgrounds and scans recolour with the page; artwork that fills it doesn't.
        if coverage >= 0.75 && page_dominant_recolors(stats_of(i).as_ref(), coverage) {
            continue;
        }
        if is_decorative_strip(&region, &page) {
            continue;
        }
        if rect.width() * scale < MIN_KEPT_PIXELS || rect.height() * scale < MIN_KEPT_PIXELS {
            continue;
        }
        let Some(stats) = stats_of(i) else { continue };
        if !stats.keeps_colors(coverage) {
            continue;
        }
        // A wide box often spans a layout column: keep it only when it is clearly a dark painting.
        if region.width() > page.width() * 0.44 && !stats.looks_like_dark_artwork(coverage) {
            continue;
        }
        if area(&region) > 0.0 && !taken[group] {
            taken[group] = true;
            if best.is_none_or(|b| area(&region) > area(&b)) {
                best = Some(region);
            }
        }
    }
    best
}

/// A picture's box shrunk to its own shape when the two disagree a lot: a painting drawn at the top of a tall column,
/// or across a whole column.
fn clamp_to_image(rect: PageRect, width: u32, height: u32) -> PageRect {
    if area(&rect) <= 0.0 {
        return rect;
    }
    let image_aspect = width as f32 / height as f32;
    let box_aspect = rect.width() / rect.height();
    const MAX_SKEW: f32 = 1.40;
    let (mut w, mut h) = (rect.width(), rect.height());
    if box_aspect < image_aspect / MAX_SKEW {
        h = (rect.width() / image_aspect).min(rect.height());
    }
    if box_aspect > image_aspect * MAX_SKEW {
        w = w.min(rect.height() * image_aspect);
    }
    PageRect {
        x1: rect.x0 + w,
        y1: rect.y0 + h,
        ..rect
    }
}

/// A box of unknown pixel size, capped at about half the page's height.
fn cap_unknown(rect: PageRect, page_height: f32) -> PageRect {
    let max = page_height * 0.48;
    if area(&rect) <= 0.0 || page_height <= 0.0 || rect.height() <= max {
        return rect;
    }
    PageRect {
        y1: rect.y0 + max,
        ..rect
    }
}

/// Spiral-binding margins, side shadows and rules: long thin strips, or narrow bands along a good part of the page.
fn is_decorative_strip(rect: &PageRect, page: &PageRect) -> bool {
    let (w, h) = (rect.width(), rect.height());
    if w <= 0.0 || h <= 0.0 || page.width() <= 0.0 || page.height() <= 0.0 {
        return false;
    }
    let (w_frac, h_frac) = (w / page.width(), h / page.height());
    w.min(h) / w.max(h) < 0.22
        || (w_frac < 0.20 && h_frac > 0.30)
        || (h_frac < 0.20 && w_frac > 0.30)
}

/// Groups boxes that touch (within `tolerance`) when together they fill their bounding box (85 %): an
/// illustration sliced into tiles. Returns each box's group and each group's box; a box that groups with nothing
/// is a group of its own.
fn group_tiles(rects: &[PageRect], tolerance: f32) -> (Vec<usize>, Vec<PageRect>) {
    let n = rects.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..n {
        if area(&rects[i]) <= 0.0 {
            continue;
        }
        let grown = inflate(&rects[i], tolerance);
        for (j, other) in rects.iter().enumerate().skip(i + 1) {
            if area(other) <= 0.0 || area(&intersect(&grown, other)) <= 0.0 {
                continue;
            }
            let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut bbox = vec![PageRect::default(); n];
    let mut tile_area = vec![0.0f32; n];
    let mut tiles = vec![0usize; n];
    for (i, rect) in rects.iter().enumerate() {
        if area(rect) <= 0.0 {
            continue;
        }
        let r = root(&mut parent, i);
        bbox[r] = if tiles[r] == 0 {
            *rect
        } else {
            union(&bbox[r], rect)
        };
        tile_area[r] += area(rect);
        tiles[r] += 1;
    }
    for i in 0..n {
        let whole = area(&bbox[i]);
        if tiles[i] >= 2 && (whole <= 0.0 || tile_area[i] < whole * 0.85) {
            tiles[i] = 0; // scattered pictures, not a tiling
        }
    }
    let mut group_of = Vec::with_capacity(n);
    let mut groups = Vec::new();
    let mut root_group = vec![None; n];
    for (i, rect) in rects.iter().enumerate() {
        let r = root(&mut parent, i);
        if area(rect) <= 0.0 || tiles[r] < 2 {
            group_of.push(groups.len());
            groups.push(*rect);
            continue;
        }
        let group = *root_group[r].get_or_insert_with(|| {
            groups.push(bbox[r]);
            groups.len() - 1
        });
        group_of.push(group);
    }
    (group_of, groups)
}

/// Area of a box; 0 when it is empty.
fn area(r: &PageRect) -> f32 {
    if r.width() <= 0.0 || r.height() <= 0.0 {
        0.0
    } else {
        r.width() * r.height()
    }
}

/// The overlap of two boxes, empty (all zero) when they don't overlap.
fn intersect(a: &PageRect, b: &PageRect) -> PageRect {
    let r = PageRect {
        x0: a.x0.max(b.x0),
        y0: a.y0.max(b.y0),
        x1: a.x1.min(b.x1),
        y1: a.y1.min(b.y1),
    };
    if area(&r) > 0.0 {
        r
    } else {
        PageRect::default()
    }
}

fn union(a: &PageRect, b: &PageRect) -> PageRect {
    PageRect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn inflate(r: &PageRect, by: f32) -> PageRect {
    PageRect {
        x0: r.x0 - by,
        y0: r.y0 - by,
        x1: r.x1 + by,
        y1: r.y1 + by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: PageRect = PageRect {
        x0: 0.0,
        y0: 0.0,
        x1: 600.0,
        y1: 800.0,
    };

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> PageRect {
        PageRect { x0, y0, x1, y1 }
    }

    fn image(bounds: PageRect) -> ImageOnPage {
        // Pixels in the box's own shape, so no clamping.
        ImageOnPage {
            bounds,
            width: bounds.width() as u32,
            height: bounds.height() as u32,
        }
    }

    fn photo() -> Option<SampleStats> {
        Some(SampleStats {
            significant_buckets: 40,
            lum_var: 0.05,
            sat_ratio: 0.4,
            high_lum_ratio: 0.3,
            border_light_ratio: 0.2,
            border_uniformity: 0.3,
        })
    }

    #[test]
    fn the_largest_photo_is_kept_and_small_ones_follow_the_page() {
        let images = [
            image(rect(50.0, 50.0, 250.0, 200.0)),
            image(rect(300.0, 300.0, 520.0, 500.0)),
            image(rect(50.0, 600.0, 90.0, 640.0)),
        ];
        let kept = kept_region(&images, PAGE, 1.0, |_| photo());
        assert_eq!(kept, Some(images[1].bounds));
    }

    #[test]
    fn a_picture_too_small_on_screen_is_not_kept_until_zoomed() {
        let images = [image(rect(50.0, 50.0, 110.0, 110.0))];
        assert_eq!(kept_region(&images, PAGE, 1.0, |_| photo()), None);
        assert_eq!(
            kept_region(&images, PAGE, 2.0, |_| photo()),
            Some(images[0].bounds)
        );
    }

    #[test]
    fn a_page_sized_scan_and_an_undecodable_picture_follow_the_page() {
        let scan = [image(PAGE)];
        assert_eq!(kept_region(&scan, PAGE, 1.0, |_| None), None);
        let small = [image(rect(50.0, 50.0, 250.0, 250.0))];
        assert_eq!(kept_region(&small, PAGE, 1.0, |_| None), None);
    }

    #[test]
    fn a_sliced_illustration_is_kept_whole() {
        let images = [
            image(rect(100.0, 100.0, 200.0, 200.0)),
            image(rect(200.0, 100.0, 300.0, 200.0)),
            image(rect(100.0, 200.0, 200.0, 300.0)),
            image(rect(200.0, 200.0, 300.0, 300.0)),
        ];
        let kept = kept_region(&images, PAGE, 1.0, |_| photo());
        assert_eq!(kept, Some(rect(100.0, 100.0, 300.0, 300.0)));
    }

    #[test]
    fn strips_are_decoration() {
        assert!(is_decorative_strip(&rect(0.0, 0.0, 30.0, 800.0), &PAGE));
        assert!(
            is_decorative_strip(&rect(100.0, 100.0, 300.0, 250.0), &PAGE),
            "under a fifth of the page high and over a third wide is a band, as Enhanced has it"
        );
        assert!(!is_decorative_strip(
            &rect(100.0, 100.0, 250.0, 300.0),
            &PAGE
        ));
    }

    #[test]
    fn a_box_much_taller_than_its_picture_is_trimmed() {
        let trimmed = clamp_to_image(rect(0.0, 0.0, 200.0, 600.0), 400, 300);
        assert_eq!(trimmed, rect(0.0, 0.0, 200.0, 150.0));
        assert_eq!(cap_unknown(rect(0.0, 0.0, 100.0, 700.0), 800.0).y1, 384.0);
    }

    #[test]
    fn the_photo_is_sampled_only_when_a_rule_needs_it() {
        let images = [
            image(rect(0.0, 0.0, 20.0, 800.0)),
            image(rect(50.0, 50.0, 60.0, 60.0)),
        ];
        let mut asked = 0;
        kept_region(&images, PAGE, 1.0, |_| {
            asked += 1;
            photo()
        });
        assert_eq!(
            asked, 0,
            "a strip and a speck are ruled out without decoding them"
        );
    }
}
