//! The part of the layout on screen (plan §7.2, R.2): scroll position, zoom and fit modes, in one place for every
//! platform.
//!
//! The viewport's size is in device pixels; its position is the layout point at its top-left corner. Zoom 1.0 shows
//! the page at its printed size: one point is `pixels_per_point` pixels (the screen's dots per inch over 72).
//!
//! In a paged layout ([`Flow::Paged`]) the viewport stays inside one band (row of pages); scrolling past its end
//! turns to the next, as SumatraPDF's non-continuous views do.

use crate::Rect;
use crate::layout::{Axis, Direction, Flow, GAP, Layout};

/// The smallest zoom, as SumatraPDF's 8 %.
pub const MIN_ZOOM: f32 = 0.08;
/// The largest zoom, as SumatraPDF's 6400 %.
pub const MAX_ZOOM: f32 = 64.0;

/// The zoom steps that zoom in and out move between, as SumatraPDF's default `ZoomLevels`.
pub const ZOOM_STEPS: [f32; 24] = [
    0.0833, 0.125, 0.18, 0.25, 0.3333, 0.5, 0.6667, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0,
    10.0, 12.0, 16.0, 20.0, 24.0, 32.0, 48.0, 64.0,
];

/// How far past a band's end the viewport must already be before scrolling on turns the page, in points.
const AT_END: f32 = 0.5;

/// The first step above `zoom` (the largest step at most).
pub fn step_in(zoom: f32) -> f32 {
    ZOOM_STEPS
        .into_iter()
        .find(|&step| step > zoom * 1.001)
        .unwrap_or(MAX_ZOOM)
}

/// The first step below `zoom` (the smallest step at least).
pub fn step_out(zoom: f32) -> f32 {
    ZOOM_STEPS
        .into_iter()
        .rev()
        .find(|&step| step < zoom * 0.999)
        .unwrap_or(ZOOM_STEPS[0])
}

/// How the zoom is chosen. A fit mode is kept: resizing the window or changing the layout fits again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ZoomMode {
    /// A zoom the reader set.
    #[default]
    Custom,
    /// The pages fill the width (SumatraPDF's "fit width").
    FitWidth,
    /// The current row of pages fits whole on screen ("fit page").
    FitPage,
    /// The current row of pages fills the height ("fit height").
    FitHeight,
}

/// A side of the screen, for the Left and Right arrows (R.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The Left arrow.
    Left,
    /// The Right arrow.
    Right,
}

/// A reading position that survives a change of layout or rotation: a page, and where the viewport's top-left
/// corner is from that page's, as a fraction of the page's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// The page.
    pub page: usize,
    /// Across, in page widths.
    pub across: f32,
    /// Down, in page heights.
    pub down: f32,
}

/// What the canvas shows of its layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    width: f32,
    height: f32,
    pixels_per_point: f32,
    zoom: f32,
    mode: ZoomMode,
    x: f32,
    y: f32,
    /// The band being read: the one a paged view shows, and the one fit modes fit.
    band: usize,
}

