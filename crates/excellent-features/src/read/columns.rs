//! Two-column and multi-column auto-detection and column reading order (plan R.24).
//!
//! Analyzes K.13 [`TextLayout`] blocks and lines to separate full-width headers/titles/footers
//! from side-by-side columns, detects the vertical gutter split, reorders blocks into true
//! column-major reading order, and drives a [`ColumnScrollNavigator`] that scrolls down one
//! column before jumping to the top of the next.

use excellent_mupdf::{PagePoint, PageRect, TextLayout};

/// Minimum fraction of page width a vertical gutter must span to split two columns.
const MIN_GUTTER_WIDTH_FRAC: f32 = 0.015;

/// Minimum vertical overlap share to consider blocks side-by-side in separate columns.
const MIN_COLUMN_OVERLAP_FRAC: f32 = 0.20;

/// Detected column geometry of a page (plan R.24).
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnLayout {
    /// Single-column page (or insufficient text to split).
    Single {
        /// Content bounding box across all blocks.
        bounds: PageRect,
    },
    /// Two-column page with optional full-width header/footer bands.
    TwoColumn {
        /// Horizontal coordinate of the center gutter dividing left and right columns.
        gutter_x: f32,
        /// Bounding box enclosing the left column blocks.
        left_bounds: PageRect,
        /// Bounding box enclosing the right column blocks.
        right_bounds: PageRect,
        /// Optional full-width header/title region above the columns.
        full_width_top: Option<PageRect>,
        /// Optional full-width footer region below the columns.
        full_width_bottom: Option<PageRect>,
    },
}

impl ColumnLayout {
    /// Whether two columns were detected on the page.
    #[must_use]
    pub const fn is_two_column(&self) -> bool {
        matches!(self, Self::TwoColumn { .. })
    }
}

/// Role of a text block on a column-analyzed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockColumnRole {
    /// Full-width top header, title, or abstract spanning across the gutter.
    FullWidthTop,
    /// Left column body block.
    LeftColumn,
    /// Right column body block.
    RightColumn,
    /// Full-width bottom footer or spanning figure/table.
    FullWidthBottom,
}

/// Analysis result combining the detected [`ColumnLayout`] and block-level reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnAnalysis {
    /// Detected page column layout.
    pub layout: ColumnLayout,
    /// Block indices into `TextLayout::blocks` sorted in true column reading order.
    pub reading_order: Vec<usize>,
    /// Assigned column role for each block index (parallel to `TextLayout::blocks`).
    pub block_roles: Vec<BlockColumnRole>,
}

