//! Rectangular block selection and block highlighting (SumatraPDF Enhanced parity):
//! plain left-drag while Block Select mode (`blockSelect` prop) is active, or `Ctrl+left-drag`
//! anytime, drags a rectangular marquee across one or more pages, clipped to each intersected
//! page's bounds. Finalized block regions can be highlighted (preserving exact rectangular quads
//! over text, figures, formulas, or scanned pages), commented on, or copied.

use ecclesia_core::transform::Rotation;
use ecclesia_mupdf::{PagePoint, PageRect};
use ecclesia_text::{PageText, TextPos};
use nsh_core::JsonValue;

use super::PageCanvas;
use crate::Rect;
use crate::layout::Layout;
use crate::page::host::{Host, Wash};
use crate::page::placement::Placement;

/// Minimum pointer travel in client pixels before a press becomes a rectangular block drag
/// rather than a plain click.
const DRAG_THRESHOLD_PX: f32 = 4.0;
/// Selection fill and outline tint matching SumatraPDF Enhanced's crisp blue rectangular box.
const BLOCK_COLOR: u32 = 0x2f6fde;
/// Crisp rectangular border thickness in logical 96-dpi pixels.
const BORDER_THICKNESS: f32 = 1.5;
/// Characters of surrounding context kept on each side when a block selection spans text.
const QUOTE_CONTEXT: usize = 32;

/// One page's clipped rectangular selection region, in page points (`x0 <= x1`, `y0 <= y1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct BlockRegion {
    pub(super) page: usize,
    pub(super) rect: PageRect,
}

impl BlockRegion {
    /// Converts this rectangular region into a PDF markup quad `[ulx, uly, urx, ury, llx, lly, lrx, lry]`.
    pub(super) const fn to_quad_array(self) -> [f32; 8] {
        let r = self.rect;
        [r.x0, r.y0, r.x1, r.y0, r.x0, r.y1, r.x1, r.y1]
    }
}

/// Active rectangular block drag in client pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BlockDrag {
    start_client: (f32, f32),
    end_client: (f32, f32),
    moved: bool,
}

/// Rectangular block selection mode, active drag, and finalized page regions.
#[derive(Debug, Default)]
pub(super) struct BlockSelectState {
    /// Controlled by the `blockSelect` prop: when `true`, plain left-drag selects rectangular blocks.
    mode: bool,
    dragging: Option<BlockDrag>,
    regions: Vec<BlockRegion>,
}

impl<H: Host> PageCanvas<H> {
    /// Enables or disables Block Select mode (the `blockSelect` prop).
    pub(crate) fn set_block_select(&mut self, enabled: bool) {
        if self.block_select.mode != enabled {
            self.block_select.mode = enabled;
            if !enabled && self.block_select.dragging.take().is_some() {
                self.block_select.regions.clear();
            }
            self.invalidate();
        }
    }

    /// Exits Block Select mode and clears any active block selection (called on Escape / `leaveModes`).
    pub(crate) fn leave_block_select_mode(&mut self) {
        self.clear_block_selection();
        if self.block_select.mode {
            self.block_select.mode = false;
            self.emit("blockSelectCleared", JsonValue::Null);
            self.invalidate();
        }
    }

    /// Whether Block Select mode is active or a rectangular block drag is currently under way.
    pub(super) fn is_block_selecting(&self) -> bool {
        self.block_select.mode || self.block_select.dragging.is_some()
    }

    /// Whether finalized non-empty block regions are currently selected (and no drag is in progress).
    pub(super) fn has_block_selection(&self) -> bool {
        self.block_select.dragging.is_none() && !self.block_select.regions.is_empty()
    }

    /// Finalized rectangular block regions in page order.
    pub(super) fn block_regions(&self) -> &[BlockRegion] {
        &self.block_select.regions
    }

