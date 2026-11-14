//! Thumbnails (plan R.5), the model: how big each thumbnail is drawn, which pages to render as the list scrolls, and
//! what each rendered page is called in the disk cache. The rendering is `excellent-core::thumbnail`; the list is the
//! app's sidebar.

use std::ops::RangeInclusive;

/// The pixel width thumbnails are rendered at: sharp at 200 % display scale for the widest frame the sidebar draws.
pub const RENDER_WIDTH: u32 = 320;

/// Rows rendered beyond each end of what the list shows, so a short scroll finds them ready.
pub const MARGIN: usize = 6;

/// The frame a page of `width` × `height` points gets inside a box of `max_width` × `max_height`: as large as fits,
/// keeping the page's shape. A page with no size (not reported yet) gets a portrait sheet's shape.
pub fn frame(width: f32, height: f32, max_width: f32, max_height: f32) -> (f32, f32) {
    let (width, height) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        // A Letter sheet's shape.
        (612.0, 792.0)
    };
    let scale = (max_width / width).min(max_height / height);
    (width * scale, height * scale)
}

/// The pages to have rendered while the list shows rows `shown` of `count`: those, and [`MARGIN`] more each way.
/// `None` when nothing is shown or there are no pages.
pub fn wanted(shown: RangeInclusive<usize>, count: usize) -> Option<RangeInclusive<usize>> {
    if count == 0 || shown.is_empty() {
        return None;
    }
    let first = shown.start().saturating_sub(MARGIN).min(count - 1);
    let last = shown.end().saturating_add(MARGIN).min(count - 1);
    Some(first..=last)
}

/// The cache file name of page `page` of the file whose identity key is `file`, at page revision `revision`, rendered
/// `width` pixels wide. A changed file or page has a new name, so a stale thumbnail is never shown.
pub fn file_name(file: u64, page: usize, revision: u64, width: u32) -> String {
    format!("{file:016x}-p{page}-r{revision}-w{width}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_the_page_shape_and_fit_the_box() {
        // A tall page fills the height.
        let (w, h) = frame(500.0, 800.0, 120.0, 156.0);
        assert!((h - 156.0).abs() < 0.01 && w <= 120.0);
        assert!((w / h - 500.0 / 800.0).abs() < 0.001);
        // A landscape page fills the width.
        let (w, h) = frame(792.0, 612.0, 120.0, 156.0);
        assert!((w - 120.0).abs() < 0.01 && h < 156.0);
        // No size yet: a sheet's shape, not a zero.
        let (w, h) = frame(0.0, 0.0, 120.0, 156.0);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn the_rows_shown_and_a_margin_are_wanted() {
        assert_eq!(wanted(10..=14, 100), Some(4..=20));
        assert_eq!(wanted(0..=3, 100), Some(0..=9));
        assert_eq!(wanted(95..=99, 100), Some(89..=99));
        assert_eq!(wanted(0..=3, 2), Some(0..=1));
        #[expect(
            clippy::reversed_empty_ranges,
            reason = "the list reports nothing shown this way"
        )]
        let none = 1..=0;
        assert_eq!(wanted(none, 100), None);
        assert_eq!(wanted(0..=3, 0), None);
    }

    #[test]
    fn names_change_with_the_file_the_page_and_its_revision() {
        let name = file_name(0xabc, 4, 0, RENDER_WIDTH);
        assert_eq!(name, "0000000000000abc-p4-r0-w320.png");
        assert_ne!(name, file_name(0xabc, 4, 1, RENDER_WIDTH));
        assert_ne!(name, file_name(0xabd, 4, 0, RENDER_WIDTH));
    }
}
