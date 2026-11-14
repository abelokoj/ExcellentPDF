//! One view of a document (plan K.5, item 2): how the reader has its pages shown (the layout, the view rotation and
//! the page box: R.1–R.4), the commands that change that, and where the reader has been ([`history`], R.8). Each
//! canvas keeps one; the live scroll and zoom are the canvas's `Viewport`, from which a [`history::Place`] is taken.
//!
//! shortcut: not serialized yet; R.9 (sessions) chooses the format and saves it with the viewport's place.

pub mod history;

use excellent_canvas::layout::{Mode, Spec};
use excellent_canvas::viewport::ZoomMode;
use excellent_mupdf::PageBox;

use crate::transform::Rotation;
use history::History;

/// Device pixels a grid cell gets at least, so a grid page stays recognisable.
const GRID_CELL: f32 = 240.0;
/// The most columns a grid has.
const GRID_MAX_COLUMNS: u8 = 8;

/// The reader's choices for one view, and its history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    /// The layout, flow and reading direction.
    pub spec: Spec,
    /// The view rotation (not saved to the file; "Rotate pages" is O.1).
    pub rotation: Rotation,
    /// The box shown.
    pub page_box: PageBox,
    /// Back and forward.
    pub history: History,
}

/// A change to the view, as the app's commands ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewCommand {
    /// Single page.
    Single,
    /// Facing pages.
    Facing,
    /// Book view.
    Book,
    /// The horizontal strip.
    Horizontal,
    /// The grid; its columns follow the window's width.
    Grid,
    /// Continuous or one row at a time.
    ToggleFlow,
    /// Left to right or right to left (manga).
    ToggleDirection,
    /// A quarter turn clockwise.
    RotateClockwise,
    /// A quarter turn anticlockwise.
    RotateAnticlockwise,
    /// Shows this box.
    PageBox(PageBox),
    /// Chooses the zoom this way from now on.
    Fit(ZoomMode),
}

impl ViewCommand {
    /// The command the app names `name` (the page canvas's protocol), if it is a view command.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "layoutSingle" => Self::Single,
            "layoutFacing" => Self::Facing,
            "layoutBook" => Self::Book,
            "layoutHorizontal" => Self::Horizontal,
            "layoutGrid" => Self::Grid,
            "toggleContinuous" => Self::ToggleFlow,
            "toggleDirection" => Self::ToggleDirection,
            "rotateClockwise" => Self::RotateClockwise,
            "rotateAnticlockwise" => Self::RotateAnticlockwise,
            "boxMedia" => Self::PageBox(PageBox::Media),
            "boxCrop" => Self::PageBox(PageBox::Crop),
            "boxBleed" => Self::PageBox(PageBox::Bleed),
            "boxTrim" => Self::PageBox(PageBox::Trim),
            "boxArt" => Self::PageBox(PageBox::Art),
            "fitWidth" => Self::Fit(ZoomMode::FitWidth),
            "fitPage" => Self::Fit(ZoomMode::FitPage),
            "fitHeight" => Self::Fit(ZoomMode::FitHeight),
            _ => return None,
        })
    }
}

/// What the canvas must do after a change to the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing changed.
    None,
    /// Lay the pages out again.
    Relayout,
    /// Pages turned: their tiles are wrong; lay out again.
    Rerender,
    /// Read every page's size again for the new box.
    Remeasure,
    /// Zoom by this mode.
    Fit(ZoomMode),
}