impl Viewport {
    /// An empty viewport on a screen with `pixels_per_point` (dpi / 72), at 100 %.
    pub fn new(pixels_per_point: f32) -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            pixels_per_point,
            zoom: 1.0,
            mode: ZoomMode::Custom,
            x: 0.0,
            y: 0.0,
            band: 0,
        }
    }

    /// Its width in device pixels.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Its height in device pixels.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// The zoom: 1.0 is the printed size.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// How the zoom is chosen.
    pub fn zoom_mode(&self) -> ZoomMode {
        self.mode
    }

    /// Device pixels per layout point: the zoom on this screen.
    pub fn scale(&self) -> f32 {
        self.zoom * self.pixels_per_point
    }

    /// The screen's device pixels per point at 100 %.
    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// The layout area on screen, in points.
    pub fn visible(&self) -> Rect {
        let s = self.scale();
        Rect::new(self.x, self.y, self.width / s, self.height / s)
    }

    /// The band being read.
    pub fn band(&self) -> usize {
        self.band
    }

    /// The viewport's length along the layout's axis, in device pixels: what one screen of scrolling moves.
    pub fn screen_length(&self, layout: &Layout) -> f32 {
        match layout.axis() {
            Axis::Vertical => self.height,
            Axis::Horizontal => self.width,
        }
    }

    /// The window was resized to `width` × `height` device pixels.
    pub fn resize(&mut self, width: f32, height: f32, layout: &Layout) {
        self.width = width.max(0.0);
        self.height = height.max(0.0);
        self.refit(layout);
    }

    /// The window moved to a screen with another density; the page keeps its printed size.
    pub fn set_pixels_per_point(&mut self, pixels_per_point: f32, layout: &Layout) {
        self.pixels_per_point = pixels_per_point;
        self.refit(layout);
    }

    /// Scrolls by `dx`, `dy` device pixels (positive: right and down). In a paged layout, scrolling down at the
    /// end of the band turns to the next one, and up at its start to the one before.
    pub fn scroll_by(&mut self, dx: f32, dy: f32, layout: &Layout) {
        let s = self.scale();
        self.x += dx / s;
        let target = self.y + dy / s;
        if layout.flow() == Flow::Paged {
            let (top, bottom) = self.band_limits(layout);
            let last = layout.bands().len().saturating_sub(1);
            if dy > 0.0 && self.y >= bottom - AT_END && target > bottom && self.band < last {
                self.band += 1;
                self.y = self.band_limits(layout).0;
            } else if dy < 0.0 && self.y <= top + AT_END && target < top && self.band > 0 {
                self.band -= 1;
                self.y = self.band_limits(layout).1;
            } else {
                self.y = target;
            }
        } else {
            self.y = target;
        }
        self.clamp(layout);
    }

    /// Scrolls `pixels` toward the later pages (negative: the earlier ones), along the layout: down, or sideways
    /// in the reading direction in the horizontal layout.
    pub fn scroll_forward(&mut self, pixels: f32, layout: &Layout) {
        match (layout.axis(), layout.spec().direction) {
            (Axis::Vertical, _) => self.scroll_by(0.0, pixels, layout),
            (Axis::Horizontal, Direction::LeftToRight) => self.scroll_by(pixels, 0.0, layout),
            (Axis::Horizontal, Direction::RightToLeft) => self.scroll_by(-pixels, 0.0, layout),
        }
    }

    /// The Left or Right arrow (R.3): sideways by `pixels` device pixels along the horizontal strip or while the
    /// pages are wider than the screen; otherwise to the start of the row of pages before or after, by the reading
    /// direction: Right goes on in a left-to-right document and back in a right-to-left one.
    pub fn step_side(&mut self, side: Side, pixels: f32, layout: &Layout) {
        let sign = match side {
            Side::Left => -1.0,
            Side::Right => 1.0,
        };
        if layout.axis() == Axis::Horizontal || layout.width() > self.visible().width() + 0.5 {
            self.scroll_by(sign * pixels, 0.0, layout);
            return;
        }
        let forward = (side == Side::Right) == (layout.spec().direction == Direction::LeftToRight);
        let band = if forward {
            self.band + 1
        } else {
            let Some(before) = self.band.checked_sub(1) else {
                return;
            };
            before
        };
        if let Some(first) = layout.bands().get(band).map(|band| band.pages.start) {
            self.scroll_to_page(first, layout);
        }
    }

    /// Scrolls to the start of page `index`'s band.
    pub fn scroll_to_page(&mut self, index: usize, layout: &Layout) {
        let Some(band) = layout.band_of(index) else {
            return;
        };
        self.band = band;
        let start = layout.bands()[band].start - GAP;
        match layout.axis() {
            Axis::Vertical => self.y = start,
            Axis::Horizontal => self.x = start,
        }
        self.clamp(layout);
    }

    /// Scrolls so that layout point (`x`, `y`), on page `index`, shows at the top of the screen (a link's target,
    /// R.7). Across the layout's axis the view moves only when the point is off screen.
    pub fn scroll_to_point(&mut self, index: usize, x: f32, y: f32, layout: &Layout) {
        let Some(band) = layout.band_of(index) else {
            return;
        };
        self.band = band;
        let visible = self.visible();
        match layout.axis() {
            Axis::Vertical => {
                self.y = y - GAP;
                if !(visible.x0..=visible.x1).contains(&x) {
                    self.x = x - GAP;
                }
            }
            Axis::Horizontal => {
                self.x = x - GAP;
                if !(visible.y0..=visible.y1).contains(&y) {
                    self.y = y - GAP;
                }
            }
        }
        self.clamp(layout);
    }

    /// Multiplies the zoom by `factor`, keeping the layout point under device pixel (`at_x`, `at_y`) where it is.
    /// The zoom becomes the reader's own ([`ZoomMode::Custom`]).
    pub fn zoom_about(&mut self, factor: f32, at_x: f32, at_y: f32, layout: &Layout) {
        self.mode = ZoomMode::Custom;
        self.zoom_to(self.zoom * factor, at_x, at_y, layout);
    }

    /// Sets the zoom, keeping the centre of the viewport where it is; the zoom becomes the reader's own.
    pub fn set_zoom(&mut self, zoom: f32, layout: &Layout) {
        self.mode = ZoomMode::Custom;
        self.zoom_to(zoom, self.width / 2.0, self.height / 2.0, layout);
    }

    /// Chooses the zoom by `mode` from now on, and applies it.
    pub fn fit(&mut self, mode: ZoomMode, layout: &Layout) {
        self.mode = mode;
        self.refit(layout);
    }

    /// Zooms so the pages fill the width.
    pub fn fit_width(&mut self, layout: &Layout) {
        self.fit(ZoomMode::FitWidth, layout);
    }

    /// The reading position: the first page on screen, and where the viewport is from it.
    pub fn anchor(&self, layout: &Layout) -> Option<Anchor> {
        let (page, rect) = layout.pages_in(&self.visible()).next()?;
        Some(Anchor {
            page,
            across: (self.x - rect.x0) / rect.width().max(1.0),
            down: (self.y - rect.y0) / rect.height().max(1.0),
        })
    }

    /// The page the reader is on: the one showing most on screen (the first of equals); `None` with no pages.
    pub fn current_page(&self, layout: &Layout) -> Option<usize> {
        let visible = self.visible();
        let mut best: Option<(usize, f32)> = None;
        for (index, rect) in layout.pages_in(&visible) {
            let shown = rect
                .intersect(&visible)
                .map_or(0.0, |r| r.width() * r.height());
            if best.is_none_or(|(_, most)| shown > most) {
                best = Some((index, shown));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Shows `layout` (the old one's replacement) from `anchor`, the position [`Self::anchor`] took in the old one,
    /// fitting it again if a fit mode is on.
    pub fn relayout(&mut self, anchor: Option<Anchor>, layout: &Layout) {
        match anchor.and_then(|a| Some((a, layout.page(a.page)?))) {
            Some((anchor, rect)) => {
                self.x = rect.x0 + anchor.across * rect.width();
                self.y = rect.y0 + anchor.down * rect.height();
                self.band = layout.band_of(anchor.page).unwrap_or(0);
            }
            None => {
                (self.x, self.y, self.band) = (0.0, 0.0, 0);
            }
        }
        self.clamp(layout);
        self.refit(layout);
    }

    /// Where layout rectangle `r` is on screen, in device pixels, each edge on a whole pixel so that touching
    /// rectangles still touch.
    pub fn to_screen(&self, r: &Rect) -> Rect {
        let s = self.scale();
        Rect {
            x0: ((r.x0 - self.x) * s).round(),
            y0: ((r.y0 - self.y) * s).round(),
            x1: ((r.x1 - self.x) * s).round(),
            y1: ((r.y1 - self.y) * s).round(),
        }
    }

    /// The layout point under device pixel (`x`, `y`).
    pub fn to_layout_point(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale();
        (x / s + self.x, y / s + self.y)
    }

    /// Where layout point (`x`, `y`) is on screen, in device pixels (not rounded: strokes are drawn antialiased).
    pub fn to_screen_point(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale();
        ((x - self.x) * s, (y - self.y) * s)
    }

    /// Sets the zoom (within limits) keeping the point under device pixel (`at_x`, `at_y`).
    fn zoom_to(&mut self, zoom: f32, at_x: f32, at_y: f32, layout: &Layout) {
        let (lx, ly) = self.to_layout_point(at_x, at_y);
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let s = self.scale();
        self.x = lx - at_x / s;
        self.y = ly - at_y / s;
        self.clamp(layout);
    }

    /// Applies the fit mode, if one is on; otherwise only keeps the layout on screen.
    fn refit(&mut self, layout: &Layout) {
        let Some(zoom) = self.fit_zoom(layout) else {
            self.clamp(layout);
            return;
        };
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        // Fitting the width keeps the top of what was shown; fitting a page shows its band from the start.
        if matches!(self.mode, ZoomMode::FitPage | ZoomMode::FitHeight)
            && let Some(band) = layout.bands().get(self.band)
        {
            let start = band.start - GAP;
            match layout.axis() {
                Axis::Vertical => self.y = start,
                Axis::Horizontal => self.x = start,
            }
        }
        self.clamp(layout);
    }

    /// The zoom the fit mode asks for, if one is on and there is something to fit.
    fn fit_zoom(&self, layout: &Layout) -> Option<f32> {
        if self.mode == ZoomMode::Custom || self.width <= 0.0 || self.height <= 0.0 {
            return None;
        }
        let band = layout
            .bands()
            .get(self.band.min(layout.bands().len().checked_sub(1)?))?;
        let (wide, high) = match layout.axis() {
            Axis::Vertical => (layout.width(), band.len() + 2.0 * GAP),
            Axis::Horizontal => (band.len() + 2.0 * GAP, layout.height()),
        };
        let across = self.width / (wide * self.pixels_per_point);
        let down = self.height / (high * self.pixels_per_point);
        Some(match self.mode {
            ZoomMode::Custom => return None,
            ZoomMode::FitWidth => across,
            ZoomMode::FitHeight => down,
            ZoomMode::FitPage => across.min(down),
        })
    }

    /// The top and bottom positions the viewport may take inside the current band (paged layouts): the band with
    /// its margins, centred when it is shorter than the screen.
    fn band_limits(&self, layout: &Layout) -> (f32, f32) {
        let Some(band) = layout.bands().get(self.band) else {
            return (0.0, 0.0);
        };
        let shown = self.height / self.scale();
        let (top, length) = (band.start - GAP, band.len() + 2.0 * GAP);
        if length <= shown {
            let centred = top - (shown - length) / 2.0;
            (centred, centred)
        } else {
            (top, top + length - shown)
        }
    }

    /// Keeps the layout on screen: centred when it is smaller than the viewport, never scrolled past either end;
    /// in a paged layout, inside the current band. Outside paged layouts the current band follows the scrolling.
    fn clamp(&mut self, layout: &Layout) {
        let s = self.scale();
        let (shown_w, shown_h) = (self.width / s, self.height / s);
        self.x = centre_or_clamp(self.x, shown_w, layout.width());
        if layout.flow() == Flow::Paged {
            self.band = self.band.min(layout.bands().len().saturating_sub(1));
            let (top, bottom) = self.band_limits(layout);
            self.y = self.y.clamp(top, bottom);
        } else {
            self.y = centre_or_clamp(self.y, shown_h, layout.height());
            let middle = match layout.axis() {
                Axis::Vertical => self.y + shown_h / 2.0,
                Axis::Horizontal => self.x + shown_w / 2.0,
            };
            self.band = layout.band_at(middle).unwrap_or(0);
        }
    }
}

/// A position along one axis: centred when the content (`content` long) is shorter than what is shown, otherwise
/// kept between its two ends.
fn centre_or_clamp(position: f32, shown: f32, content: f32) -> f32 {
    if content <= shown {
        -(shown - content) / 2.0
    } else {
        position.clamp(0.0, content - shown)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::layout::{Mode, PageSize, Spec};

    const LETTER: PageSize = PageSize {
        width: 612.0,
        height: 792.0,
    };

    fn letters(count: usize) -> Layout {
        Layout::continuous(&vec![LETTER; count])
    }

    fn laid_out(mode: Mode, flow: Flow, count: usize) -> Layout {
        let spec = Spec {
            mode,
            flow,
            ..Spec::default()
        };
        Layout::new(&vec![LETTER; count], spec)
    }

    fn viewport(layout: &Layout) -> Viewport {
        let mut v = Viewport::new(96.0 / 72.0);
        v.resize(1000.0, 800.0, layout);
        v
    }

    #[test]
    fn the_side_arrows_turn_rows_by_the_reading_direction_or_scroll_sideways() {
        let facing = laid_out(Mode::Facing, Flow::Continuous, 10);
        let mut shown = viewport(&facing);
        shown.fit(ZoomMode::FitPage, &facing);
        shown.step_side(Side::Right, 100.0, &facing);
        assert_eq!(shown.band(), 1);
        shown.step_side(Side::Left, 100.0, &facing);
        assert_eq!(shown.band(), 0);
        // At the first row, back goes nowhere.
        shown.step_side(Side::Left, 100.0, &facing);
        assert_eq!(shown.band(), 0);

        let manga = Layout::new(
            &[LETTER; 10],
            Spec {
                mode: Mode::Facing,
                direction: Direction::RightToLeft,
                ..Spec::default()
            },
        );
        let mut shown = viewport(&manga);
        shown.fit(ZoomMode::FitPage, &manga);
        shown.step_side(Side::Left, 100.0, &manga);
        assert_eq!(shown.band(), 1);

        // Zoomed in past the width, the arrows scroll sideways.
        let mut shown = viewport(&facing);
        shown.set_zoom(4.0, &facing);
        let x = shown.visible().x0;
        shown.step_side(Side::Right, 100.0, &facing);
        assert!(shown.visible().x0 > x);
        assert_eq!(shown.band(), 0);
    }

    #[test]
    fn the_current_page_is_the_one_showing_most() {
        let layout = letters(10);
        let mut shown = viewport(&layout);
        assert_eq!(shown.current_page(&layout), Some(0));
        shown.scroll_to_page(4, &layout);
        assert_eq!(shown.current_page(&layout), Some(4));
        // Most of page 4 scrolled away: page 5 shows more.
        shown.scroll_by(0.0, 700.0, &layout);
        assert_eq!(shown.current_page(&layout), Some(5));
        let empty = Layout::default();
        assert_eq!(viewport(&empty).current_page(&empty), None);
    }

    #[test]
    fn a_link_target_shows_at_the_top() {
        let layout = letters(10);
        let mut shown = viewport(&layout);
        let page = layout.page(6).expect("page 6");
        shown.scroll_to_point(6, page.x0 + 20.0, page.y0 + 300.0, &layout);
        assert_eq!(shown.visible().y0, page.y0 + 300.0 - GAP);
        assert_eq!(shown.current_page(&layout), Some(6));
        // Past the end, the view stops at the end.
        let last = layout.page(9).expect("page 9");
        shown.scroll_to_point(9, last.x0, last.y1, &layout);
        assert!(shown.visible().y1 <= last.y1 + GAP + 0.5);
    }

    #[test]
    fn points_map_to_the_screen_and_back() {
        let layout = letters(3);
        let mut view = Viewport::new(2.0);
        view.resize(800.0, 600.0, &layout);
        view.set_zoom(1.5, &layout);
        view.scroll_by(0.0, 1234.0, &layout);
        let (x, y) = view.to_layout_point(100.0, 250.0);
        let (sx, sy) = view.to_screen_point(x, y);
        assert!(
            (sx - 100.0).abs() < 1e-3 && (sy - 250.0).abs() < 1e-3,
            "{sx}, {sy}"
        );
    }

    proptest! {
        /// R.2's "Done when": zooming about the pointer keeps the point under it fixed.
        #[test]
        fn zooming_keeps_the_point_under_the_pointer(
            start in 2.0f32..8.0,
            factor in 0.8f32..1.25,
            at_x in 0.0f32..1000.0,
            at_y in 0.0f32..800.0,
        ) {
            let layout = letters(200);
            let mut v = viewport(&layout);
            v.set_zoom(start, &layout);
            let middle = (layout.width() * v.scale() - 1000.0) / 2.0;
            v.scroll_by(middle - v.visible().x0 * v.scale(), layout.height() * v.scale() / 2.0, &layout);
            let before = v.to_layout_point(at_x, at_y);
            v.zoom_about(factor, at_x, at_y, &layout);
            let after = v.to_layout_point(at_x, at_y);
            prop_assert!((before.0 - after.0).abs() < 1e-2 && (before.1 - after.1).abs() < 1e-2,
                "{:?} moved to {:?}", before, after);
            prop_assert_eq!(v.zoom_mode(), ZoomMode::Custom);
        }
    }

    #[test]
    fn a_narrow_layout_is_centred_and_scrolling_stops_at_the_ends() {
        let layout = letters(3);
        let mut v = viewport(&layout);
        let page = v.to_screen(&layout.page(0).expect("page 0"));
        assert!(
            ((page.x0) - (1000.0 - page.x1)).abs() <= 1.0,
            "centred: {page:?}"
        );
        v.scroll_by(0.0, -500.0, &layout);
        assert_eq!(v.visible().y0, 0.0);
        v.scroll_by(0.0, 1e9, &layout);
        assert!((v.visible().y1 - layout.height()).abs() < 1e-2);
    }

    #[test]
    fn zoom_steps_move_to_the_next_level() {
        assert_eq!(step_in(1.0), 1.25);
        assert_eq!(step_in(1.1), 1.25, "between steps: the next one up");
        assert_eq!(step_out(1.1), 1.0);
        assert_eq!(step_out(1.0), 0.75);
        assert_eq!(step_in(64.0), MAX_ZOOM);
        assert_eq!(step_out(0.0833), 0.0833);
    }

    #[test]
    fn fit_width_fills_the_width_and_zoom_has_limits() {
        let layout = letters(2);
        let mut v = viewport(&layout);
        v.fit_width(&layout);
        let whole = v.to_screen(&crate::Rect::new(0.0, 0.0, layout.width(), 1.0));
        assert!((whole.width() - 1000.0).abs() <= 1.0);
        let page = layout.page(0).expect("page 0");
        assert!((v.to_screen(&page).x0 - GAP * v.scale()).abs() <= 1.0);

        v.set_zoom(1000.0, &layout);
        assert_eq!(v.zoom(), MAX_ZOOM);
        assert_eq!(v.zoom_mode(), ZoomMode::Custom);
        v.set_zoom(0.0001, &layout);
        assert_eq!(v.zoom(), MIN_ZOOM);
    }

    #[test]
    fn fit_modes_stay_on_through_a_resize() {
        let layout = letters(4);
        let mut v = viewport(&layout);
        v.fit(ZoomMode::FitPage, &layout);
        let band = layout.bands()[0].len() + 2.0 * GAP;
        assert!(
            (band * v.scale() - 800.0).abs() < 0.5,
            "the page fills the height"
        );
        v.resize(1000.0, 400.0, &layout);
        assert!(
            (band * v.scale() - 400.0).abs() < 0.5,
            "and still does after a resize"
        );
        v.fit(ZoomMode::FitHeight, &layout);
        assert_eq!(v.zoom_mode(), ZoomMode::FitHeight);
    }

    #[test]
    fn a_paged_view_turns_the_page_at_the_end() {
        let layout = laid_out(Mode::Single, Flow::Paged, 3);
        let mut v = viewport(&layout);
        v.set_zoom(2.0, &layout);
        assert_eq!(v.band(), 0);
        // Scroll to the end of page 1: still on it.
        v.scroll_by(0.0, 1e6, &layout);
        assert_eq!(v.band(), 0);
        assert!(v.visible().y1 <= layout.bands()[0].end + GAP + 0.01);
        // One more scroll turns to page 2, from its top.
        v.scroll_by(0.0, 10.0, &layout);
        assert_eq!(v.band(), 1);
        assert!((v.visible().y0 - (layout.bands()[1].start - GAP)).abs() < 0.01);
        // And back.
        v.scroll_by(0.0, -10.0, &layout);
        assert_eq!(v.band(), 0);
    }

    #[test]
    fn scrolling_forward_follows_the_strip() {
        let ltr = laid_out(Mode::Horizontal, Flow::Continuous, 5);
        let mut v = viewport(&ltr);
        v.set_zoom(2.0, &ltr);
        let x = v.visible().x0;
        v.scroll_forward(100.0, &ltr);
        assert!(v.visible().x0 > x, "rightwards");

        let spec = Spec {
            mode: Mode::Horizontal,
            direction: Direction::RightToLeft,
            ..Spec::default()
        };
        let rtl = Layout::new(&[LETTER; 5], spec);
        let mut v = viewport(&rtl);
        v.set_zoom(2.0, &rtl);
        v.scroll_to_page(0, &rtl);
        let x = v.visible().x0;
        v.scroll_forward(100.0, &rtl);
        assert!(v.visible().x0 < x, "leftwards");
    }

    /// R.1's "Done when": switching layouts keeps the same text at the top of the view.
    #[test]
    fn a_new_layout_keeps_the_reading_position() {
        let single = letters(20);
        let mut v = viewport(&single);
        v.set_zoom(1.5, &single);
        v.scroll_by(0.0, 9000.0, &single);
        let anchor = v.anchor(&single).expect("a page on screen");
        let page = single.page(anchor.page).expect("page");
        let offset = v.visible().y0 - page.y0;

        let book = laid_out(Mode::Book, Flow::Continuous, 20);
        v.relayout(Some(anchor), &book);
        let moved = book.page(anchor.page).expect("page");
        assert!(
            (v.visible().y0 - moved.y0 - offset).abs() < 0.01,
            "the same line of page {} is at the top",
            anchor.page
        );
    }
}
