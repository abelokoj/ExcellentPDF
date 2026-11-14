//! The reader's layout choices (plan R.1, R.3), as SumatraPDF Enhanced models them: single, facing or book pages,
//! each continuous or one screen at a time, plus a horizontal strip and a grid; and the reading direction.

/// How pages are put together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// One page across (SumatraPDF's single page view).
    #[default]
    Single,
    /// Two pages side by side: 1–2, 3–4, ... (facing view).
    Facing,
    /// Two pages side by side with the cover alone: 1, 2–3, 4–5, ..., as a printed book opens (book view).
    Book,
    /// One row of pages, scrolled sideways.
    Horizontal,
    /// Rows of `columns` pages (0 is taken as 1).
    Grid {
        /// Pages in a row.
        columns: u8,
    },
}

impl Mode {
    /// How many pages share a row.
    pub fn per_row(self) -> usize {
        match self {
            Self::Single | Self::Horizontal => 1,
            Self::Facing | Self::Book => 2,
            Self::Grid { columns } => usize::from(columns.max(1)),
        }
    }
}

/// Whether the view scrolls through the pages or shows one row of them at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Flow {
    /// Pages scroll by without a break (SumatraPDF's "continuous").
    #[default]
    Continuous,
    /// One row at a time: scrolling past its end turns to the next. Single, facing and book layouts only; the
    /// horizontal strip and the grid always scroll.
    Paged,
}

impl Flow {
    /// The other flow.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Continuous => Self::Paged,
            Self::Paged => Self::Continuous,
        }
    }
}

/// The order pages are read across a row (plan R.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    /// Left to right.
    #[default]
    LeftToRight,
    /// Right to left: Arabic, Hebrew, manga.
    RightToLeft,
}

impl Direction {
    /// The other direction.
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::LeftToRight => Self::RightToLeft,
            Self::RightToLeft => Self::LeftToRight,
        }
    }
}

/// Everything that decides a layout besides the page sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Spec {
    /// How pages are put together.
    pub mode: Mode,
    /// Continuous or one row at a time.
    pub flow: Flow,
    /// The reading direction.
    pub direction: Direction,
}

impl Spec {
    /// Whether the view shows one row at a time: [`Flow::Paged`] in a layout that has it.
    pub fn is_paged(&self) -> bool {
        self.flow == Flow::Paged && matches!(self.mode, Mode::Single | Mode::Facing | Mode::Book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rows_of_pages_can_be_paged() {
        let paged = |mode| {
            Spec {
                mode,
                flow: Flow::Paged,
                ..Spec::default()
            }
            .is_paged()
        };
        assert!(paged(Mode::Single) && paged(Mode::Facing) && paged(Mode::Book));
        assert!(!paged(Mode::Horizontal) && !paged(Mode::Grid { columns: 3 }));
        assert!(!Spec::default().is_paged());
    }

    #[test]
    fn a_grid_has_at_least_one_column() {
        assert_eq!(Mode::Grid { columns: 0 }.per_row(), 1);
        assert_eq!(Mode::Grid { columns: 5 }.per_row(), 5);
        assert_eq!(Mode::Book.per_row(), 2);
    }
}
