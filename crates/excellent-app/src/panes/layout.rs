//! The split view's layout (V.4), plain data: which cells of a two-by-two grid hold a pane, the active one (V.5),
//! and where the dividers were dragged to. It round-trips through JSON for the session (V.7).
//!
//! A pane keeps its cell for as long as it is open, so splitting and closing never move the others (their canvases
//! stay as they are). Two panes side by side are the top row's cells; stacked, the left column's; four, the grid.
//! Flipping (V.10) turns the grid over its diagonal; a maximized pane hides the others until the panes are put back.
//! Each of these is its own undo: doing it again puts the layout back as it was.

use serde_json::{Value, json};

use crate::tabs::Step;

/// The grid's cells, in reading order: top left, top right, bottom left, bottom right.
pub(crate) const CELLS: usize = 4;

/// Where a new pane goes beside the active one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Right,
    Down,
    Left,
    Up,
}

/// Where [`Layout::plan_split`] puts a new pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Split {
    /// The new pane's cell.
    pub(crate) cell: usize,
    /// A pane that moves first, to make room on the side asked for: its cell, and the one it moves to.
    pub(crate) moved: Option<(usize, usize)>,
}

/// The size every pane of a row or column has after [`Layout::equalize`]: panels that all have sizes share the
/// room in proportion to them.
const EQUAL: f32 = 1.0;

/// The panes' arrangement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    open: [bool; CELLS],
    active: usize,
    /// The rows' heights as the divider between them was last dragged to, in points; none share alike.
    rows: Option<[f32; 2]>,
    /// Each row's columns' widths, the same way.
    columns: [Option<[f32; 2]>; 2],
    /// The pane that fills the window while the others wait (V.10).
    maximized: Option<usize>,
}

impl Default for Layout {
    /// One pane.
    fn default() -> Self {
        Self {
            open: [true, false, false, false],
            active: 0,
            rows: None,
            columns: [None, None],
            maximized: None,
        }
    }
}