    /// Clears any active or finalized block selection and hides the popup if it was showing.
    pub(super) fn clear_block_selection(&mut self) -> bool {
        let had =
            self.block_select.dragging.take().is_some() || !self.block_select.regions.is_empty();
        self.block_select.regions.clear();
        if had {
            self.clear_external_popup();
            self.invalidate();
        }
        had
    }

    /// Starts a rectangular block drag when Block Select mode is on, or when `Ctrl` is held on a
    /// single click, provided no incompatible tool (drawing, lasso, snapshot, reflow) is active.
    pub(super) fn block_select_down(&mut self, x: i32, y: i32, clicks: u32) -> bool {
        if self.document.is_none()
            || self.bounds.is_empty()
            || self.is_drawing_tool()
            || self.lasso_on()
            || self.reflow.is_some()
        {
            return false;
        }
        let trigger = self.block_select.mode || (clicks == 1 && self.host.ctrl_down());
        if !trigger {
            return false;
        }
        let had_text = self.selection.take().is_some();
        self.selecting = false;
        self.markbar.caret = None;
        self.on_page = false;
        let had_block = !self.block_select.regions.is_empty();
        self.block_select.regions.clear();
        self.clear_external_popup();
        let start = (x as f32, y as f32);
        self.block_select.dragging = Some(BlockDrag {
            start_client: start,
            end_client: start,
            moved: false,
        });
        if had_text {
            self.host.selection_changed();
        }
        if had_text || had_block {
            self.invalidate();
        }
        true
    }

