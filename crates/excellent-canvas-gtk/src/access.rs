//! The canvas's text for screen readers on Linux (plan U.3): GTK's `GtkAccessibleText`, which GTK hands to AT-SPI, so
//! Orca reads the pages in order and follows the selection. The text is the shared canvas's flat text
//! ([`excellent_text::FlatText`]): the pages read so far, a line break after each line.
//!
//! Offsets count characters, as GTK's do.

use excellent_text::{FlatText, TextUnit};
use nsh_platform_linux::gtk4 as gtk;

use gtk::glib;

use crate::host::Canvas;

/// The characters of `start..end` as UTF-8.
pub(crate) fn contents(canvas: &Canvas, start: u32, end: u32) -> glib::Bytes {
    let flat = canvas.flat_text();
    // GTK asks for the rest of the text with an end of -1, which arrives as `u32::MAX`.
    bytes(&flat, start as usize..end as usize)
}

/// The `granularity` unit around `offset`: its offsets and its text.
pub(crate) fn contents_at(
    canvas: &Canvas,
    offset: u32,
    granularity: gtk::AccessibleTextGranularity,
) -> (u32, u32, glib::Bytes) {
    let flat = canvas.flat_text();
    let unit = match granularity {
        gtk::AccessibleTextGranularity::Character => TextUnit::Character,
        gtk::AccessibleTextGranularity::Word => TextUnit::Word,
        gtk::AccessibleTextGranularity::Line => TextUnit::Line,
        // shortcut: no sentence unit yet (K.13 step 3's word breaks bring one); a paragraph stands in.
        _ => TextUnit::Paragraph,
    };
    let range = canvas.flat_unit(&flat, offset as usize, unit);
    (
        offset_u32(range.start),
        offset_u32(range.end),
        bytes(&flat, range),
    )
}

/// The caret: the selection's start, or the text's start.
pub(crate) fn caret(canvas: &Canvas) -> u32 {
    let flat = canvas.flat_text();
    canvas
        .flat_selection(&flat)
        .map_or(0, |range| offset_u32(range.start))
}

/// The selection, if any.
pub(crate) fn selection(canvas: &Canvas) -> Vec<gtk::AccessibleTextRange> {
    let flat = canvas.flat_text();
    canvas
        .flat_selection(&flat)
        .filter(|range| !range.is_empty())
        .map(|range| gtk::AccessibleTextRange::new(range.start, range.len()))
        .into_iter()
        .collect()
}

/// The text's length, for change notices.
pub(crate) fn len(canvas: &Canvas) -> u32 {
    offset_u32(canvas.flat_text().len())
}

fn bytes(flat: &FlatText, range: std::ops::Range<usize>) -> glib::Bytes {
    glib::Bytes::from_owned(flat.text(range).into_bytes())
}

fn offset_u32(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}