/// Detects whether `text` on a page of `page_bounds` uses a two-column layout and computes its
/// column-major reading order (plan R.24).
#[must_use]
pub fn detect_columns(text: &TextLayout, page_bounds: PageRect) -> ColumnAnalysis {
    let block_count = text.blocks.len();
    if block_count == 0 {
        return ColumnAnalysis {
            layout: ColumnLayout::Single {
                bounds: page_bounds,
            },
            reading_order: Vec::new(),
            block_roles: Vec::new(),
        };
    }

    let page_w = (page_bounds.x1 - page_bounds.x0).max(1.0);
    let page_h = (page_bounds.y1 - page_bounds.y0).max(1.0);
    let content_bounds = union_rects(text.blocks.iter().map(|b| b.bbox)).unwrap_or(page_bounds);

    // Search for a clean vertical gutter near the middle [35%..65%] of page width.
    // Use lines rather than whole blocks so slightly ragged blocks still register cleanly.
    let candidates: Vec<(usize, PageRect)> = text
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.bbox.width() > 1.0 && b.bbox.height() > 1.0)
        .map(|(i, b)| (i, b.bbox))
        .collect();

    if candidates.len() < 2 {
        return single_analysis(block_count, content_bounds);
    }

    // Evaluate candidate split coordinates in the central band using a fine grid.
    let band_left = page_bounds.x0 + page_w * 0.35;
    let band_right = page_bounds.x0 + page_w * 0.65;
    let steps = 60usize;
    let min_gutter = page_w * MIN_GUTTER_WIDTH_FRAC;

    let mut best_gutter: Option<(f32, usize, f32)> = None; // (x, score, clearance)
    for step in 0..=steps {
        let x = band_left + (band_right - band_left) * (step as f32 / steps as f32);
        let mut left_only = Vec::new();
        let mut right_only = Vec::new();
        let mut crossing_tall = 0usize;
        let mut min_clearance = f32::MAX;

        for &(_, r) in &candidates {
            if r.x1 <= x - min_gutter * 0.5 {
                left_only.push(r);
                min_clearance = min_clearance.min(x - r.x1);
            } else if r.x0 >= x + min_gutter * 0.5 {
                right_only.push(r);
                min_clearance = min_clearance.min(r.x0 - x);
            } else {
                // Full-width title/header/footer blocks are narrow vertically compared to column height.
                if r.height() > page_h * 0.35 {
                    crossing_tall += 1;
                }
            }
        }

        if crossing_tall > 0 || left_only.is_empty() || right_only.is_empty() {
            continue;
        }

        let left_box = union_rects(left_only.iter().copied());
        let right_box = union_rects(right_only.iter().copied());
        if let (Some(lb), Some(rb)) = (left_box, right_box) {
            let overlap_y = (lb.y1.min(rb.y1) - lb.y0.max(rb.y0)).max(0.0);
            let min_col_h = lb.height().min(rb.height()).max(1.0);
            if overlap_y / min_col_h < MIN_COLUMN_OVERLAP_FRAC {
                continue;
            }
            // Prefer balanced columns with strong horizontal clearance.
            let pair_count = left_only.len().min(right_only.len());
            let balance = left_only.len() + right_only.len();
            let score = pair_count * 10 + balance;
            let replace = match best_gutter {
                None => true,
                Some((_, best_score, best_clear)) => {
                    score > best_score || (score == best_score && min_clearance > best_clear)
                }
            };
            if replace {
                best_gutter = Some((x, score, min_clearance));
            }
        }
    }

    let Some((gutter_x, _, _)) = best_gutter else {
        return single_analysis(block_count, content_bounds);
    };

    let mut left_indices = Vec::new();
    let mut right_indices = Vec::new();
    let mut crossing_indices = Vec::new();

    for (idx, block) in text.blocks.iter().enumerate() {
        let r = block.bbox;
        if r.x1 <= gutter_x {
            left_indices.push(idx);
        } else if r.x0 >= gutter_x {
            right_indices.push(idx);
        } else {
            crossing_indices.push(idx);
        }
    }

    let Some(left_bounds) = union_rects(left_indices.iter().map(|&i| text.blocks[i].bbox)) else {
        return single_analysis(block_count, content_bounds);
    };
    let Some(right_bounds) = union_rects(right_indices.iter().map(|&i| text.blocks[i].bbox)) else {
        return single_analysis(block_count, content_bounds);
    };

    let cols_mid_y =
        0.5 * (left_bounds.y0.min(right_bounds.y0) + left_bounds.y1.max(right_bounds.y1));

    let mut top_indices = Vec::new();
    let mut bottom_indices = Vec::new();
    for idx in crossing_indices {
        let r = text.blocks[idx].bbox;
        let cy = 0.5 * (r.y0 + r.y1);
        if cy <= cols_mid_y {
            top_indices.push(idx);
        } else {
            bottom_indices.push(idx);
        }
    }

    let cmp_top = |&a: &usize, &b: &usize| {
        text.blocks[a]
            .bbox
            .y0
            .total_cmp(&text.blocks[b].bbox.y0)
            .then_with(|| text.blocks[a].bbox.x0.total_cmp(&text.blocks[b].bbox.x0))
    };
    top_indices.sort_by(cmp_top);
    left_indices.sort_by(cmp_top);
    right_indices.sort_by(cmp_top);
    bottom_indices.sort_by(cmp_top);

    let mut block_roles = vec![BlockColumnRole::LeftColumn; block_count];
    for &i in &top_indices {
        block_roles[i] = BlockColumnRole::FullWidthTop;
    }
    for &i in &left_indices {
        block_roles[i] = BlockColumnRole::LeftColumn;
    }
    for &i in &right_indices {
        block_roles[i] = BlockColumnRole::RightColumn;
    }
    for &i in &bottom_indices {
        block_roles[i] = BlockColumnRole::FullWidthBottom;
    }

    let mut reading_order = Vec::with_capacity(block_count);
    reading_order.extend_from_slice(&top_indices);
    reading_order.extend_from_slice(&left_indices);
    reading_order.extend_from_slice(&right_indices);
    reading_order.extend_from_slice(&bottom_indices);

    let full_width_top = union_rects(top_indices.iter().map(|&i| text.blocks[i].bbox));
    let full_width_bottom = union_rects(bottom_indices.iter().map(|&i| text.blocks[i].bbox));

    ColumnAnalysis {
        layout: ColumnLayout::TwoColumn {
            gutter_x,
            left_bounds,
            right_bounds,
            full_width_top,
            full_width_bottom,
        },
        reading_order,
        block_roles,
    }
}

