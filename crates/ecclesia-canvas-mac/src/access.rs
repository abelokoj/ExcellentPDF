//! The canvas's text for screen readers on macOS (plan U.3): what the view answers as an accessibility text area, so
//! VoiceOver reads the pages in order, by line, and follows the selection. The text is the shared canvas's flat text
//! ([`ecclesia_text::FlatText`]): the pages read so far, a line break after each line.
//!
// shortcut: offsets count characters, where AppKit's ranges count UTF-16 units; they differ only after a character
// outside the Basic Multilingual Plane (some emoji, rare CJK), which PDFs seldom hold. Map through UTF-16 if one does.

use std::ops::Range;

use ecclesia_canvas::Rect;

use crate::host::Canvas;

/// All of the text.
pub(crate) fn value(canvas: &Canvas) -> String {
    let flat = canvas.flat_text();
    flat.text(0..flat.len())
}

/// The number of characters.
pub(crate) fn count(canvas: &Canvas) -> usize {
    canvas.flat_text().len()
}

/// The characters of `range`.
pub(crate) fn string(canvas: &Canvas, range: Range<usize>) -> String {
    canvas.flat_text().text(range)
}

/// The selection, or the caret at the start.
pub(crate) fn selected(canvas: &Canvas) -> Range<usize> {
    let flat = canvas.flat_text();
    canvas.flat_selection(&flat).unwrap_or(0..0)
}

/// Selects `range` (VoiceOver's selection).
pub(crate) fn select(canvas: &mut Canvas, range: Range<usize>) {
    let flat = canvas.flat_text();
    canvas.flat_select(&flat, range);
}

/// The characters on the pages on the screen.
pub(crate) fn shown(canvas: &Canvas) -> Range<usize> {
    let flat = canvas.flat_text();
    canvas.flat_shown(&flat)
}

/// The line holding character `index`.
pub(crate) fn line_at(canvas: &Canvas, index: usize) -> usize {
    canvas.flat_text().line_at(index)
}

/// Line `line`'s characters.
pub(crate) fn line_range(canvas: &Canvas, line: usize) -> Range<usize> {
    canvas.flat_text().line_range(line)
}

/// The area `range` covers, in the view's device pixels; `None` when none of it is laid out on a page.
pub(crate) fn area(canvas: &Canvas, range: Range<usize>) -> Option<Rect> {
    let flat = canvas.flat_text();
    canvas
        .flat_areas(&flat, range)
        .into_iter()
        .reduce(|a, b| Rect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
}