impl Layout {
    /// The cells that hold a shown pane, in reading order.
    pub(crate) fn panes(&self) -> impl Iterator<Item = usize> + '_ {
        (0..CELLS).filter(|&cell| self.is_open(cell))
    }

    /// How many panes there are.
    pub(crate) fn count(&self) -> usize {
        self.panes().count()
    }

    /// Whether the window is split.
    pub(crate) fn is_split(&self) -> bool {
        self.count() > 1
    }

    /// Whether `cell` holds a pane that shows: while one is maximized, only that one does.
    pub(crate) fn is_open(&self, cell: usize) -> bool {
        self.open.get(cell).copied().unwrap_or(false)
            && self.maximized.is_none_or(|maximized| maximized == cell)
    }

    /// Whether a pane is maximized (V.10).
    pub(crate) fn is_maximized(&self) -> bool {
        self.maximized.is_some()
    }

    /// Maximizes the active pane, or puts the panes back when one is (V.10); `false`, changing nothing, with one
    /// pane.
    pub(crate) fn toggle_maximized(&mut self) -> bool {
        if self.maximized.take().is_some() {
            return true;
        }
        if !self.is_split() {
            return false;
        }
        self.maximized = Some(self.active);
        true
    }

    /// Turns the grid over its diagonal (V.10): panes side by side become stacked and back, the top right and bottom
    /// left cells trading places with the rows' heights and the top row's widths. Gives the two cells, whose panes
    /// trade tabs. Flipping again puts it back.
    pub(crate) fn flip(&mut self) -> (usize, usize) {
        const TRADED: (usize, usize) = (1, 2);
        let across = |cell: usize| match cell {
            1 => 2,
            2 => 1,
            other => other,
        };
        self.open.swap(TRADED.0, TRADED.1);
        self.active = across(self.active);
        self.maximized = self.maximized.map(across);
        std::mem::swap(&mut self.rows, &mut self.columns[0]);
        TRADED
    }

    /// Whether the row `row` (0 the top) holds a pane.
    pub(crate) fn row_open(&self, row: usize) -> bool {
        self.is_open(row * 2) || self.is_open(row * 2 + 1)
    }

    /// The active pane's cell: the one commands act on.
    pub(crate) fn active(&self) -> usize {
        self.active
    }

    /// Makes the pane in `cell` the active one; `false` when there is none there.
    pub(crate) fn focus(&mut self, cell: usize) -> bool {
        let open = self.is_open(cell);
        if open {
            self.active = cell;
        }
        open
    }

    /// The cell of the `position`th pane in reading order, from 0 (Alt+1 is the first).
    pub(crate) fn at(&self, position: usize) -> Option<usize> {
        self.panes().nth(position)
    }

    /// The pane after or before the active one in reading order, round the end.
    pub(crate) fn step(&self, step: Step) -> usize {
        let panes: Vec<usize> = self.panes().collect();
        let at = panes
            .iter()
            .position(|&cell| cell == self.active)
            .unwrap_or(0);
        let next = match step {
            Step::Next => (at + 1) % panes.len(),
            Step::Previous => (at + panes.len() - 1) % panes.len(),
        };
        panes[next]
    }

    /// Where a new pane on `side` of the active one goes: the cell there when it is free; when the active pane is
    /// at that edge of the grid and the cell on its other side is free, the active pane moves there and the new one
    /// takes its cell; else the first free cell. `None` when the grid is full.
    pub(crate) fn plan_split(&self, side: Side) -> Option<Split> {
        let cell = self.active;
        // The partner across the side's axis, and whether the side points away from it.
        let (partner, toward) = match side {
            Side::Right => (cell ^ 1, cell.is_multiple_of(2)),
            Side::Left => (cell ^ 1, cell % 2 == 1),
            Side::Down => (cell ^ 2, cell < 2),
            Side::Up => (cell ^ 2, cell >= 2),
        };
        if !self.open[partner] {
            return Some(if toward {
                Split {
                    cell: partner,
                    moved: None,
                }
            } else {
                Split {
                    cell,
                    moved: Some((cell, partner)),
                }
            });
        }
        let free = (0..CELLS).find(|&cell| !self.open[cell])?;
        Some(Split {
            cell: free,
            moved: None,
        })
    }

    /// Opens the pane `split` planned (its pane already moved), active, and evens the panes out.
    pub(crate) fn apply(&mut self, split: Split) {
        if let Some((from, to)) = split.moved {
            self.open[from] = false;
            self.open[to] = true;
        }
        self.open[split.cell] = true;
        self.active = split.cell;
        self.maximized = None;
        self.equalize();
    }

    /// Closes the pane in `cell`, and gives the neighbour that takes its tabs: the pane beside it, else the one above
    /// or below, else the first. `None` for the last pane, which stays.
    pub(crate) fn close(&mut self, cell: usize) -> Option<usize> {
        // The panes come back first: the neighbour may be one the maximized pane hid.
        self.maximized = None;
        if !self.is_open(cell) || self.count() == 1 {
            return None;
        }
        self.open[cell] = false;
        let neighbour = [cell ^ 1, cell ^ 2]
            .into_iter()
            .find(|&other| self.open[other])
            .or_else(|| self.panes().next())?;
        if self.active == cell {
            self.active = neighbour;
        }
        Some(neighbour)
    }

    /// Makes every pane of a row or column the same size (a divider's double-click).
    pub(crate) fn equalize(&mut self) {
        self.rows = Some([EQUAL; 2]);
        self.columns = [Some([EQUAL; 2]); 2];
    }

    /// The height row `row` wants, while both rows are shown (a row alone takes the room).
    pub(crate) fn row_size(&self, row: usize) -> Option<f32> {
        let both = self.row_open(0) && self.row_open(1);
        self.rows.filter(|_| both).map(|rows| rows[row])
    }

    /// The width the pane in `cell` wants, while both of its row's cells are shown (a pane alone takes the row).
    pub(crate) fn column_size(&self, cell: usize) -> Option<f32> {
        let both = self.is_open(cell) && self.is_open(cell ^ 1);
        self.columns[cell / 2]
            .filter(|_| both)
            .map(|columns| columns[cell % 2])
    }

    /// Notes the rows' heights a divider drag left (both rows shown); `false` when they were so already.
    pub(crate) fn resize_rows(&mut self, sizes: &[f32]) -> bool {
        set_pair(&mut self.rows, sizes)
    }

    /// Notes the widths of row `row`'s cells a divider drag left (both shown); `false` when they were so already.
    pub(crate) fn resize_columns(&mut self, row: usize, sizes: &[f32]) -> bool {
        set_pair(&mut self.columns[row], sizes)
    }

    /// The layout as the session keeps it.
    pub(crate) fn to_json(&self) -> String {
        json!({
            "open": self.open,
            "active": self.active,
            "rows": self.rows,
            "columns": self.columns,
            "maximized": self.maximized,
        })
        .to_string()
    }

    /// A layout the session kept; one pane when it is broken or isn't one.
    pub(crate) fn from_json(text: &str) -> Self {
        Self::parse(text).unwrap_or_default()
    }

    fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let open: Vec<bool> = value
            .get("open")?
            .as_array()?
            .iter()
            .map(Value::as_bool)
            .collect::<Option<_>>()?;
        let open: [bool; CELLS] = open.try_into().ok()?;
        if !open.contains(&true) {
            return None;
        }
        let active = value
            .get("active")
            .and_then(Value::as_u64)
            .and_then(|active| usize::try_from(active).ok())
            .filter(|&active| active < CELLS && open[active])
            .or_else(|| open.iter().position(|&open| open))?;
        let columns = value.get("columns").and_then(Value::as_array);
        Some(Self {
            open,
            active,
            rows: value.get("rows").and_then(pair),
            columns: [0, 1].map(|row| columns.and_then(|columns| pair(columns.get(row)?))),
            maximized: value
                .get("maximized")
                .and_then(Value::as_u64)
                .and_then(|cell| usize::try_from(cell).ok())
                .filter(|&cell| {
                    cell < CELLS && open[cell] && open.iter().filter(|&&o| o).count() > 1
                }),
        })
    }
}

