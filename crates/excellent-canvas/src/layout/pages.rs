//! A finished layout: every page's rectangle, grouped into bands, and the queries the viewport and canvas need.

use std::ops::Range;

use super::arrange;
use super::spec::{Flow, Mode, Spec};
use crate::Rect;

/// Space around and between pages, in points.
pub const GAP: f32 = 8.0;

/// A page's size as shown (after rotation), in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageSize {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// The direction the pages run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Axis {
    /// Rows of pages, top to bottom.
    #[default]
    Vertical,
    /// One page after the other, sideways.
    Horizontal,
}

impl Axis {
    /// The part of `r` along this axis: its top and bottom, or its left and right.
    pub fn span(self, r: &Rect) -> (f32, f32) {
        match self {
            Self::Vertical => (r.y0, r.y1),
            Self::Horizontal => (r.x0, r.x1),
        }
    }
}

/// A row of pages (a column, in the horizontal layout): the unit that paged views show and fit modes fit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Band {
    /// Where it starts along the axis, in points.
    pub start: f32,
    /// Where it ends along the axis, in points.
    pub end: f32,
    /// The pages in it.
    pub pages: Range<usize>,
}

impl Band {
    /// Its length along the axis.
    pub fn len(&self) -> f32 {
        self.end - self.start
    }
}

/// Every page's place in layout space.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    spec: Spec,
    pages: Vec<Rect>,
    /// In order along the axis; they never overlap.
    bands: Vec<Band>,
    /// The band each page is in.
    band_of: Vec<usize>,
    width: f32,
    height: f32,
}

impl Layout {
    /// The pages of `sizes`, laid out as `spec` says.
    pub fn new(sizes: &[PageSize], spec: Spec) -> Self {
        arrange::arrange(sizes, spec)
    }

    /// One page under the other, scrolling (the default layout).
    pub fn continuous(sizes: &[PageSize]) -> Self {
        Self::new(sizes, Spec::default())
    }

    /// A layout from its parts, as `arrange` places them.
    pub(super) fn from_parts(
        spec: Spec,
        pages: Vec<Rect>,
        bands: Vec<Band>,
        width: f32,
        height: f32,
    ) -> Self {
        let mut band_of = vec![0; pages.len()];
        for (index, band) in bands.iter().enumerate() {
            for page in band.pages.clone() {
                band_of[page] = index;
            }
        }
        Self {
            spec,
            pages,
            bands,
            band_of,
            width,
            height,
        }
    }

    /// What it was laid out for.
    pub fn spec(&self) -> Spec {
        self.spec
    }

    /// The direction its pages run in.
    pub fn axis(&self) -> Axis {
        match self.spec.mode {
            Mode::Horizontal => Axis::Horizontal,
            _ => Axis::Vertical,
        }
    }

    /// Whether the view shows one band at a time.
    pub fn flow(&self) -> Flow {
        if self.spec.is_paged() {
            Flow::Paged
        } else {
            Flow::Continuous
        }
    }

    /// How many pages it holds.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// True when it holds no page.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Page `index`'s rectangle, if there is such a page.
    pub fn page(&self, index: usize) -> Option<Rect> {
        self.pages.get(index).copied()
    }

    /// Every page's rectangle, in page order.
    pub fn pages(&self) -> &[Rect] {
        &self.pages
    }

    /// Its bands, in order along the axis.
    pub fn bands(&self) -> &[Band] {
        &self.bands
    }

    /// The band page `index` is in.
    pub fn band_of(&self, index: usize) -> Option<usize> {
        self.band_of.get(index).copied()
    }

    /// The band at `along` (a coordinate along the axis), or the nearest one when `along` is in a gap or past an
    /// end. `None` only when there are no pages.
    pub fn band_at(&self, along: f32) -> Option<usize> {
        let last = self.bands.len().checked_sub(1)?;
        let next = self
            .bands
            .partition_point(|band| band.end <= along)
            .min(last);
        let nearer_before = next > 0
            && along < self.bands[next].start
            && along - self.bands[next - 1].end < self.bands[next].start - along;
        Some(if nearer_before { next - 1 } else { next })
    }