impl View {
    /// Applies `command`; `width` is the canvas's width in device pixels (for the grid's columns).
    pub fn apply(&mut self, command: ViewCommand, width: f32) -> Effect {
        let before = (self.spec, self.rotation, self.page_box);
        match command {
            ViewCommand::Single => self.spec.mode = Mode::Single,
            ViewCommand::Facing => self.spec.mode = Mode::Facing,
            ViewCommand::Book => self.spec.mode = Mode::Book,
            ViewCommand::Horizontal => self.spec.mode = Mode::Horizontal,
            ViewCommand::Grid => {
                self.spec.mode = Mode::Grid {
                    columns: grid_columns(width),
                };
            }
            ViewCommand::ToggleFlow => self.spec.flow = self.spec.flow.toggled(),
            ViewCommand::ToggleDirection => self.spec.direction = self.spec.direction.flipped(),
            ViewCommand::RotateClockwise => self.rotation = self.rotation.turned_clockwise(),
            ViewCommand::RotateAnticlockwise => {
                self.rotation = self.rotation.turned_anticlockwise();
            }
            ViewCommand::PageBox(page_box) => self.page_box = page_box,
            ViewCommand::Fit(mode) => return Effect::Fit(mode),
        }
        let (spec, rotation, page_box) = before;
        if self.rotation != rotation {
            Effect::Rerender
        } else if self.page_box != page_box {
            Effect::Remeasure
        } else if self.spec != spec {
            Effect::Relayout
        } else {
            Effect::None
        }
    }

    /// The window is now `width` device pixels wide: a grid takes as many columns as fit. Returns whether the
    /// layout changed.
    pub fn resized(&mut self, width: f32) -> bool {
        let Mode::Grid { columns } = self.spec.mode else {
            return false;
        };
        let fitting = grid_columns(width);
        self.spec.mode = Mode::Grid { columns: fitting };
        fitting != columns
    }
}

/// How many grid columns a canvas `width` device pixels wide holds.
fn grid_columns(width: f32) -> u8 {
    // The float-to-int `as` saturates, and the value is clamped to 1..=8 first anyway.
    (width / GRID_CELL).clamp(1.0, f32::from(GRID_MAX_COLUMNS)) as u8
}

#[cfg(test)]
mod tests {
    use excellent_canvas::layout::{Direction, Flow};

    use super::*;

    #[test]
    fn each_change_asks_for_what_it_needs() {
        let mut view = View::default();
        assert_eq!(view.apply(ViewCommand::Book, 1000.0), Effect::Relayout);
        assert_eq!(view.apply(ViewCommand::Book, 1000.0), Effect::None);
        assert_eq!(
            view.apply(ViewCommand::RotateClockwise, 1000.0),
            Effect::Rerender
        );
        assert_eq!(view.rotation, Rotation::Clockwise);
        assert_eq!(
            view.apply(ViewCommand::PageBox(PageBox::Media), 1000.0),
            Effect::Remeasure
        );
        assert_eq!(
            view.apply(ViewCommand::Fit(ZoomMode::FitPage), 1000.0),
            Effect::Fit(ZoomMode::FitPage)
        );
        view.apply(ViewCommand::ToggleFlow, 1000.0);
        view.apply(ViewCommand::ToggleDirection, 1000.0);
        assert_eq!(view.spec.flow, Flow::Paged);
        assert_eq!(view.spec.direction, Direction::RightToLeft);
    }

    #[test]
    fn the_grid_follows_the_width() {
        let mut view = View::default();
        assert!(!view.resized(2000.0), "not a grid");
        view.apply(ViewCommand::Grid, 1000.0);
        assert_eq!(view.spec.mode, Mode::Grid { columns: 4 });
        assert!(view.resized(500.0));
        assert_eq!(view.spec.mode, Mode::Grid { columns: 2 });
        assert!(!view.resized(510.0));
        view.resized(10.0);
        assert_eq!(view.spec.mode, Mode::Grid { columns: 1 });
        view.resized(1e6);
        assert_eq!(
            view.spec.mode,
            Mode::Grid {
                columns: GRID_MAX_COLUMNS
            }
        );
    }

    #[test]
    fn every_command_name_parses() {
        for name in [
            "layoutSingle",
            "layoutFacing",
            "layoutBook",
            "layoutHorizontal",
            "layoutGrid",
            "toggleContinuous",
            "toggleDirection",
            "rotateClockwise",
            "rotateAnticlockwise",
            "boxMedia",
            "boxCrop",
            "boxBleed",
            "boxTrim",
            "boxArt",
            "fitWidth",
            "fitPage",
            "fitHeight",
        ] {
            assert!(ViewCommand::parse(name).is_some(), "{name}");
        }
        assert_eq!(ViewCommand::parse("zoomIn"), None);
    }
}
