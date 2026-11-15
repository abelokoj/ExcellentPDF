//! A view as the app saves it with a tab (R.1, R.2, R.9): the page and the place on it, the zoom, the layout and the
//! view rotation, as the JSON of the canvas's `view` event and `setView` command. The page box isn't kept (each
//! document opens on its crop box), and a grid's columns follow the window, so only "grid" is kept.
//!
//! ```json
//! {"page": 3, "across": 0.0, "down": 0.25, "fit": "width", "zoom": 1.5,
//!  "layout": "facing", "paged": false, "rtl": false, "rotation": 90}
//! ```

use excellent_canvas::layout::{Direction, Flow, Mode, Spec};
use excellent_canvas::viewport::{Anchor, ZoomMode};
use excellent_core::transform::Rotation;
use excellent_core::view::history::Place;
use nsh_core::JsonValue;

/// Where the reader is, and how the pages are shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SavedView {
    pub(crate) place: Place,
    pub(crate) spec: Spec,
    pub(crate) rotation: Rotation,
}

impl SavedView {
    /// The view as JSON.
    pub(crate) fn to_json(self) -> JsonValue {
        let Anchor { page, across, down } = self.place.anchor;
        let fit = match self.place.mode {
            ZoomMode::Custom => "custom",
            ZoomMode::FitWidth => "width",
            ZoomMode::FitPage => "page",
            ZoomMode::FitHeight => "height",
        };
        let layout = match self.spec.mode {
            Mode::Single => "single",
            Mode::Facing => "facing",
            Mode::Book => "book",
            Mode::Horizontal => "horizontal",
            Mode::Grid { .. } => "grid",
        };
        let rotation = match self.rotation {
            Rotation::Upright => 0,
            Rotation::Clockwise => 90,
            Rotation::UpsideDown => 180,
            Rotation::Anticlockwise => 270,
        };
        serde_json::json!({
            "page": page,
            "across": across,
            "down": down,
            "fit": fit,
            "zoom": self.place.zoom,
            "layout": layout,
            "paged": self.spec.flow == Flow::Paged,
            "rtl": self.spec.direction == Direction::RightToLeft,
            "rotation": rotation,
        })
    }

    /// The view `value` holds; `None` when a part is missing or unknown (an older or broken state).
    pub(crate) fn from_json(value: &JsonValue) -> Option<Self> {
        let number = |key: &str| value.get(key)?.as_f64().filter(|n| n.is_finite());
        let flag = |key: &str| value.get(key)?.as_bool();
        let text = |key: &str| value.get(key)?.as_str();
        let page = usize::try_from(value.get("page")?.as_u64()?).ok()?;
        let mode = match text("fit")? {
            "custom" => ZoomMode::Custom,
            "width" => ZoomMode::FitWidth,
            "page" => ZoomMode::FitPage,
            "height" => ZoomMode::FitHeight,
            _ => return None,
        };
        let layout = match text("layout")? {
            "single" => Mode::Single,
            "facing" => Mode::Facing,
            "book" => Mode::Book,
            "horizontal" => Mode::Horizontal,
            // The canvas fits the columns to its width.
            "grid" => Mode::Grid { columns: 1 },
            _ => return None,
        };
        let rotation = match value.get("rotation")?.as_u64()? {
            0 => Rotation::Upright,
            90 => Rotation::Clockwise,
            180 => Rotation::UpsideDown,
            270 => Rotation::Anticlockwise,
            _ => return None,
        };
        let zoom = number("zoom").filter(|zoom| *zoom > 0.0)? as f32;
        Some(Self {
            place: Place {
                anchor: Anchor {
                    page,
                    across: number("across")? as f32,
                    down: number("down")? as f32,
                },
                mode,
                zoom,
            },
            spec: Spec {
                mode: layout,
                flow: if flag("paged")? {
                    Flow::Paged
                } else {
                    Flow::Continuous
                },
                direction: if flag("rtl")? {
                    Direction::RightToLeft
                } else {
                    Direction::LeftToRight
                },
            },
            rotation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SavedView {
        SavedView {
            place: Place {
                anchor: Anchor {
                    page: 41,
                    across: 0.125,
                    down: 0.5,
                },
                mode: ZoomMode::Custom,
                zoom: 1.75,
            },
            spec: Spec {
                mode: Mode::Book,
                flow: Flow::Paged,
                direction: Direction::RightToLeft,
            },
            rotation: Rotation::Anticlockwise,
        }
    }

    #[test]
    fn a_saved_view_comes_back_as_it_was() {
        let saved = view();
        assert_eq!(SavedView::from_json(&saved.to_json()), Some(saved));
        let grid = SavedView {
            spec: Spec {
                mode: Mode::Grid { columns: 5 },
                ..Spec::default()
            },
            ..saved
        };
        // The columns follow the window, not the save.
        let back = SavedView::from_json(&grid.to_json()).expect("a view");
        assert_eq!(back.spec.mode, Mode::Grid { columns: 1 });
    }

    #[test]
    fn a_page_alone_or_a_broken_view_is_not_a_view() {
        assert_eq!(
            SavedView::from_json(&serde_json::json!({ "page": 3 })),
            None
        );
        let mut broken = view().to_json();
        broken["rotation"] = 45.into();
        assert_eq!(SavedView::from_json(&broken), None);
        let mut broken = view().to_json();
        broken["zoom"] = 0.into();
        assert_eq!(SavedView::from_json(&broken), None);
        assert_eq!(SavedView::from_json(&JsonValue::Null), None);
    }
}