/// Two sizes kept in JSON.
fn pair(value: &Value) -> Option<[f32; 2]> {
    let sizes = value.as_array()?;
    let size = |at: usize| {
        let size = sizes.get(at)?.as_f64()? as f32;
        (size.is_finite() && size > 0.0).then_some(size)
    };
    Some([size(0)?, size(1)?])
}

/// Sets `slot` to the two `sizes`; `false` when they aren't two, or are what it holds.
fn set_pair(slot: &mut Option<[f32; 2]>, sizes: &[f32]) -> bool {
    let Ok(sizes) = <[f32; 2]>::try_from(sizes) else {
        return false;
    };
    if *slot == Some(sizes) {
        return false;
    }
    *slot = Some(sizes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(layout: &mut Layout, side: Side) -> Split {
        let split = layout.plan_split(side).expect("room for a pane");
        layout.apply(split);
        split
    }

    #[test]
    fn splitting_right_then_down_fills_the_grid() {
        let mut layout = Layout::default();
        assert_eq!(split(&mut layout, Side::Right).cell, 1);
        assert_eq!(
            (layout.panes().collect::<Vec<_>>(), layout.active()),
            (vec![0, 1], 1)
        );
        assert_eq!(split(&mut layout, Side::Down).cell, 3);
        layout.focus(0);
        assert_eq!(split(&mut layout, Side::Down).cell, 2);
        assert_eq!(layout.count(), 4);
        assert_eq!(layout.plan_split(Side::Right), None, "four panes at most");
    }

    #[test]
    fn a_pane_at_the_edge_moves_over_for_a_new_one_on_that_side() {
        let mut layout = Layout::default();
        // A pane on the left of the first: the first moves right.
        let left = split(&mut layout, Side::Left);
        assert_eq!(
            left,
            Split {
                cell: 0,
                moved: Some((0, 1))
            }
        );
        assert_eq!(layout.panes().collect::<Vec<_>>(), [0, 1]);
        // Right of the left pane is taken: the first free cell.
        layout.focus(0);
        assert_eq!(split(&mut layout, Side::Right).cell, 2);
    }

    #[test]
    fn closing_a_pane_hands_its_tabs_to_the_one_beside_it() {
        let mut layout = Layout::default();
        split(&mut layout, Side::Right);
        split(&mut layout, Side::Down);
        // 0 and 1 on top, 3 under 1; 3 is active.
        assert_eq!(layout.close(3), Some(1), "above it, nothing beside it");
        assert_eq!(layout.active(), 1);
        assert_eq!(layout.close(0), Some(1), "beside it");
        assert_eq!(layout.close(1), None, "the last pane stays");
        assert_eq!(layout.count(), 1);
    }

    #[test]
    fn panes_are_reached_in_reading_order_round_the_end() {
        let mut layout = Layout::default();
        split(&mut layout, Side::Down);
        split(&mut layout, Side::Right);
        // 0 above 2, 3 right of 2.
        assert_eq!(layout.panes().collect::<Vec<_>>(), [0, 2, 3]);
        assert_eq!(layout.at(1), Some(2));
        assert_eq!(layout.at(3), None);
        assert_eq!(layout.step(Step::Next), 0);
        assert_eq!(layout.step(Step::Previous), 2);
        assert!(!layout.focus(1), "no pane there");
        assert!(layout.row_open(1) && !layout.is_open(1));
    }

    #[test]
    fn a_layout_round_trips_through_json() {
        let mut layout = Layout::default();
        split(&mut layout, Side::Right);
        split(&mut layout, Side::Down);
        assert!(layout.resize_columns(0, &[420.0, 610.5]));
        assert!(!layout.resize_columns(0, &[420.0, 610.5]), "unchanged");
        assert!(!layout.resize_rows(&[300.0]), "one row shown: no divider");
        assert_eq!(Layout::from_json(&layout.to_json()), layout);
        // Broken or foreign: one pane.
        assert_eq!(Layout::from_json("{}"), Layout::default());
        assert_eq!(
            Layout::from_json(r#"{"open":[false,false,false,false],"active":0}"#),
            Layout::default()
        );
        let lost = Layout::from_json(r#"{"open":[false,true,false,false],"active":3}"#);
        assert_eq!(lost.active(), 1, "the active pane must be open");
    }

    #[test]
    fn swap_flip_and_maximize_each_undo_themselves() {
        let mut layout = Layout::default();
        split(&mut layout, Side::Right);
        layout.resize_columns(0, &[300.0, 700.0]);
        let before = layout.clone();

        // Flip: side by side becomes stacked, and back.
        assert_eq!(layout.flip(), (1, 2));
        assert_eq!(layout.panes().collect::<Vec<_>>(), [0, 2]);
        assert_eq!(layout.active(), 2, "the active pane goes with its cell");
        assert_eq!(layout.row_size(0), Some(300.0), "the widths become heights");
        layout.flip();
        assert_eq!(layout, before);

        // Maximize: only the active pane shows; nothing else changes, and putting back restores it all.
        assert!(layout.toggle_maximized());
        assert_eq!(layout.panes().collect::<Vec<_>>(), [1]);
        assert!(!layout.is_split() && !layout.is_open(0) && layout.is_maximized());
        assert_eq!(
            layout.plan_split(Side::Left).map(|s| s.cell),
            Some(2),
            "hidden panes keep their cells"
        );
        assert_eq!(
            Layout::from_json(&layout.to_json()),
            layout,
            "the session keeps it"
        );
        assert!(layout.toggle_maximized());
        assert_eq!(layout, before);
        assert!(
            !Layout::default().toggle_maximized(),
            "one pane has nothing to hide"
        );

        // Closing while maximized brings the panes back first.
        layout.toggle_maximized();
        assert_eq!(layout.close(1), Some(0));
        assert!(!layout.is_maximized());
    }

    #[test]
    fn evening_out_gives_every_pane_the_same_share() {
        let mut layout = Layout::default();
        split(&mut layout, Side::Right);
        layout.resize_columns(0, &[200.0, 800.0]);
        layout.equalize();
        assert_eq!(layout.column_size(0), Some(EQUAL));
        assert_eq!(layout.column_size(1), Some(EQUAL));
        assert_eq!(layout.row_size(0), None, "one row: it takes the room");
    }
}
