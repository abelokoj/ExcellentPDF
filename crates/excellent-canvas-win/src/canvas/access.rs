//! What the canvas tells screen readers (plan U.2): its pages, their links, notes and text, for the UI Automation
//! providers in `crate::uia`. Areas come back in client pixels.

use std::ops::Range;

use excellent_canvas::Rect;
use excellent_core::render::Note;
use excellent_mupdf::{Link, PageRect};
use excellent_text::{DocumentText, Granularity, Selection, TextPos};

use super::Canvas;
use crate::placement::Placement;

impl Canvas {
    /// The open file's name, once one is open.
    pub(crate) fn file_name(&self) -> Option<String> {
        self.document.as_ref()?;
        Some(
            self.path
                .as_ref()?
                .file_name()?
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// The number of pages shown; none before a document is open.
    pub(crate) fn page_count(&self) -> usize {
        if self.document.is_some() {
            self.layout.len()
        } else {
            0
        }
    }

    /// The document's text as far as it is read: the pages on and near the screen.
    pub(crate) fn document_text(&self) -> DocumentText<'_> {
        DocumentText::new(
            (0..self.page_count())
                .map(|page| self.texts.get(page))
                .collect(),
        )
    }

    /// The first and last pages on the screen.
    pub(crate) fn shown_pages(&self) -> Option<Range<usize>> {
        let visible = self.viewport.visible();
        let mut pages = self.layout.pages_in(&visible).map(|(index, _)| index);
        let first = pages.next()?;
        let (low, high) = pages.fold((first, first), |(low, high), page| {
            (low.min(page), high.max(page))
        });
        Some(low..high + 1)
    }

    /// Page `page`'s area in client pixels; off the window when it isn't on the screen.
    pub(crate) fn page_area(&self, page: usize) -> Option<Rect> {
        Some(self.viewport.to_screen(&self.layout.page(page)?))
    }

    /// Area `rect` of page `page` (page points), in client pixels.
    pub(crate) fn area_on_page(&self, page: usize, rect: PageRect) -> Option<Rect> {
        let placement = self.placement(page)?;
        let corners = [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x0, rect.y1),
            (rect.x1, rect.y1),
        ];
        Some(self.bounding(
            corners.map(|(x, y)| placement.to_layout(excellent_mupdf::PagePoint { x, y })),
        ))
    }

    /// The areas of characters `range` of page `page`, one per line, in client pixels.
    pub(crate) fn text_areas(&self, page: usize, range: Range<usize>) -> Vec<Rect> {
        let (Some(text), Some(placement)) = (self.texts.get(page), self.placement(page)) else {
            return Vec::new();
        };
        text.highlight(range)
            .into_iter()
            .map(|quad| {
                self.bounding([quad.ul, quad.ur, quad.lr, quad.ll].map(|p| placement.to_layout(p)))
            })
            .collect()
    }

    fn placement(&self, page: usize) -> Option<Placement> {
        Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)
    }

    /// The box around layout points `corners`, in client pixels.
    fn bounding(&self, corners: [(f32, f32); 4]) -> Rect {
        let screen = corners.map(|(x, y)| self.viewport.to_screen_point(x, y));
        let xs = screen.map(|(x, _)| x);
        let ys = screen.map(|(_, y)| y);
        Rect {
            x0: xs.into_iter().fold(f32::INFINITY, f32::min),
            y0: ys.into_iter().fold(f32::INFINITY, f32::min),
            x1: xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
            y1: ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
        }
    }

    /// Page `page`'s links, once read.
    pub(crate) fn links_on(&self, page: usize) -> &[Link] {
        self.links.get(page).map_or(&[], Vec::as_slice)
    }

    /// Page `page`'s notes, once read.
    pub(crate) fn notes_on(&self, page: usize) -> &[Note] {
        self.notes.get(page).map_or(&[], Vec::as_slice)
    }

    /// The characters of page `page` inside `rect` (a link's words), once its text is read.
    pub(crate) fn text_within(&self, page: usize, rect: PageRect) -> Option<Range<usize>> {
        self.texts.get(page)?.within(rect)
    }

    /// The caret nearest client pixel (`x`, `y`), on the page there.
    pub(crate) fn caret_at(&self, x: i32, y: i32) -> Option<TextPos> {
        self.text_pos(x, y, Granularity::Char)
    }

    /// The selected text's first and last carets, once the text of its first and last pages is read.
    pub(crate) fn selected(&self) -> Option<(TextPos, TextPos)> {
        let selection = self.selection?;
        let mut ranges = selection
            .pages()
            .filter_map(|page| Some((page, selection.range_on(page, self.texts.get(page)?)?)));
        let (first_page, first) = ranges.next()?;
        let (last_page, last) = ranges.next_back().unwrap_or((first_page, first.clone()));
        Some((
            TextPos {
                page: first_page,
                index: first.start,
            },
            TextPos {
                page: last_page,
                index: last.end,
            },
        ))
    }

    /// Selects `start..end` (a screen reader's selection); an empty one clears it.
    pub(crate) fn select(&mut self, start: TextPos, end: TextPos) {
        let selection = Selection {
            anchor: start,
            focus: end,
            granularity: Granularity::Char,
        };
        self.selection = (!selection.is_collapsed()).then_some(selection);
        self.invalidate();
    }

    /// Scrolls to `at`: its line at the top of the window, or its page's top while its text isn't read.
    pub(crate) fn show_text(&mut self, at: TextPos) {
        let point = self.texts.get(at.page).and_then(|text| {
            let index = at.index.min(text.len().checked_sub(1)?);
            Some(text.highlight(index..index + 1).first()?.ul)
        });
        match point {
            Some(point) => self.go_to_point(at.page, point, None),
            None => self.go_to_page(at.page),
        }
    }

    /// Follows `link`: a place in the document jumps there; any other goes to the app (`openLink`: `{uri}`), which
    /// decides whether to open it.
    pub(crate) fn follow(&mut self, link: &Link) {
        match link.target {
            Some(target) => self.jump(target),
            None => self.emit("openLink", serde_json::json!({ "uri": link.uri })),
        }
    }
}
