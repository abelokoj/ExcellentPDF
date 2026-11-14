//! The text layer (plan K.13, Appendix D.1.3): what search, selection, read aloud, reflow and accessibility read a
//! page's text through. A page's text is copied out of MuPDF once, on a worker ([`PageText`]), so finding the
//! character under the pointer, snapping to words and lines, drawing a selection and copying it are plain Rust on the
//! UI thread, quick enough to follow a drag.
//!
//! shortcut: reading order is MuPDF's (with the PDF's structure when it has some); column detection, Unicode
//! normalization and ICU4X word breaks (languages written without spaces, bidirectional text) come next (K.13 steps 2
//! and 3). Until then a Chinese or Japanese character is a word of its own and a Thai run is one word.

mod page;
mod selection;
mod words;

pub use excellent_mupdf::TextLayout;
pub use page::PageText;
pub use selection::{Granularity, Selection, TextPos};