    /// Updates a live rectangular block drag as the pointer moves.
    pub(super) fn block_select_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        let Some(mut drag) = self.block_select.dragging else {
            return false;
        };
        if let Some((x, y)) = at {
            drag.end_client = (x as f32, y as f32);
            drag.moved |= (drag.end_client.0 - drag.start_client.0)
                .hypot(drag.end_client.1 - drag.start_client.1)
                >= DRAG_THRESHOLD_PX;
            self.block_select.dragging = Some(drag);
            if drag.moved {
                self.block_select.regions =
                    self.compute_block_regions(drag.start_client, drag.end_client);
                self.invalidate();
            }
        }
        true
    }

    /// Finalizes a rectangular block drag on mouse-up. Tiny clicks clear the block selection and
    /// fall through cleanly as a page press; non-empty drags keep their regions and anchor the
    /// floating selection popup.
    pub(super) fn block_select_up(&mut self, x: i32, y: i32) -> bool {
        let Some(mut drag) = self.block_select.dragging.take() else {
            return false;
        };
        drag.end_client = (x as f32, y as f32);
        drag.moved |= (drag.end_client.0 - drag.start_client.0)
            .hypot(drag.end_client.1 - drag.start_client.1)
            >= DRAG_THRESHOLD_PX;
        if !drag.moved {
            self.block_select.regions.clear();
            self.clear_external_popup();
            self.invalidate();
            self.page_press(x, y);
            return true;
        }
        let regions = self.compute_block_regions(drag.start_client, drag.end_client);
        if regions.is_empty() {
            self.block_select.regions.clear();
            self.clear_external_popup();
            self.invalidate();
            return true;
        }
        self.block_select.regions = regions;
        self.invalidate();
        if self.external_markbar_enabled() {
            self.update_external_popup();
        }
        true
    }

    /// Cancels an in-progress block drag if window mouse capture is lost.
    pub(super) fn block_select_cancel(&mut self) {
        if self.block_select.dragging.take().is_some() {
            self.block_select.regions.clear();
            self.clear_external_popup();
            self.invalidate();
        }
    }

    /// Computes clipped per-page regions intersected by the client-pixel rectangle `(start, end)`.
    fn compute_block_regions(&self, start: (f32, f32), end: (f32, f32)) -> Vec<BlockRegion> {
        let (lx0, ly0) = self.viewport.to_layout_point(start.0, start.1);
        let (lx1, ly1) = self.viewport.to_layout_point(end.0, end.1);
        let drag_layout = Rect {
            x0: lx0.min(lx1),
            y0: ly0.min(ly1),
            x1: lx0.max(lx1),
            y1: ly0.max(ly1),
        };
        compute_regions_in_layout(&self.layout, &self.bounds, self.view.rotation, drag_layout)
    }

    /// Draws the translucent selection tint and crisp 1.5-DIP blue border for every visible block region.
    pub(super) fn draw_block_selection(&mut self) -> Result<(), H::Error> {
        if self.block_select.regions.is_empty() {
            return Ok(());
        }
        let screen = self.screen();
        let border = (BORDER_THICKNESS * self.dpi_scale()).max(1.0);
        let rects: Vec<Rect> = self
            .block_select
            .regions
            .iter()
            .filter_map(|region| {
                let box_px = self.screen_box(region.page, &region.rect)?;
                box_px.intersect(&screen)?;
                Some(box_px)
            })
            .collect();
        for r in rects {
            let quad = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)];
            self.host
                .highlight(&[quad], BLOCK_COLOR, Wash::Translucent)?;
            let w = r.width().max(0.0);
            let h = r.height().max(0.0);
            self.host
                .fill(&Rect::new(r.x0, r.y0, w, border), BLOCK_COLOR)?;
            self.host.fill(
                &Rect::new(r.x0, (r.y1 - border).max(r.y0), w, border),
                BLOCK_COLOR,
            )?;
            self.host
                .fill(&Rect::new(r.x0, r.y0, border, h), BLOCK_COLOR)?;
            self.host.fill(
                &Rect::new((r.x1 - border).max(r.x0), r.y0, border, h),
                BLOCK_COLOR,
            )?;
        }
        Ok(())
    }

    /// Screen-pixel anchor `(center_x, top_y, bottom_y)` for positioning the floating selection
    /// popup above the topmost visible block region.
    pub(super) fn block_selection_screen_bounds(&self) -> Option<(f32, f32, f32)> {
        if !self.has_block_selection() {
            return None;
        }
        let mut topmost: Option<Rect> = None;
        let mut bottom_y = f32::NEG_INFINITY;
        for region in &self.block_select.regions {
            let Some(rect) = self.screen_box(region.page, &region.rect) else {
                continue;
            };
            if topmost.is_none_or(|best| rect.y0 < best.y0) {
                topmost = Some(rect);
            }
            bottom_y = bottom_y.max(rect.y1);
        }
        let top_rect = topmost?;
        let center_x = (top_rect.x0 + top_rect.x1) / 2.0;
        Some((center_x, top_rect.y0, bottom_y))
    }

    /// Extracts all text intersecting every finalized block region in reading order.
    pub(super) fn block_selection_text(&self) -> String {
        let mut parts = Vec::new();
        for region in &self.block_select.regions {
            let Some(text) = self.texts.get(region.page) else {
                continue;
            };
            let extracted = extract_text_in_rect(text, region.rect);
            if !extracted.trim().is_empty() {
                parts.push(extracted);
            }
        }
        parts.join("\n\n")
    }

    /// Builds an optional quote JSON object for a single [`BlockRegion`] if text intersects it,
    /// or `JsonValue::Null` over whitespace, figures, formulas, or scanned pages.
    pub(super) fn block_region_quote(&self, region: BlockRegion) -> JsonValue {
        let Some(text) = self.texts.get(region.page) else {
            return JsonValue::Null;
        };
        let Some((exact, first_idx, last_idx)) = extract_text_and_span_in_rect(text, region.rect)
        else {
            return JsonValue::Null;
        };
        if exact.trim().is_empty() {
            return JsonValue::Null;
        }
        let flat = self.flat_text();
        serde_json::json!({
            "prefix": text.text(first_idx.saturating_sub(QUOTE_CONTEXT)..first_idx),
            "exact": exact,
            "suffix": text.text(last_idx..last_idx.saturating_add(QUOTE_CONTEXT).min(text.len())),
            "start": flat.offset(TextPos { page: region.page, index: first_idx }),
        })
    }
}

