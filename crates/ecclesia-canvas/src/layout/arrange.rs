//! Placing pages: rows for single, facing, book and grid layouts; one strip for the horizontal layout.
//!
//! Every page gets a slot as wide as the widest page (as tall as the tallest, in the strip), so a row of mixed sizes
//! still lines up. In two-page rows the pages meet at the spine: the left page sits against the right of its slot,
//! the right page against the left. Single pages and grid cells are centred in their slots.

use std::ops::Range;

use super::pages::{Band, GAP, Layout, PageSize};
use super::spec::{Direction, Mode, Spec};
use crate::Rect;

/// The pages of `sizes`, laid out as `spec` says.
pub(super) fn arrange(sizes: &[PageSize], spec: Spec) -> Layout {
    match spec.mode {
        Mode::Horizontal => strip(sizes, spec),
        _ => rows(sizes, spec),
    }
}

/// The pages of each row, in reading order.
fn row_ranges(count: usize, mode: Mode) -> Vec<Range<usize>> {
    let per_row = mode.per_row();
    // Book view: the cover is a row of its own.
    let first = match mode {
        Mode::Book => count.min(1),
        _ => 0,
    };
    let rest = (first..count)
        .step_by(per_row)
        .map(|start| start..(start + per_row).min(count));
    (first > 0)
        .then_some(0..first)
        .into_iter()
        .chain(rest)
        .collect()
}

/// Which slot of its row, counted in reading order, page `index` goes in.
fn slot_in_reading_order(mode: Mode, row: &Range<usize>, index: usize) -> usize {
    match mode {
        // The cover is a right-hand page in a left-to-right book: the second slot.
        Mode::Book if row.start == 0 => 1,
        _ => index - row.start,
    }
}

/// Rows of pages, top to bottom.
fn rows(sizes: &[PageSize], spec: Spec) -> Layout {
    let per_row = spec.mode.per_row();
    let slot = sizes.iter().map(|s| s.width).fold(0.0, f32::max);
    let width = per_row as f32 * slot + (per_row as f32 + 1.0) * GAP;
    let two_up = per_row == 2 && matches!(spec.mode, Mode::Facing | Mode::Book);

    let ranges = row_ranges(sizes.len(), spec.mode);
    let mut pages = vec![Rect::default(); sizes.len()];
    let mut bands = Vec::with_capacity(ranges.len());
    let mut y = GAP;
    for row in ranges {
        let height = sizes[row.clone()]
            .iter()
            .map(|s| s.height)
            .fold(0.0, f32::max);
        for index in row.clone() {
            let size = sizes[index];
            let reading = slot_in_reading_order(spec.mode, &row, index);
            let column = match spec.direction {
                Direction::LeftToRight => reading,
                Direction::RightToLeft => per_row - 1 - reading,
            };
            let slot_x = GAP + column as f32 * (slot + GAP);
            let x = if !two_up {
                slot_x + (slot - size.width) / 2.0
            } else if column == 0 {
                slot_x + slot - size.width
            } else {
                slot_x
            };
            pages[index] = Rect::new(x, y + (height - size.height) / 2.0, size.width, size.height);
        }
        bands.push(Band {
            start: y,
            end: y + height,
            pages: row,
        });
        y += height + GAP;
    }
    let height = if sizes.is_empty() { 0.0 } else { y };
    Layout::from_parts(spec, pages, bands, width, height)
}

/// One page after the other, sideways; right to left puts the first page at the right end.
fn strip(sizes: &[PageSize], spec: Spec) -> Layout {
    let tallest = sizes.iter().map(|s| s.height).fold(0.0, f32::max);
    let height = tallest + 2.0 * GAP;
    let order: Vec<usize> = match spec.direction {
        Direction::LeftToRight => (0..sizes.len()).collect(),
        Direction::RightToLeft => (0..sizes.len()).rev().collect(),
    };
    let mut pages = vec![Rect::default(); sizes.len()];
    let mut bands = Vec::with_capacity(sizes.len());
    let mut x = GAP;
    for index in order {
        let size = sizes[index];
        pages[index] = Rect::new(
            x,
            GAP + (tallest - size.height) / 2.0,
            size.width,
            size.height,
        );
        bands.push(Band {
            start: x,
            end: x + size.width,
            pages: index..index + 1,
        });
        x += size.width + GAP;
    }
    let width = if sizes.is_empty() { 2.0 * GAP } else { x };
    let height = if sizes.is_empty() { 0.0 } else { height };
    Layout::from_parts(spec, pages, bands, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Flow;

    const LETTER: PageSize = PageSize {
        width: 612.0,
        height: 792.0,
    };

    fn layout(mode: Mode, direction: Direction, count: usize) -> Layout {
        let spec = Spec {
            mode,
            flow: Flow::Continuous,
            direction,
        };
        Layout::new(&vec![LETTER; count], spec)
    }

    fn row_of(layout: &Layout, band: usize) -> Vec<usize> {
        layout.bands()[band].pages.clone().collect()
    }

    #[test]
    fn book_view_puts_the_cover_alone_on_the_right() {
        let book = layout(Mode::Book, Direction::LeftToRight, 5);
        assert_eq!(row_of(&book, 0), [0]);
        assert_eq!(row_of(&book, 1), [1, 2]);
        assert_eq!(row_of(&book, 2), [3, 4]);
        let cover = book.page(0).expect("cover");
        assert_eq!(cover.x0, GAP + 612.0 + GAP, "the right-hand slot");
        let (left, right) = (book.page(1).expect("2"), book.page(2).expect("3"));
        assert_eq!(left.x1 + GAP, right.x0, "the pages meet at the spine");
    }

    #[test]
    fn facing_view_pairs_from_the_first_page() {
        let facing = layout(Mode::Facing, Direction::LeftToRight, 3);
        assert_eq!(row_of(&facing, 0), [0, 1]);
        assert_eq!(row_of(&facing, 1), [2]);
        assert!(facing.page(0).expect("1").x0 < facing.page(1).expect("2").x0);
        assert_eq!(
            facing.page(2).expect("3").x0,
            GAP,
            "a last odd page is a left page"
        );
    }

    #[test]
    fn right_to_left_mirrors_the_rows_and_the_strip() {
        let manga = layout(Mode::Facing, Direction::RightToLeft, 2);
        assert!(manga.page(0).expect("1").x0 > manga.page(1).expect("2").x0);
        let book = layout(Mode::Book, Direction::RightToLeft, 3);
        assert_eq!(
            book.page(0).expect("cover").x0,
            GAP,
            "the cover is on the left"
        );

        let strip = layout(Mode::Horizontal, Direction::RightToLeft, 3);
        assert!(strip.page(0).expect("1").x0 > strip.page(2).expect("3").x0);
        assert_eq!(row_of(&strip, 0), [2], "bands run left to right in space");
    }

    #[test]
    fn a_grid_fills_its_columns() {
        let grid = layout(Mode::Grid { columns: 3 }, Direction::LeftToRight, 7);
        assert_eq!(grid.bands().len(), 3);
        assert_eq!(row_of(&grid, 2), [6]);
        assert_eq!(grid.width(), 3.0 * 612.0 + 4.0 * GAP);
    }
}