fn single_analysis(block_count: usize, bounds: PageRect) -> ColumnAnalysis {
    ColumnAnalysis {
        layout: ColumnLayout::Single { bounds },
        reading_order: (0..block_count).collect(),
        block_roles: vec![BlockColumnRole::LeftColumn; block_count],
    }
}

fn union_rects(mut iter: impl Iterator<Item = PageRect>) -> Option<PageRect> {
    let mut acc = iter.next()?;
    for r in iter {
        acc.x0 = acc.x0.min(r.x0);
        acc.y0 = acc.y0.min(r.y0);
        acc.x1 = acc.x1.max(r.x1);
        acc.y1 = acc.y1.max(r.y1);
    }
    Some(acc)
}

/// Action produced by [`ColumnScrollNavigator`] when advancing in column reading mode (plan R.24).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnScrollTarget {
    /// Scroll vertically within the active column by `delta_y` page points.
    ScrollInColumn {
        /// Which column is currently active (`0` = left/single, `1` = right).
        column: usize,
        /// Target top-left viewport anchor in page points.
        anchor: PagePoint,
    },
    /// Jump from the bottom of column 1 to the top of column 2 on the same page.
    JumpToNextColumn {
        /// Target column index (`1`).
        column: usize,
        /// Top-left anchor of the next column in page points.
        anchor: PagePoint,
    },
    /// Finished the final column on the current page; caller should advance to the next page.
    AdvanceToNextPage,
}

/// Column-mode scroll navigator (plan R.24): steps down one column and jumps cleanly to the top
/// of the next column before advancing to the next page.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnScrollNavigator {
    layout: ColumnLayout,
    active_column: usize,
    scroll_y: f32,
    viewport_height_pt: f32,
}

impl ColumnScrollNavigator {
    /// Creates a navigator for `layout` with a viewport height of `viewport_height_pt` page points.
    #[must_use]
    pub fn new(layout: ColumnLayout, viewport_height_pt: f32) -> Self {
        let initial_y = match &layout {
            ColumnLayout::Single { bounds } => bounds.y0,
            ColumnLayout::TwoColumn {
                left_bounds,
                full_width_top,
                ..
            } => full_width_top.map_or(left_bounds.y0, |top| top.y0),
        };
        Self {
            layout,
            active_column: 0,
            scroll_y: initial_y,
            viewport_height_pt: viewport_height_pt.max(1.0),
        }
    }

    /// Zero-based active column (`0` = left/single, `1` = right).
    #[must_use]
    pub const fn active_column(&self) -> usize {
        self.active_column
    }

