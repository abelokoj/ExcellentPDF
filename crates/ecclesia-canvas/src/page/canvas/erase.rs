//! Erasers (H.5): while an eraser is chosen (the `eraser` prop), or the stylus's eraser end touches down, the pen
//! erases instead of drawing. The canvas shows the eraser's path as a grey swath and, on pen-up, sends the app where
//! it went (`erase`: `{page, eraser, radius, points: [[x, y]]}`, in page points), which the app applies to the
//! page's ink as one step of undo. The model is `ecclesia-ink`'s `erase`.

use ecclesia_ink::erase::Eraser;
use ecclesia_mupdf::PagePoint;

use super::{InkPixel, PageCanvas, PenSample, ScreenQuad, segment_quads};
use crate::page::host::Host;
use crate::page::placement::Placement;

/// The eraser's radius on screen, in pixels: the same size under the pen at any zoom.
const RADIUS_PX: f32 = 10.0;

/// The swath the eraser leaves while it moves: a light grey, drawn darkest-wins like a highlight, so the page shows
/// through it.
const SWATH: u32 = 0xd0d0d0;

/// The erasers: the one chosen on the toolbar, the one the stylus's eraser end uses, and a pass under way.
#[derive(Debug, Default)]
pub(super) struct Erasers {
    /// Chosen on the toolbar: the pen erases with it. `None`: the pen draws.
    chosen: Option<Eraser>,
    /// What the stylus's eraser end erases with: the last eraser chosen, at first the stroke eraser.
    end: Eraser,
    /// A pass under way.
    pass: Option<Pass>,
}

/// An eraser moving over a page.
#[derive(Debug)]
struct Pass {
    /// The pointer moving it.
    id: u32,
    page: usize,
    eraser: Eraser,
    /// In page points, the same on screen at the zoom it started at.
    radius: f32,
    /// Where it went, in page points.
    points: Vec<PagePoint>,
}

impl<H: Host> PageCanvas<H> {
    /// The eraser the pen uses from now on (the `eraser` prop); `None` puts the pen back to drawing. A chosen eraser
    /// also becomes the stylus's eraser end's.
    pub(crate) fn set_eraser(&mut self, eraser: Option<Eraser>) {
        self.erasers.chosen = eraser;
        if let Some(eraser) = eraser {
            self.erasers.end = eraser;
        }
    }

    /// The pen touched down: an eraser pass starts when the eraser end touched or an eraser is chosen, over a page.
    /// Returns whether it did.
    pub(super) fn erase_down(&mut self, sample: &PenSample) -> bool {
        let eraser = match (sample.eraser, self.erasers.chosen) {
            (true, _) => self.erasers.end,
            (false, Some(chosen)) => chosen,
            (false, None) => return false,
        };
        let (x, y) = self.viewport.to_layout_point(sample.x, sample.y);
        let Some(page) = self.layout.page_at(x, y) else {
            return false;
        };
        self.erasers.pass = Some(Pass {
            id: sample.id,
            page,
            eraser,
            radius: RADIUS_PX / self.viewport.scale(),
            points: Vec::new(),
        });
        self.erase_moved(std::slice::from_ref(sample));
        true
    }

    /// The pen moved: returns whether an eraser pass took the samples.
    pub(super) fn erase_moved(&mut self, samples: &[PenSample]) -> bool {
        let Some(page) = self.erasers.pass.as_ref().map(|pass| pass.page) else {
            return false;
        };
        let Some(placement) =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
        else {
            return true;
        };
        let Some(pass) = &mut self.erasers.pass else {
            return true;
        };
        for sample in samples.iter().filter(|s| s.id == pass.id && s.contact) {
            let (x, y) = self.viewport.to_layout_point(sample.x, sample.y);
            pass.points.push(placement.to_page(x, y));
        }
        self.invalidate();
        true
    }

    /// The pen lifted: a pass under way goes to the app (`erase`). Returns whether there was one.
    pub(super) fn erase_up(&mut self) -> bool {
        let Some(pass) = self.erasers.pass.take() else {
            return false;
        };
        if !pass.points.is_empty() {
            let points: Vec<[f32; 2]> = pass.points.iter().map(|p| [p.x, p.y]).collect();
            self.emit(
                "erase",
                serde_json::json!({
                    "page": pass.page,
                    "eraser": pass.eraser.name(),
                    "radius": pass.radius,
                    "points": points,
                }),
            );
        }
        self.invalidate();
        true
    }

    /// Whether pointer `id` is erasing.
    pub(super) fn is_erasing(&self, id: u32) -> bool {
        self.erasers.pass.as_ref().is_some_and(|pass| pass.id == id)
    }

    /// Whether an eraser is chosen: the pen erases.
    pub(super) fn erasing_chosen(&self) -> bool {
        self.erasers.chosen.is_some()
    }

    /// The swath of the pass under way, as screen quads and their colour, to draw darkest-wins; `None` without one.
    pub(super) fn eraser_swath(&self) -> Option<(Vec<ScreenQuad>, u32)> {
        let pass = self.erasers.pass.as_ref()?;
        let placement =
            Placement::of_page(&self.layout, &self.bounds, self.view.rotation, pass.page)?;
        let width = pass.radius * 2.0 * self.viewport.scale();
        let points: Vec<InkPixel> = pass
            .points
            .iter()
            .map(|&p| {
                let (x, y) = placement.to_layout(p);
                let (x, y) = self.viewport.to_screen_point(x, y);
                (x, y, width)
            })
            .collect();
        Some((segment_quads(&points), SWATH))
    }
}
