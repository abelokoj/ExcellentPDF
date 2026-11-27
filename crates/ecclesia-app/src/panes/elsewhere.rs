//! Opening a link in the other pane (V.9): Ctrl+click (Command on a Mac) or middle-click on a link in the pages, an
//! outline entry or a search result shows where it goes in another pane, and the pane it was clicked in stays where
//! it is. With one pane, the window splits.

use ecclesia_features::outline::Spot;
use nsh_core::JsonValue;

use super::{Panes, Side};
use crate::tabs::{Step, Then};

/// Where a link goes: a page, and the spot on it when the link names one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Place {
    pub(crate) page: usize,
    pub(crate) spot: Option<Spot>,
}

impl Place {
    /// The canvas's `openElsewhere` event, `{page, x, y}` (the point only when the link names one).
    pub(crate) fn from_json(data: &JsonValue) -> Option<Self> {
        let page = usize::try_from(data.get("page")?.as_u64()?).ok()?;
        let number = |key: &str| data.get(key)?.as_f64().filter(|n| n.is_finite());
        let spot = number("x").zip(number("y")).map(|(x, y)| Spot {
            x: x as f32,
            y: y as f32,
            zoom: None,
        });
        Some(Self { page, spot })
    }

    /// The canvas command that goes there, remembering where the pane was (R.8).
    pub(crate) fn command(self) -> Then {
        match self.spot {
            Some(Spot { x, y, zoom }) => {
                let mut args = serde_json::json!({ "page": self.page, "x": x, "y": y });
                if let Some(zoom) = zoom {
                    args["zoom"] = zoom.into();
                }
                ("goToPoint", args)
            }
            None => ("goToPage", serde_json::json!({ "index": self.page })),
        }
    }
}

impl Panes {
    /// Shows `place` of the document the pane in `cell` shows in the other pane: the active one when `cell` isn't
    /// active, else the next in reading order; with one pane, in a new one on the right. The document's tab there is
    /// shown, or a copy of this pane's tab added; the pane in `cell` doesn't move.
    pub(crate) fn open_elsewhere(self, cell: usize, place: Place) {
        let Some(copy) = self.slots[cell].tabs.shown_copy() else {
            return;
        };
        let then = place.command();
        let other = self.layout.with_untracked(|layout| {
            layout.is_split().then(|| match layout.active() {
                active if active != cell => active,
                _ => layout.step(Step::Next),
            })
        });
        match other {
            Some(to) => {
                let pane = self.slots[to];
                pane.tabs.show_then(copy, then, pane.canvas);
            }
            None => {
                self.split(Side::Right, move |pane| {
                    pane.tabs.show_then(copy, then, pane.canvas);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_place_reads_its_point_only_when_it_has_one() {
        let at = Place::from_json(&serde_json::json!({ "page": 4, "x": 72.0, "y": 300.5 }));
        assert_eq!(
            at,
            Some(Place {
                page: 4,
                spot: Some(Spot {
                    x: 72.0,
                    y: 300.5,
                    zoom: None
                })
            })
        );
        let (name, args) = at.expect("a place").command();
        assert_eq!(name, "goToPoint");
        assert_eq!(args["page"], 4);
        let page = Place::from_json(&serde_json::json!({ "page": 2 })).expect("a page");
        assert_eq!(
            page.command(),
            ("goToPage", serde_json::json!({ "index": 2 }))
        );
        assert_eq!(Place::from_json(&serde_json::json!({ "x": 1.0 })), None);
    }
}
