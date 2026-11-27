//! Margin notes (A.11): while the app shows its margin column (the `margin` prop), the canvas tells it where each
//! page it shows is (`margin`: `{pages: [{page, m: [a, b, c, d, e, f]}]}`, each map from the page's points to the
//! app's pixels from the canvas's top-left corner), whenever that moves, so the cards stay level with their notes
//! while the pages scroll and zoom. The model is `ecclesia-features`' `annotate::margin`.

use ecclesia_features::annotate::margin::ToScreen;
use ecclesia_mupdf::PagePoint;

use super::PageCanvas;
use crate::page::host::Host;
use crate::page::placement::Placement;

/// The margin column is shown: what it was last told, each shown page and its map (`None`: nothing yet).
#[derive(Debug, Default)]
pub(super) struct Margin {
    reported: Option<Vec<(usize, ToScreen)>>,
}

impl<H: Host> PageCanvas<H> {
    /// Whether the app shows its margin column (the `margin` prop). Turned on, the pages are reported at once.
    pub(crate) fn set_margin(&mut self, on: bool) {
        if on != self.margin.is_some() {
            self.margin = on.then(Margin::default);
            self.invalidate();
        }
    }

    /// Reports where the shown pages are when it changed, while the margin column is shown.
    pub(super) fn report_margin(&mut self) {
        if self.margin.is_none() {
            return;
        }
        let shown = self.shown_maps();
        if self
            .margin
            .as_ref()
            .is_some_and(|margin| margin.reported.as_ref() == Some(&shown))
        {
            return;
        }
        let pages: Vec<_> = shown
            .iter()
            .map(|(page, map)| serde_json::json!({ "page": page, "m": map.to_json() }))
            .collect();
        self.emit("margin", serde_json::json!({ "pages": pages }));
        self.margin = Some(Margin {
            reported: Some(shown),
        });
    }

    /// Each page on screen and how its points map to the app's pixels. A reflowed book (R.28) has no notes.
    fn shown_maps(&self) -> Vec<(usize, ToScreen)> {
        if self.reflow.is_some() {
            return Vec::new();
        }
        // Device pixels to the app's: the platform's pixels a point at 100 %, over the screen's.
        let to_app = self.host.ui_pixels_per_point() / self.viewport.pixels_per_point();
        let visible = self.viewport.visible();
        self.layout
            .pages_in(&visible)
            .filter_map(|(page, _)| {
                let placement =
                    Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
                let screen = |x: f32, y: f32| {
                    let (lx, ly) = placement.to_layout(PagePoint { x, y });
                    let (sx, sy) = self.viewport.to_screen_point(lx, ly);
                    (sx * to_app, sy * to_app)
                };
                Some((
                    page,
                    ToScreen::through(screen(0.0, 0.0), screen(1.0, 0.0), screen(0.0, 1.0)),
                ))
            })
            .collect()
    }
}