/// Clips the axis-aligned page-point rectangle spanned by `p0` and `p1` to `bounds`.
/// Returns `None` when the clipped rectangle has zero width or height.
pub(super) fn clip_rect_to_bounds(
    p0: PagePoint,
    p1: PagePoint,
    bounds: PageRect,
) -> Option<PageRect> {
    let x0 = p0.x.min(p1.x).max(bounds.x0);
    let y0 = p0.y.min(p1.y).max(bounds.y0);
    let x1 = p0.x.max(p1.x).min(bounds.x1);
    let y1 = p0.y.max(p1.y).min(bounds.y1);
    (x1 > x0 && y1 > y0).then_some(PageRect { x0, y0, x1, y1 })
}

/// Intersects `drag_layout` with every page in `layout`, returning clipped page-point regions
/// sorted by ascending page index.
fn compute_regions_in_layout(
    layout: &Layout,
    bounds: &[PageRect],
    rotation: Rotation,
    drag_layout: Rect,
) -> Vec<BlockRegion> {
    if drag_layout.width() <= 0.0 || drag_layout.height() <= 0.0 {
        return Vec::new();
    }
    let mut regions = Vec::new();
    for (page, page_rect) in layout.pages_in(&drag_layout) {
        let Some(hit) = drag_layout.intersect(&page_rect) else {
            continue;
        };
        let (Some(&page_bounds), Some(placement)) = (
            bounds.get(page),
            Placement::of_page(layout, bounds, rotation, page),
        ) else {
            continue;
        };
        let p0 = placement.to_page(hit.x0, hit.y0);
        let p1 = placement.to_page(hit.x1, hit.y1);
        if let Some(rect) = clip_rect_to_bounds(p0, p1, page_bounds) {
            regions.push(BlockRegion { page, rect });
        }
    }
    regions.sort_by_key(|region| region.page);
    regions
}

/// Returns whether two page-point rectangles have a non-empty area overlap.
fn rects_intersect(a: PageRect, b: PageRect) -> bool {
    a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0
}

/// Extracts characters from `text` whose glyph boxes intersect `rect` in reading order, preserving
/// line and paragraph breaks.
pub(super) fn extract_text_in_rect(text: &PageText, rect: PageRect) -> String {
    extract_text_and_span_in_rect(text, rect)
        .map(|(extracted, _, _)| extracted)
        .unwrap_or_default()
}