    /// The whole layout's width, margins included.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// The whole layout's height, margins included.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// The pages that `area` touches, with their rectangles, band by band along the axis.
    pub fn pages_in(&self, area: &Rect) -> impl Iterator<Item = (usize, Rect)> + '_ {
        let area = *area;
        let (from, to) = self.axis().span(&area);
        // Bands are in order and apart, so skip straight to the first one that can touch `area`.
        let first = self.bands.partition_point(|band| band.end <= from);
        self.bands[first..]
            .iter()
            .take_while(move |band| band.start < to)
            .flat_map(|band| band.pages.clone())
            .filter_map(move |index| {
                let page = self.pages[index];
                page.intersect(&area).map(|_| (index, page))
            })
    }

    /// The page under layout point (`x`, `y`), if one is.
    pub fn page_at(&self, x: f32, y: f32) -> Option<usize> {
        let along = match self.axis() {
            Axis::Vertical => y,
            Axis::Horizontal => x,
        };
        let band = self.bands.get(self.band_at(along)?)?;
        band.pages.clone().find(|&index| {
            let page = &self.pages[index];
            page.x0 <= x && x < page.x1 && page.y0 <= y && y < page.y1
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: PageSize = PageSize {
        width: 612.0,
        height: 792.0,
    };
    const LANDSCAPE: PageSize = PageSize {
        width: 792.0,
        height: 612.0,
    };

    #[test]
    fn pages_stack_and_centre_on_the_widest() {
        let layout = Layout::continuous(&[LETTER, LANDSCAPE]);
        assert_eq!(layout.width(), 792.0 + 2.0 * GAP);
        assert_eq!(
            layout.page(0),
            Some(Rect::new(GAP + 90.0, GAP, 612.0, 792.0))
        );
        assert_eq!(
            layout.page(1),
            Some(Rect::new(GAP, 2.0 * GAP + 792.0, 792.0, 612.0))
        );
        assert_eq!(layout.height(), 3.0 * GAP + 792.0 + 612.0);
    }

    #[test]
    fn pages_in_finds_only_the_touched_pages() {
        let layout = Layout::continuous(&[LETTER; 10]);
        let page = 792.0 + GAP;
        let area = Rect::new(0.0, 3.5 * page, 100.0, page);
        let touched: Vec<usize> = layout.pages_in(&area).map(|(i, _)| i).collect();
        assert_eq!(touched, [3, 4]);
        assert_eq!(
            layout.pages_in(&Rect::new(0.0, -50.0, 10.0, 10.0)).count(),
            0
        );
    }

    #[test]
    fn an_empty_document_has_no_extent() {
        let layout = Layout::continuous(&[]);
        assert!(layout.is_empty());
        assert_eq!(layout.height(), 0.0);
        assert_eq!(layout.band_at(10.0), None);
    }

    #[test]
    fn band_at_picks_the_nearest_band() {
        let layout = Layout::continuous(&[LETTER; 3]);
        assert_eq!(layout.band_at(-100.0), Some(0));
        assert_eq!(layout.band_at(GAP + 400.0), Some(0));
        // In the gap after page 0, nearer page 0's end.
        assert_eq!(layout.band_at(GAP + 792.0 + 1.0), Some(0));
        assert_eq!(layout.band_at(GAP + 792.0 + GAP - 1.0), Some(1));
        assert_eq!(layout.band_at(1e9), Some(2));
    }

    #[test]
    fn page_at_finds_the_page_under_a_point() {
        let layout = Layout::continuous(&[LETTER; 3]);
        let second = layout.page(1).expect("page 1");
        assert_eq!(layout.page_at(second.x0 + 5.0, second.y0 + 5.0), Some(1));
        assert_eq!(layout.page_at(1.0, 1.0), None, "in the margin");
    }
}
