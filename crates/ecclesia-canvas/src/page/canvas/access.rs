//! What the canvas tells screen readers (plan U.2): its pages, their links, notes and text, for the UI Automation
//! providers on Windows, and as one flat string ([`FlatText`]) for GTK's and AppKit's text interfaces (U.3). Areas come
//! back in client pixels.

use std::ops::Range;

use crate::Rect;
use ecclesia_core::render::Note;
use ecclesia_mupdf::{Link, PageRect};
use ecclesia_text::{DocumentText, FlatText, Granularity, Selection, TextPos, TextUnit};

use super::PageCanvas;
use crate::page::host::Host;
use crate::page::placement::Placement;

impl<H: Host> PageCanvas<H> {
    /// The open file's name, once one is open.
    pub fn file_name(&self) -> Option<String> {
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
    pub fn page_count(&self) -> usize {
        if self.document.is_some() {
            self.layout.len()
        } else {
            0
        }
    }

    /// The document's text as far as it is read: the pages on and near the screen.
    pub fn document_text(&self) -> DocumentText<'_> {
        DocumentText::new(
            (0..self.page_count())
                .map(|page| self.texts.get(page))
                .collect(),
        )
    }

    /// The first and last pages on the screen.
    pub fn shown_pages(&self) -> Option<Range<usize>> {
        let visible = self.viewport.visible();
        let mut pages = self.layout.pages_in(&visible).map(|(index, _)| index);
        let first = pages.next()?;
        let (low, high) = pages.fold((first, first), |(low, high), page| {
            (low.min(page), high.max(page))
        });
        Some(low..high + 1)
    }

    /// Page `page`'s area in client pixels; off the window when it isn't on the screen.
    pub fn page_area(&self, page: usize) -> Option<Rect> {
        Some(self.viewport.to_screen(&self.layout.page(page)?))
    }

    /// Area `rect` of page `page` (page points), in client pixels.
    pub fn area_on_page(&self, page: usize, rect: PageRect) -> Option<Rect> {
        let placement = self.placement(page)?;
        let corners = [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x0, rect.y1),
            (rect.x1, rect.y1),
        ];
        Some(self.bounding(
            corners.map(|(x, y)| placement.to_layout(ecclesia_mupdf::PagePoint { x, y })),
        ))
    }

    /// The areas of characters `range` of page `page`, one per line, in client pixels.
    pub fn text_areas(&self, page: usize, range: Range<usize>) -> Vec<Rect> {
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
    pub fn links_on(&self, page: usize) -> &[Link] {
        self.links.get(page).map_or(&[], Vec::as_slice)
    }

    /// Page `page`'s notes, once read.
    pub fn notes_on(&self, page: usize) -> &[Note] {
        self.notes.get(page).map_or(&[], Vec::as_slice)
    }

    /// The characters of page `page` inside `rect` (a link's words), once its text is read.
    pub fn text_within(&self, page: usize, rect: PageRect) -> Option<Range<usize>> {
        self.texts.get(page)?.within(rect)
    }

    /// The caret nearest client pixel (`x`, `y`), on the page there.
    pub fn caret_at(&self, x: i32, y: i32) -> Option<TextPos> {
        self.text_pos(x, y, Granularity::Char)
    }

    /// The selected text's first and last carets, once the text of its first and last pages is read.
    pub fn selected(&self) -> Option<(TextPos, TextPos)> {
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
    pub fn select(&mut self, start: TextPos, end: TextPos) {
        let selection = Selection {
            anchor: start,
            focus: end,
            granularity: Granularity::Char,
        };
        self.selection = (!selection.is_collapsed()).then_some(selection);
        self.invalidate();
    }

    /// Scrolls to `at`: its line at the top of the window, or its page's top while its text isn't read.
    pub fn show_text(&mut self, at: TextPos) {
        let point = self.texts.get(at.page).and_then(|text| {
            let index = at.index.min(text.len().checked_sub(1)?);
            Some(text.highlight(index..index + 1).first()?.ul)
        });
        match point {
            Some(point) => self.go_to_point(at.page, point, None),
            None => self.go_to_page(at.page),
        }
    }

    /// The text as far as it is read, as one string (U.3).
    // shortcut: made again on every call; keep it until the text changes if screen readers make scrolling stall.
    pub fn flat_text(&self) -> FlatText {
        FlatText::new(&self.document_text())
    }

    /// The `unit` around offset `offset` of `flat`, as its offsets.
    pub fn flat_unit(&self, flat: &FlatText, offset: usize, unit: TextUnit) -> Range<usize> {
        let (start, end) = self.document_text().expand(flat.pos(offset), unit);
        flat.offset(start)..flat.offset(end)
    }

    /// The selection as offsets of `flat`.
    pub fn flat_selection(&self, flat: &FlatText) -> Option<Range<usize>> {
        let (start, end) = self.selected()?;
        Some(flat.offset(start)..flat.offset(end))
    }

    /// Selects offsets `range` of `flat`.
    pub fn flat_select(&mut self, flat: &FlatText, range: Range<usize>) {
        self.select(flat.pos(range.start), flat.pos(range.end));
    }

    /// The offsets of `flat` on the pages on the screen.
    pub fn flat_shown(&self, flat: &FlatText) -> Range<usize> {
        let Some(pages) = self.shown_pages() else {
            return 0..0;
        };
        let start = flat.offset(TextPos {
            page: pages.start,
            index: 0,
        });
        // Past the last page, `offset` gives the text's end.
        let end = flat.offset(TextPos {
            page: pages.end,
            index: 0,
        });
        start..end.max(start)
    }

    /// The client-pixel areas of offsets `range` of `flat`.
    pub fn flat_areas(&self, flat: &FlatText, range: Range<usize>) -> Vec<Rect> {
        self.document_text()
            .spans(flat.pos(range.start), flat.pos(range.end))
            .into_iter()
            .flat_map(|(page, chars)| self.text_areas(page, chars))
            .collect()
    }

    /// Follows `link`: a place in the document jumps there; any other goes to the app (`openLink`: `{uri}`), which
    /// decides whether to open it.
    pub fn follow(&mut self, link: &Link) {
        match link.target {
            Some(target) => self.jump(target),
            None => self.emit("openLink", serde_json::json!({ "uri": link.uri })),
        }
    }
}