/// Extracts intersecting characters in reading order along with the `(first_index, last_end)`
/// character index span for quote anchoring.
fn extract_text_and_span_in_rect(
    text: &PageText,
    rect: PageRect,
) -> Option<(String, usize, usize)> {
    let layout = text.layout();
    let mut out = String::new();
    let mut previous_block: Option<usize> = None;
    let mut first_idx: Option<usize> = None;
    let mut last_end = 0usize;

    for line in &layout.lines {
        let mut line_buf = String::new();
        for idx in line.chars.clone() {
            let ch = &layout.chars[idx];
            if rects_intersect(ch.quad.bounds(), rect) {
                if first_idx.is_none() {
                    first_idx = Some(idx);
                }
                last_end = idx + 1;
                line_buf.push(ch.c);
            }
        }
        if line_buf.is_empty() {
            continue;
        }
        if let Some(prev) = previous_block {
            out.push_str(if prev == line.block { "\n" } else { "\n\n" });
        }
        out.push_str(&line_buf);
        previous_block = Some(line.block);
    }

    first_idx.map(|first| (out, first, last_end))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::Path;
    use std::sync::Arc;

    use ecclesia_core::annot::Markup;
    use ecclesia_core::document::Document;
    use ecclesia_core::transform::Rotation;
    use ecclesia_mupdf::{
        PagePoint, PageRect, Pixmap, Quad, TextBlock, TextChar, TextLayout, TextLine,
    };
    use ecclesia_text::PageText;
    use nsh_core::JsonValue;

    use super::{clip_rect_to_bounds, compute_regions_in_layout, extract_text_in_rect};
    use crate::Rect;
    use crate::layout::{Layout, PageSize, Spec};
    use crate::page::canvas::{Cursor, NoteMode, PageCanvas};
    use crate::page::clicks::ClickLimits;
    use crate::page::host::{Host, InkPixel, Quad as ScreenQuad, Wake, Wash};

    const TEST_CLICK_LIMITS: ClickLimits = ClickLimits {
        time: 500,
        width: 4,
        height: 4,
    };

    struct MockHost {
        size: (f32, f32),
        ctrl: Cell<bool>,
        events: RefCell<Vec<(String, JsonValue)>>,
        highlights: RefCell<Vec<(Vec<ScreenQuad>, u32, Wash)>>,
        fills: RefCell<Vec<(Rect, u32)>>,
    }

    impl MockHost {
        fn new(width: f32, height: f32) -> Self {
            Self {
                size: (width, height),
                ctrl: Cell::new(false),
                events: RefCell::new(Vec::new()),
                highlights: RefCell::new(Vec::new()),
                fills: RefCell::new(Vec::new()),
            }
        }

        fn take_events(&self) -> Vec<(String, JsonValue)> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }

    impl Host for MockHost {
        type Image = ();
        type Error = &'static str;

        fn size(&self) -> (f32, f32) {
            self.size
        }

        fn invalidate(&self) {}

        fn emit(&self, name: &str, data: JsonValue) {
            self.events.borrow_mut().push((name.to_owned(), data));
        }

        fn waker(&self) -> Wake {
            Box::new(|| {})
        }

        fn upload(&mut self, _pixmap: &Pixmap) -> Option<(Self::Image, usize)> {
            None
        }

        fn begin(&mut self, _width: f32, _height: f32) -> Result<(), Self::Error> {
            Ok(())
        }

        fn end(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn clear(&mut self, _rgb: u32) {}

        fn fill(&mut self, rect: &Rect, rgb: u32) -> Result<(), Self::Error> {
            self.fills.borrow_mut().push((*rect, rgb));
            Ok(())
        }

        fn image(&mut self, _image: &Self::Image, _dest: &Rect) {}

        fn faint_image(&mut self, _image: &Self::Image, _dest: &Rect, _opacity: f32) {}

        fn highlight(
            &mut self,
            quads: &[ScreenQuad],
            rgb: u32,
            wash: Wash,
        ) -> Result<(), Self::Error> {
            self.highlights
                .borrow_mut()
                .push((quads.to_vec(), rgb, wash));
            Ok(())
        }

        fn band(
            &mut self,
            _rect: &Rect,
            _radius: f32,
            _rgb: u32,
            _wash: Wash,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn dim_outside(
            &mut self,
            _screen: &Rect,
            _hole: &Rect,
            _radius: f32,
            _rgb: u32,
            _alpha: f32,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn polyline(&mut self, _points: &[InkPixel], _rgb: u32) -> Result<(), Self::Error> {
            Ok(())
        }

        fn ctrl_down(&self) -> bool {
            self.ctrl.get()
        }
    }

    fn canvas_with_corpus_pdf() -> PageCanvas<MockHost> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
        let doc = Arc::new(Document::open(&path).expect("corpus pdf opens"));
        let bounds = vec![
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            },
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            },
        ];
        let mut canvas = PageCanvas::new(MockHost::new(800.0, 1000.0), 1.0);
        canvas.document = Some(doc);
        canvas.bounds = bounds;
        canvas.layout = Layout::new(
            &[
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
            ],
            Spec::default(),
        );
        canvas.viewport.resize(800.0, 1000.0, &canvas.layout);
        canvas.viewport.set_zoom(1.0, &canvas.layout);
        canvas.set_external_markbar(true);
        canvas
    }

    #[test]
    fn clips_reversed_and_out_of_bounds_rectangles_to_page_bounds() {
        let bounds = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        let clipped = clip_rect_to_bounds(
            PagePoint { x: 700.0, y: 200.0 },
            PagePoint { x: -20.0, y: 50.0 },
            bounds,
        )
        .expect("clipped rect");
        assert_eq!(
            clipped,
            PageRect {
                x0: 0.0,
                y0: 50.0,
                x1: 612.0,
                y1: 200.0,
            }
        );
        assert_eq!(
            clip_rect_to_bounds(
                PagePoint { x: -50.0, y: 10.0 },
                PagePoint { x: -10.0, y: 20.0 },
                bounds,
            ),
            None
        );
    }

    #[test]
    fn multi_page_drag_clips_each_intersected_page_and_generates_exact_quads() {
        let bounds = vec![
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            },
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            },
        ];
        let layout = Layout::new(
            &[
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
                PageSize {
                    width: 600.0,
                    height: 800.0,
                },
            ],
            Spec::default(),
        );
        let p0 = layout.page(0).expect("page 0");
        let p1 = layout.page(1).expect("page 1");
        let drag = Rect {
            x0: p0.x0 - 50.0,
            y0: p0.y1 - 100.0,
            x1: p0.x0 + 250.0,
            y1: p1.y0 + 120.0,
        };
        let regions = compute_regions_in_layout(&layout, &bounds, Rotation::Upright, drag);
        assert_eq!(regions.len(), 2, "drag across page gap spans both pages");
        assert_eq!(regions[0].page, 0);
        assert_eq!(
            regions[0].rect,
            PageRect {
                x0: 0.0,
                y0: 700.0,
                x1: 250.0,
                y1: 800.0,
            }
        );
        assert_eq!(regions[1].page, 1);
        assert_eq!(
            regions[1].rect,
            PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 250.0,
                y1: 120.0,
            }
        );
        assert_eq!(
            regions[0].to_quad_array(),
            [0.0, 700.0, 250.0, 700.0, 0.0, 800.0, 250.0, 800.0]
        );
    }

    #[test]
    fn tiny_click_is_rejected_and_does_not_create_block_selection() {
        let mut canvas = canvas_with_corpus_pdf();
        canvas.set_block_select(true);
        assert_eq!(canvas.cursor(), Cursor::Cross);

        let limits = TEST_CLICK_LIMITS;
        assert!(canvas.mouse_down(200, 200, 100, limits));
        canvas.mouse_moved(Some((202, 201)));
        assert!(canvas.mouse_up(202, 201));

        assert!(!canvas.has_block_selection());
        let events = canvas.host().take_events();
        assert!(
            !events
                .iter()
                .any(|(name, data)| name == "selectionPopup" && data["visible"] == true),
            "tiny click must not open block selection popup"
        );
    }

    #[test]
    fn block_drag_and_mark_with_color_emits_block_highlight_and_clears() {
        let mut canvas = canvas_with_corpus_pdf();
        canvas.host().ctrl.set(true);
        let page0 = canvas
            .viewport
            .to_screen(&canvas.layout.page(0).expect("page 0"));
        let sx0 = (page0.x0 + 40.0) as i32;
        let sy0 = (page0.y0 + 60.0) as i32;
        let sx1 = (page0.x0 + 240.0) as i32;
        let sy1 = (page0.y0 + 160.0) as i32;

        let limits = TEST_CLICK_LIMITS;
        assert!(canvas.mouse_down(sx0, sy0, 100, limits));
        assert_eq!(canvas.cursor(), Cursor::Cross);
        canvas.mouse_moved(Some((sx1, sy1)));
        assert!(canvas.mouse_up(sx1, sy1));

        assert!(canvas.has_block_selection());
        assert_eq!(canvas.block_regions().len(), 1);
        let region = canvas.block_regions()[0];
        assert_eq!(region.page, 0);
        assert!((region.rect.x0 - 40.0).abs() < 1.5);
        assert!((region.rect.y0 - 60.0).abs() < 1.5);
        assert!((region.rect.x1 - 240.0).abs() < 1.5);
        assert!((region.rect.y1 - 160.0).abs() < 1.5);

        let popup_events = canvas.host().take_events();
        let popup = popup_events
            .iter()
            .rev()
            .find(|(name, _)| name == "selectionPopup")
            .expect("selectionPopup emitted after block drag");
        assert_eq!(popup.1["visible"], true);
        assert_eq!(popup.1["block"], true);
        assert_eq!(popup.1["editable"], true);

        canvas
            .draw_block_selection()
            .expect("draw_block_selection succeeds");
        assert_eq!(
            canvas.host().highlights.borrow().len(),
            1,
            "one translucent block fill drawn"
        );
        assert_eq!(
            canvas.host().fills.borrow().len(),
            4,
            "four crisp rectangular border edges drawn"
        );

        canvas.mark_with_color(Markup::Highlight, Some(0xFF_CA28), NoteMode::Plain);
        assert!(!canvas.has_block_selection());

        let events = canvas.host().take_events();
        let markup = events
            .iter()
            .find(|(name, _)| name == "markup")
            .expect("markup event emitted for block highlight");
        assert_eq!(markup.1["kind"], "highlight");
        assert_eq!(markup.1["block"], true);
        assert_eq!(markup.1["note"], false);
        let pages = markup.1["pages"].as_array().expect("pages array");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["page"], 0);
        let quads = pages[0]["quads"].as_array().expect("quads array");
        assert_eq!(quads.len(), 1);
        let q = quads[0].as_array().expect("8-float quad");
        assert_eq!(q.len(), 8);
        assert!((q[0].as_f64().expect("ulx") - 40.0).abs() < 1.5);
        assert!((q[1].as_f64().expect("uly") - 60.0).abs() < 1.5);
        assert!((q[2].as_f64().expect("urx") - 240.0).abs() < 1.5);
        assert!((q[5].as_f64().expect("lly") - 160.0).abs() < 1.5);

        assert!(
            events
                .iter()
                .any(|(name, data)| name == "selectionPopup" && data["visible"] == false),
            "popup dismissed after block highlight"
        );
    }

    #[test]
    fn extracts_only_characters_whose_boxes_intersect_block_rect() {
        let mut layout = TextLayout::default();
        for (line_idx, text) in ["hello world", "second line"].iter().enumerate() {
            let y0 = line_idx as f32 * 20.0;
            let first = layout.chars.len();
            for (col, ch) in text.chars().enumerate() {
                let x0 = col as f32 * 10.0;
                layout.chars.push(TextChar {
                    c: ch,
                    quad: Quad {
                        ul: PagePoint { x: x0, y: y0 },
                        ur: PagePoint {
                            x: x0 + 10.0,
                            y: y0,
                        },
                        ll: PagePoint {
                            x: x0,
                            y: y0 + 12.0,
                        },
                        lr: PagePoint {
                            x: x0 + 10.0,
                            y: y0 + 12.0,
                        },
                    },
                    size: 12.0,
                });
            }
            layout.lines.push(TextLine {
                bbox: PageRect {
                    x0: 0.0,
                    y0,
                    x1: text.len() as f32 * 10.0,
                    y1: y0 + 12.0,
                },
                dir: PagePoint { x: 1.0, y: 0.0 },
                vertical: false,
                joined: false,
                block: 0,
                chars: first..layout.chars.len(),
            });
        }
        layout.blocks.push(TextBlock {
            bbox: PageRect {
                x0: 0.0,
                y0: 0.0,
                x1: 110.0,
                y1: 32.0,
            },
            lines: 0..2,
        });
        let page_text = PageText::new(layout);
        let extracted = extract_text_in_rect(
            &page_text,
            PageRect {
                x0: 55.0,
                y0: 0.0,
                x1: 115.0,
                y1: 32.0,
            },
        );
        assert_eq!(extracted, " world\nd line");
    }
}