    /// Current vertical scroll coordinate in page points.
    #[must_use]
    pub const fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    /// Advances forward by `step_pt` page points down the current column, jumping to the top of
    /// column 2 when column 1 reaches its bottom.
    pub fn step_down(&mut self, step_pt: f32) -> ColumnScrollTarget {
        let step = step_pt.max(1.0);
        match &self.layout {
            ColumnLayout::Single { bounds } => {
                let next_y = self.scroll_y + step;
                if next_y >= bounds.y1 {
                    ColumnScrollTarget::AdvanceToNextPage
                } else {
                    self.scroll_y = next_y;
                    ColumnScrollTarget::ScrollInColumn {
                        column: 0,
                        anchor: PagePoint {
                            x: bounds.x0,
                            y: self.scroll_y,
                        },
                    }
                }
            }
            ColumnLayout::TwoColumn {
                left_bounds,
                right_bounds,
                ..
            } => {
                let col_bounds = if self.active_column == 0 {
                    left_bounds
                } else {
                    right_bounds
                };
                let next_y = self.scroll_y + step;
                let max_top = (col_bounds.y1 - self.viewport_height_pt * 0.25).max(col_bounds.y0);
                if next_y <= max_top {
                    self.scroll_y = next_y;
                    ColumnScrollTarget::ScrollInColumn {
                        column: self.active_column,
                        anchor: PagePoint {
                            x: col_bounds.x0,
                            y: self.scroll_y,
                        },
                    }
                } else if self.active_column == 0 {
                    self.active_column = 1;
                    self.scroll_y = right_bounds.y0;
                    ColumnScrollTarget::JumpToNextColumn {
                        column: 1,
                        anchor: PagePoint {
                            x: right_bounds.x0,
                            y: right_bounds.y0,
                        },
                    }
                } else {
                    ColumnScrollTarget::AdvanceToNextPage
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use excellent_mupdf::{PagePoint, TextBlock, TextLine};

    use super::*;

    fn block(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBlock {
        TextBlock {
            bbox: PageRect { x0, y0, x1, y1 },
            lines: 0..0,
        }
    }

    #[test]
    fn detects_two_columns_with_full_width_title_and_orders_left_then_right() {
        let page = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        // Interleaved block order as PDF content streams sometimes store rows left-to-right.
        let layout = TextLayout {
            blocks: vec![
                block(72.0, 60.0, 540.0, 95.0),    // 0: full-width title
                block(320.0, 130.0, 540.0, 300.0), // 1: right col top
                block(72.0, 130.0, 290.0, 300.0),  // 2: left col top
                block(320.0, 320.0, 540.0, 680.0), // 3: right col bottom
                block(72.0, 320.0, 290.0, 680.0),  // 4: left col bottom
            ],
            lines: Vec::<TextLine>::new(),
            chars: Vec::new(),
        };

        let analysis = detect_columns(&layout, page);
        assert!(analysis.layout.is_two_column());
        assert_eq!(analysis.reading_order, vec![0, 2, 4, 1, 3]);
        assert_eq!(analysis.block_roles[0], BlockColumnRole::FullWidthTop);
        assert_eq!(analysis.block_roles[2], BlockColumnRole::LeftColumn);
        assert_eq!(analysis.block_roles[1], BlockColumnRole::RightColumn);

        let mut nav = ColumnScrollNavigator::new(analysis.layout, 200.0);
        assert_eq!(nav.active_column(), 0);
        // Scroll past left column bottom triggers JumpToNextColumn to top of right column (x=320, y=130).
        let mut jumped = None;
        for _ in 0..10 {
            if let ColumnScrollTarget::JumpToNextColumn { column, anchor } = nav.step_down(150.0) {
                jumped = Some((column, anchor));
                break;
            }
        }
        assert_eq!(jumped, Some((1, PagePoint { x: 320.0, y: 130.0 })));
    }

    #[test]
    fn single_column_page_stays_single() {
        let page = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        };
        let layout = TextLayout {
            blocks: vec![
                block(72.0, 72.0, 540.0, 220.0),
                block(72.0, 240.0, 540.0, 480.0),
            ],
            lines: Vec::new(),
            chars: Vec::new(),
        };
        let analysis = detect_columns(&layout, page);
        assert!(!analysis.layout.is_two_column());
        assert_eq!(analysis.reading_order, vec![0, 1]);
    }
}
